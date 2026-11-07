//! Administrative control plane gRPC service implementation.
//!
//! Provides cluster operators and automation pipelines with task ingestion,
//! node observability, and cancellation capabilities.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-004], [INV-002], [NFR-SEC-001]

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::admin_service_server::AdminService;
use crate::scheduler::ClusterScheduler;
use crate::{
    CancelTaskRequest, CancelTaskResponse, GetTaskStatusRequest, GetTaskStatusResponse,
    ListWorkerNodesRequest, ListWorkerNodesResponse, SubmitTaskRequest, SubmitTaskResponse,
};

/// Production implementation of AdminService backed by atomic ClusterScheduler.
#[derive(Debug, Clone)]
pub struct AdminServiceImpl {
    scheduler: Arc<ClusterScheduler>,
}

impl AdminServiceImpl {
    /// Constructs a new AdminServiceImpl with the shared cluster scheduler.
    pub fn new(scheduler: Arc<ClusterScheduler>) -> Self {
        Self { scheduler }
    }
}

#[tonic::async_trait]
impl AdminService for AdminServiceImpl {
    /// Enqueues a compute task into the cluster scheduling pool.
    ///
    /// Traces to: [REQ-OP-001], [REQ-OP-004]
    async fn submit_task(
        &self,
        request: Request<SubmitTaskRequest>,
    ) -> Result<Response<SubmitTaskResponse>, Status> {
        let req = request.into_inner();
        let task = req.task.ok_or_else(|| {
            Status::invalid_argument("task field is required in SubmitTaskRequest")
        })?;

        let task_id = task.task_id.clone();
        if task_id.is_empty() {
            return Err(Status::invalid_argument(
                "task_id must not be empty in TaskSpec",
            ));
        }

        match self.scheduler.submit_task(task) {
            Ok(()) => Ok(Response::new(SubmitTaskResponse {
                accepted: true,
                task_id,
                message: "Task accepted and enqueued".to_string(),
            })),
            Err(e) => Err(Status::invalid_argument(format!(
                "Failed to enqueue task: {e}"
            ))),
        }
    }

    /// Retrieves the current execution status and outputs of a task.
    ///
    /// Traces to: [REQ-OP-004]
    async fn get_task_status(
        &self,
        request: Request<GetTaskStatusRequest>,
    ) -> Result<Response<GetTaskStatusResponse>, Status> {
        let req = request.into_inner();
        let task_id = req.task_id;
        if task_id.is_empty() {
            return Err(Status::invalid_argument(
                "task_id must not be empty in GetTaskStatusRequest",
            ));
        }

        let (state, attempt, result_opt) = self
            .scheduler
            .get_task_details(&task_id)
            .ok_or_else(|| Status::not_found(format!("Task '{task_id}' not found")))?;

        let mut response = GetTaskStatusResponse {
            task_id,
            state: state as i32,
            active_attempt: attempt,
            exit_code: 0,
            stdout_bytes: Vec::new(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 0,
        };

        if let Some(res) = result_opt {
            response.exit_code = res.exit_code;
            response.stdout_bytes = res.stdout_bytes;
            response.stderr_bytes = res.stderr_bytes;
            response.error_code = res.error_code;
            response.error_message = res.error_message;
            response.execution_duration_ms = res.execution_duration_ms;
        }

        Ok(Response::new(response))
    }

    /// Lists all registered worker nodes, their capabilities, and connection states.
    ///
    /// Traces to: [REQ-OP-001]
    async fn list_worker_nodes(
        &self,
        _request: Request<ListWorkerNodesRequest>,
    ) -> Result<Response<ListWorkerNodesResponse>, Status> {
        let nodes = self.scheduler.list_worker_nodes();
        Ok(Response::new(ListWorkerNodesResponse { nodes }))
    }

    /// Cancels a scheduled or running task.
    ///
    /// Traces to: [REQ-OP-004], Section 6.2 State Machine
    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<CancelTaskResponse>, Status> {
        let req = request.into_inner();
        let task_id = req.task_id;
        if task_id.is_empty() {
            return Err(Status::invalid_argument(
                "task_id must not be empty in CancelTaskRequest",
            ));
        }

        match self.scheduler.cancel_task(&task_id) {
            Ok(()) => Ok(Response::new(CancelTaskResponse {
                cancelled: true,
                message: format!("Task '{task_id}' cancelled successfully"),
            })),
            Err(crate::SchedulingError::TaskNotFound(id)) => {
                Err(Status::not_found(format!("Task '{id}' not found")))
            }
            Err(crate::SchedulingError::AlreadyFinalized(id)) => Err(Status::failed_precondition(
                format!("Task '{id}' is already finalized in a terminal state"),
            )),
            Err(e) => Err(Status::internal(format!("Failed to cancel task: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionEngine, NetworkTransportType, NodeCapabilities, TaskSpec, TaskState};

    fn make_test_task(task_id: &str) -> TaskSpec {
        TaskSpec {
            task_id: task_id.to_string(),
            run_attempt: 1,
            engine: ExecutionEngine::WasmWasi as i32,
            timeout_seconds: 30,
            entrypoint: String::new(),
            env_vars: std::collections::HashMap::new(),
            wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d],
            shell_command: String::new(),
            artifact_sha256: "test_sha256".to_string(),
            requires_unmetered: false,
            artifact_url: String::new(),
        }
    }

    #[tokio::test]
    async fn test_submit_task_and_get_status() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let service = AdminServiceImpl::new(Arc::clone(&scheduler));

        let task = make_test_task("task_admin_01");
        let submit_res = service
            .submit_task(Request::new(SubmitTaskRequest { task: Some(task) }))
            .await
            .expect("Submit must succeed");

        assert!(submit_res.get_ref().accepted);
        assert_eq!(submit_res.get_ref().task_id, "task_admin_01");

        let status_res = service
            .get_task_status(Request::new(GetTaskStatusRequest {
                task_id: "task_admin_01".to_string(),
            }))
            .await
            .expect("Status must succeed");

        assert_eq!(status_res.get_ref().state, TaskState::Pending as i32);
        assert_eq!(status_res.get_ref().active_attempt, 0);
    }

    #[tokio::test]
    async fn test_cancel_pending_task() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let service = AdminServiceImpl::new(Arc::clone(&scheduler));

        let task = make_test_task("task_cancel_01");
        service
            .submit_task(Request::new(SubmitTaskRequest { task: Some(task) }))
            .await
            .unwrap();

        let cancel_res = service
            .cancel_task(Request::new(CancelTaskRequest {
                task_id: "task_cancel_01".to_string(),
            }))
            .await
            .expect("Cancel must succeed");

        assert!(cancel_res.get_ref().cancelled);

        let status_res = service
            .get_task_status(Request::new(GetTaskStatusRequest {
                task_id: "task_cancel_01".to_string(),
            }))
            .await
            .unwrap();

        assert_eq!(status_res.get_ref().state, TaskState::Failed as i32);
        assert_eq!(status_res.get_ref().error_code, "TASK_CANCELLED");
    }

    #[tokio::test]
    async fn test_list_worker_nodes() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let service = AdminServiceImpl::new(Arc::clone(&scheduler));

        let caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
            battery_temperature_celsius: 32,
            thermal_status: 1,
        };

        scheduler
            .register_node(
                "worker_alpha".to_string(),
                caps,
                "0.1.0".to_string(),
                1700000000,
            )
            .unwrap();

        let nodes_res = service
            .list_worker_nodes(Request::new(ListWorkerNodesRequest {}))
            .await
            .expect("List nodes must succeed");

        let nodes = &nodes_res.get_ref().nodes;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, "worker_alpha");
        assert_eq!(nodes[0].client_version, "0.1.0");
        assert_eq!(nodes[0].capabilities.as_ref().unwrap().cpu_cores, 8);
    }
}
