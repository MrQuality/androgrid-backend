//! Administrative control plane gRPC service implementation.
//!
//! Provides cluster operators and automation pipelines with task ingestion,
//! node observability, and cancellation capabilities.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-004], [INV-002], [NFR-SEC-001]

use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::admin_service_server::AdminService;
use crate::artifact::ArtifactStore;
use crate::pairing::{PairingPayload, PairingTokenStore};
use crate::scheduler::ClusterScheduler;
use crate::transport::tls::AuthenticatedPeer;
use crate::{
    AssignNodesToGroupRequest, AssignNodesToGroupResponse, CancelTaskGroupRequest,
    CancelTaskGroupResponse, CancelTaskRequest, CancelTaskResponse, CreateNodeGroupRequest,
    CreateNodeGroupResponse, DrainNodesRequest, DrainNodesResponse, GeneratePairingTokenRequest,
    GeneratePairingTokenResponse, GetTaskGroupStatusRequest, GetTaskGroupStatusResponse,
    GetTaskStatusRequest, GetTaskStatusResponse, ListNodeGroupsRequest, ListNodeGroupsResponse,
    ListTaskGroupsRequest, ListTaskGroupsResponse, ListTasksRequest, ListTasksResponse,
    ListWorkerNodesRequest, ListWorkerNodesResponse, ReplaceTaskGroupRequest,
    ReplaceTaskGroupResponse, SubmitTaskGroupRequest, SubmitTaskGroupResponse, SubmitTaskRequest,
    SubmitTaskResponse, TaskGroupState, TaskState, UpdateNodeGroupRequest, UpdateNodeGroupResponse,
    UpgradeNodesRequest, UpgradeNodesResponse, UploadArtifactRequest, UploadArtifactResponse,
};

/// Production implementation of AdminService backed by atomic ClusterScheduler.
#[derive(Clone)]
pub struct AdminServiceImpl {
    scheduler: Arc<ClusterScheduler>,
    artifact_store: Arc<ArtifactStore>,
    artifact_base_url: String,
    token_store: Arc<PairingTokenStore>,
    ca_fingerprint: String,
    cluster_endpoint: String,
}

impl AdminServiceImpl {
    /// Constructs a new AdminServiceImpl with the shared cluster scheduler.
    pub fn new(scheduler: Arc<ClusterScheduler>) -> Self {
        Self {
            scheduler,
            artifact_store: Arc::new(ArtifactStore::new()),
            artifact_base_url: "http://127.0.0.1:50052".to_string(),
            token_store: Arc::new(PairingTokenStore::default()),
            ca_fingerprint: String::new(),
            cluster_endpoint: "https://127.0.0.1:50051".to_string(),
        }
    }

    /// Constructs an AdminServiceImpl with explicit artifact store and base URL injection.
    ///
    /// Traces to: [NFR-PERF-001], [REQ-OP-003]
    pub fn with_artifact_store(
        scheduler: Arc<ClusterScheduler>,
        artifact_store: Arc<ArtifactStore>,
        artifact_base_url: String,
    ) -> Self {
        Self {
            scheduler,
            artifact_store,
            artifact_base_url,
            token_store: Arc::new(PairingTokenStore::default()),
            ca_fingerprint: String::new(),
            cluster_endpoint: "https://127.0.0.1:50051".to_string(),
        }
    }

    /// Constructs an AdminServiceImpl with full artifact and pairing subsystem injection.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    pub fn with_services(
        scheduler: Arc<ClusterScheduler>,
        artifact_store: Arc<ArtifactStore>,
        artifact_base_url: String,
        token_store: Arc<PairingTokenStore>,
        ca_fingerprint: String,
        cluster_endpoint: String,
    ) -> Self {
        Self {
            scheduler,
            artifact_store,
            artifact_base_url,
            token_store,
            ca_fingerprint,
            cluster_endpoint,
        }
    }

    /// Enforces a positive allowlist requiring operator certificates
    /// (`OU = Control Plane`) when mTLS identity metadata is attached.
    fn authorize_admin_peer<T>(request: &Request<T>) -> Result<(), Status> {
        if let Some(peer) = request.extensions().get::<AuthenticatedPeer>() {
            if peer.organizational_unit.as_deref() != Some("Control Plane") {
                return Err(Status::permission_denied(
                    "AdminService requires an operator certificate with OU=Control Plane",
                ));
            }
        }
        Ok(())
    }

    /// Validates WASM artifact references against ArtifactStore when inline bytecode is empty.
    fn validate_wasm_artifact(
        &self,
        engine: i32,
        wasm_bytecode: &[u8],
        artifact_sha256: &str,
    ) -> Result<(), Status> {
        if engine == crate::ExecutionEngine::WasmWasi as i32 && wasm_bytecode.is_empty() {
            let clean_sha = artifact_sha256.trim().to_ascii_lowercase();
            if clean_sha.len() != 64 || !clean_sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Status::invalid_argument(
                    "WASM task with empty wasm_bytecode requires a valid 64-char artifact_sha256",
                ));
            }
            match self.artifact_store.get_blob_kind(&clean_sha) {
                None => {
                    return Err(Status::failed_precondition(format!(
                        "Artifact '{clean_sha}' is not present in orchestrator ArtifactStore; upload it via UploadArtifact first"
                    )));
                }
                Some(kind) if kind != crate::artifact::store::BlobKind::Wasm => {
                    return Err(Status::invalid_argument(format!(
                        "Artifact '{clean_sha}' has kind '{}' and cannot be scheduled as a WASM task",
                        kind.as_str()
                    )));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    fn current_epoch_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
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
        Self::authorize_admin_peer(&request)?;
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
        if !task.group_id.is_empty() || task.group_epoch != 0 {
            return Err(Status::invalid_argument(
                "Standalone SubmitTask cannot specify group_id or group_epoch; use SubmitTaskGroup",
            ));
        }

        self.validate_wasm_artifact(task.engine, &task.wasm_bytecode, &task.artifact_sha256)?;

        match self.scheduler.submit_task(task) {
            Ok(()) => Ok(Response::new(SubmitTaskResponse {
                accepted: true,
                task_id,
                message: "Task accepted and enqueued".to_string(),
            })),
            Err(e @ crate::SchedulingError::NodeGroupNotFound(_)) => Err(Status::from(e)),
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
        Self::authorize_admin_peer(&request)?;
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
        request: Request<ListWorkerNodesRequest>,
    ) -> Result<Response<ListWorkerNodesResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let nodes = self.scheduler.list_worker_nodes();
        Ok(Response::new(ListWorkerNodesResponse { nodes }))
    }

    /// Lists all known compute tasks and their lifecycle execution states.
    ///
    /// Traces to: [REQ-OP-004], [INV-004], [INV-005]
    async fn list_tasks(
        &self,
        request: Request<ListTasksRequest>,
    ) -> Result<Response<ListTasksResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let state_filter = if req.state_filter == 0 {
            None
        } else {
            TaskState::try_from(req.state_filter).ok()
        };
        let tasks = self.scheduler.list_tasks(state_filter);
        Ok(Response::new(ListTasksResponse { tasks }))
    }

    /// Cancels a scheduled or running task.
    ///
    /// Traces to: [REQ-OP-004], Section 6.2 State Machine
    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<CancelTaskResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
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

    /// Uploads or registers a WebAssembly bytecode artifact for content-addressed distribution.
    ///
    /// Traces to: [NFR-PERF-001], [NFR-POW-001], [REQ-OP-003]
    async fn upload_artifact(
        &self,
        request: Request<UploadArtifactRequest>,
    ) -> Result<Response<UploadArtifactResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let bytecode = req.bytecode;
        if bytecode.is_empty() {
            return Err(Status::invalid_argument(
                "bytecode must not be empty in UploadArtifactRequest",
            ));
        }

        // Reserve protobuf framing overhead for FetchArtifactResponse (tag + varint + 64-char sha256)
        // so any accepted artifact is guaranteed fetchable within MAX_FRAME_SIZE_BYTES [NFR-PERF-001].
        let fetch_envelope_overhead = prost::Message::encoded_len(&crate::FetchArtifactResponse {
            wasm_bytecode: Vec::new(),
            artifact_sha256: "0".repeat(64),
        }) + 8;
        let max_artifact_bytes =
            crate::MAX_FRAME_SIZE_BYTES.saturating_sub(fetch_envelope_overhead);
        if bytecode.len() > max_artifact_bytes {
            return Err(Status::resource_exhausted(format!(
                "Artifact ({} bytes) exceeds maximum fetchable payload of {max_artifact_bytes} bytes inside 16 MB gRPC frame",
                bytecode.len()
            )));
        }

        match self
            .artifact_store
            .store_artifact_verified(&bytecode, &req.expected_sha256)
        {
            Ok(sha256) => {
                let base = self.artifact_base_url.trim_end_matches('/');
                let artifact_url = format!("{base}/artifacts/{sha256}.wasm");
                Ok(Response::new(UploadArtifactResponse {
                    success: true,
                    artifact_sha256: sha256,
                    artifact_url,
                    message: "Artifact uploaded and indexed successfully".to_string(),
                }))
            }
            Err(e) => Err(Status::invalid_argument(format!(
                "Artifact storage or validation error: {e}"
            ))),
        }
    }

    /// Generates an ephemeral pairing token and payload for worker node onboarding.
    ///
    /// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
    async fn generate_pairing_token(
        &self,
        request: Request<GeneratePairingTokenRequest>,
    ) -> Result<Response<GeneratePairingTokenResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        if self.ca_fingerprint.is_empty() {
            return Err(Status::failed_precondition(
                "Root CA signer is not initialized; pairing enrollment is unavailable (check ANDROGRID_CA_KEY_PATH)",
            ));
        }

        let req = request.into_inner();
        let target_node_id = if req.node_id.trim().is_empty() {
            None
        } else {
            Some(req.node_id.trim().to_string())
        };

        let ttl = if req.expires_in_seconds > 0 {
            Some(req.expires_in_seconds)
        } else {
            None
        };

        let default_port = crate::pairing::strip_uri_scheme(&self.cluster_endpoint)
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .filter(|&p| p > 0)
            .unwrap_or(50051);

        let raw_endpoint = if !req.san.trim().is_empty() {
            &req.san
        } else {
            &self.cluster_endpoint
        };
        let cluster_endpoint =
            crate::pairing::normalize_cluster_endpoint(raw_endpoint, default_port)
                .map_err(Status::invalid_argument)?;

        let record = self
            .token_store
            .generate_token(target_node_id.clone(), ttl)
            .map_err(|e| Status::failed_precondition(e.to_string()))?;

        let payload = PairingPayload {
            cluster_endpoint: cluster_endpoint.clone(),
            ca_fingerprint: self.ca_fingerprint.clone(),
            pairing_token: record.token.clone(),
            node_id: target_node_id.clone(),
            expires_at_epoch_s: record.expires_at_epoch_s,
        };

        let payload_json = payload
            .to_compact_json()
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(GeneratePairingTokenResponse {
            pairing_token: record.token,
            cluster_endpoint,
            ca_fingerprint: self.ca_fingerprint.clone(),
            node_id: target_node_id.unwrap_or_default(),
            expires_at_epoch_seconds: record.expires_at_epoch_s,
            pairing_payload_json: payload_json,
        }))
    }

    /// Enqueues a partitioned TaskGroup workload backed by content-addressed artifacts.
    ///
    /// Traces to: [REQ-OP-004], [INV-004], [INV-006], [NFR-PERF-001]
    async fn submit_task_group(
        &self,
        request: Request<SubmitTaskGroupRequest>,
    ) -> Result<Response<SubmitTaskGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let group = req.group.ok_or_else(|| {
            Status::invalid_argument("group field is required in SubmitTaskGroupRequest")
        })?;
        let template = group.template.as_ref().ok_or_else(|| {
            Status::invalid_argument("template field is required in TaskGroupSpec")
        })?;

        self.validate_wasm_artifact(template.engine, &[], &template.artifact_sha256)?;

        self.scheduler
            .submit_task_group(group, Self::current_epoch_ms())
            .map(Response::new)
            .map_err(Status::from)
    }

    /// Atomically replaces a TaskGroup workload with a new epoch.
    ///
    /// Traces to: [INV-004], [NFR-REL-001], [NFR-PERF-001]
    async fn replace_task_group(
        &self,
        request: Request<ReplaceTaskGroupRequest>,
    ) -> Result<Response<ReplaceTaskGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let template = req.template.as_ref().ok_or_else(|| {
            Status::invalid_argument("template field is required in ReplaceTaskGroupRequest")
        })?;

        self.validate_wasm_artifact(template.engine, &[], &template.artifact_sha256)?;

        self.scheduler
            .replace_task_group(req, Self::current_epoch_ms())
            .map(Response::new)
            .map_err(Status::from)
    }

    /// Cancels an active or draining TaskGroup.
    ///
    /// Traces to: [REQ-OP-004], [INV-004]
    async fn cancel_task_group(
        &self,
        request: Request<CancelTaskGroupRequest>,
    ) -> Result<Response<CancelTaskGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        if req.group_id.is_empty() {
            return Err(Status::invalid_argument("group_id must not be empty"));
        }

        self.scheduler
            .cancel_task_group(&req.group_id, req.expected_epoch, Self::current_epoch_ms())
            .map(Response::new)
            .map_err(Status::from)
    }

    /// Retrieves aggregate status and optional per-partition details for a TaskGroup.
    ///
    /// Traces to: [REQ-OP-004], [NFR-PERF-001]
    async fn get_task_group_status(
        &self,
        request: Request<GetTaskGroupStatusRequest>,
    ) -> Result<Response<GetTaskGroupStatusResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        if req.group_id.is_empty() {
            return Err(Status::invalid_argument("group_id must not be empty"));
        }

        self.scheduler
            .get_task_group_status(&req.group_id, req.include_tasks)
            .map(Response::new)
            .map_err(Status::from)
    }

    /// Lists all registered TaskGroups.
    ///
    /// Traces to: [REQ-OP-004]
    async fn list_task_groups(
        &self,
        request: Request<ListTaskGroupsRequest>,
    ) -> Result<Response<ListTaskGroupsResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let state_filter = if req.state_filter == 0 {
            None
        } else {
            TaskGroupState::try_from(req.state_filter).ok()
        };
        let groups = self.scheduler.list_task_groups(state_filter);
        Ok(Response::new(ListTaskGroupsResponse { groups }))
    }

    /// Creates a new logical NodeGroup for fleet partitioning.
    ///
    /// Traces to: [REQ-OP-001], [INV-005], [INV-006], [REQ-VER-002]
    async fn create_node_group(
        &self,
        request: Request<CreateNodeGroupRequest>,
    ) -> Result<Response<CreateNodeGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let res = self
            .scheduler
            .create_node_group(req, Self::current_epoch_ms())?;
        Ok(Response::new(res))
    }

    /// Updates an existing NodeGroup's description or label selector.
    ///
    /// Traces to: [REQ-OP-001], [REQ-VER-002]
    async fn update_node_group(
        &self,
        request: Request<UpdateNodeGroupRequest>,
    ) -> Result<Response<UpdateNodeGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let res = self
            .scheduler
            .update_node_group(req, Self::current_epoch_ms())?;
        Ok(Response::new(res))
    }

    /// Lists all registered NodeGroups.
    ///
    /// Traces to: [REQ-OP-001], [NFR-PERF-001]
    async fn list_node_groups(
        &self,
        request: Request<ListNodeGroupsRequest>,
    ) -> Result<Response<ListNodeGroupsResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let groups = self.scheduler.list_node_groups();
        Ok(Response::new(ListNodeGroupsResponse { groups }))
    }

    /// Explicitly assigns or unassigns worker nodes to a NodeGroup.
    ///
    /// Traces to: [REQ-OP-001], [NFR-REL-001]
    async fn assign_nodes_to_group(
        &self,
        request: Request<AssignNodesToGroupRequest>,
    ) -> Result<Response<AssignNodesToGroupResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let res = self
            .scheduler
            .assign_nodes_to_group(req, Self::current_epoch_ms())?;
        Ok(Response::new(res))
    }

    /// Drains or undrains targeted worker nodes.
    ///
    /// Traces to: [REQ-OP-002], [REQ-OP-003], [NFR-REL-001]
    async fn drain_nodes(
        &self,
        request: Request<DrainNodesRequest>,
    ) -> Result<Response<DrainNodesResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let res = self.scheduler.drain_nodes(req)?;
        Ok(Response::new(res))
    }

    /// Signals a target client version upgrade to targeted worker nodes.
    ///
    /// Traces to: [REQ-OP-002], [INV-003], [NFR-POW-001]
    async fn upgrade_nodes(
        &self,
        request: Request<UpgradeNodesRequest>,
    ) -> Result<Response<UpgradeNodesResponse>, Status> {
        Self::authorize_admin_peer(&request)?;
        let req = request.into_inner();
        let res = self.scheduler.upgrade_nodes(req)?;
        Ok(Response::new(res))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionEngine, NetworkTransportType, NodeCapabilities, TaskSpec, TaskState};

    #[allow(deprecated)]
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
            group_id: String::new(),
            group_epoch: 0,
            node_group_affinity: String::new(),
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

    #[tokio::test]
    async fn test_generate_pairing_token() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let token_store = Arc::new(PairingTokenStore::default());
        let service = AdminServiceImpl::with_services(
            scheduler,
            Arc::new(ArtifactStore::new()),
            "http://127.0.0.1:50052".to_string(),
            Arc::clone(&token_store),
            "fingerprint_12345".to_string(),
            "https://10.0.0.1:50051".to_string(),
        );

        let req = GeneratePairingTokenRequest {
            node_id: "test-node-pair".to_string(),
            expires_in_seconds: 300,
            san: "192.168.1.100:50051".to_string(),
        };

        let res = service
            .generate_pairing_token(Request::new(req))
            .await
            .expect("Generate pairing token must succeed");

        let inner = res.into_inner();
        assert_eq!(inner.node_id, "test-node-pair");
        assert_eq!(inner.ca_fingerprint, "fingerprint_12345");
        assert_eq!(inner.cluster_endpoint, "192.168.1.100:50051");
        assert_eq!(inner.pairing_token.len(), 64);
        assert!(inner.pairing_payload_json.contains("test-node-pair"));
        assert!(inner.pairing_payload_json.contains(&inner.pairing_token));
        assert!(!inner.pairing_payload_json.contains("https://"));
    }

    #[tokio::test]
    async fn test_generate_pairing_token_strips_uri_scheme_from_san_and_default() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        let token_store = Arc::new(PairingTokenStore::default());
        let service = AdminServiceImpl::with_services(
            scheduler,
            artifact_store,
            "http://127.0.0.1:8080".to_string(),
            Arc::clone(&token_store),
            "fp_abc".to_string(),
            "https://10.0.0.1:50051".to_string(),
        );

        let res_default = service
            .generate_pairing_token(Request::new(GeneratePairingTokenRequest {
                node_id: "node-1".to_string(),
                expires_in_seconds: 300,
                san: "".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(res_default.cluster_endpoint, "10.0.0.1:50051");

        let res_san = service
            .generate_pairing_token(Request::new(GeneratePairingTokenRequest {
                node_id: "node-2".to_string(),
                expires_in_seconds: 300,
                san: "https://192.168.1.42:50051".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(res_san.cluster_endpoint, "192.168.1.42:50051");
    }

    #[allow(deprecated)]
    #[tokio::test]
    async fn test_submit_task_rejects_missing_external_artifact() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        let token_store = Arc::new(PairingTokenStore::default());
        let service = AdminServiceImpl::with_services(
            scheduler,
            Arc::clone(&artifact_store),
            "http://127.0.0.1:50052".to_string(),
            token_store,
            "fp_abc".to_string(),
            "https://127.0.0.1:50051".to_string(),
        );

        let missing_sha = "a".repeat(64);
        let err = service
            .submit_task(Request::new(SubmitTaskRequest {
                task: Some(TaskSpec {
                    task_id: "missing-artifact-task".to_string(),
                    engine: ExecutionEngine::WasmWasi as i32,
                    artifact_sha256: missing_sha,
                    artifact_url: "https://external.example.com/mod.wasm".to_string(),
                    ..Default::default()
                }),
            }))
            .await
            .expect_err("Missing artifact in store must fail fast");
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);

        let wasm_bytes = b"\x00asm\x01\x00\x00\x00".to_vec();
        let upload = service
            .upload_artifact(Request::new(UploadArtifactRequest {
                bytecode: wasm_bytes,
                expected_sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        let ok = service
            .submit_task(Request::new(SubmitTaskRequest {
                task: Some(TaskSpec {
                    task_id: "uploaded-artifact-task".to_string(),
                    engine: ExecutionEngine::WasmWasi as i32,
                    artifact_sha256: upload.artifact_sha256,
                    artifact_url: upload.artifact_url,
                    ..Default::default()
                }),
            }))
            .await
            .expect("Uploaded artifact task must be accepted")
            .into_inner();
        assert!(ok.accepted);
    }

    #[tokio::test]
    async fn test_fetch_artifact_round_trip_and_errors() {
        use crate::andro_grid_service_server::AndroGridService;
        use crate::transport::service::AndroGridServiceImpl;
        use crate::FetchArtifactRequest;

        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        let wasm_bytes = b"\x00asm\x01\x00\x00\x00".to_vec();
        let sha256 = artifact_store.store_artifact(&wasm_bytes).unwrap();

        let svc = AndroGridServiceImpl::with_artifact_store(scheduler, artifact_store);

        let fetched = svc
            .fetch_artifact(Request::new(FetchArtifactRequest {
                artifact_sha256: sha256.clone(),
                node_id: "node-test-01".to_string(),
            }))
            .await
            .expect("FetchArtifact must return stored bytecode")
            .into_inner();
        assert_eq!(fetched.wasm_bytecode, wasm_bytes);
        assert_eq!(fetched.artifact_sha256, sha256);

        let not_found = svc
            .fetch_artifact(Request::new(FetchArtifactRequest {
                artifact_sha256: "b".repeat(64),
                node_id: "node-test-01".to_string(),
            }))
            .await
            .expect_err("Unknown sha256 must return NotFound");
        assert_eq!(not_found.code(), tonic::Code::NotFound);

        let invalid = svc
            .fetch_artifact(Request::new(FetchArtifactRequest {
                artifact_sha256: "../etc/passwd".to_string(),
                node_id: "node-test-01".to_string(),
            }))
            .await
            .expect_err("Invalid sha256 must return InvalidArgument");
        assert_eq!(invalid.code(), tonic::Code::InvalidArgument);

        let empty_node = svc
            .fetch_artifact(Request::new(FetchArtifactRequest {
                artifact_sha256: sha256,
                node_id: String::new(),
            }))
            .await
            .expect_err("Empty node_id must return InvalidArgument");
        assert_eq!(empty_node.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_fetch_artifact_rejects_oversized_envelope() {
        use crate::andro_grid_service_server::AndroGridService;
        use crate::transport::service::AndroGridServiceImpl;
        use crate::{FetchArtifactRequest, MAX_FRAME_SIZE_BYTES};

        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        // Exact MAX_FRAME_SIZE_BYTES raw bytecode exceeds 16 MB once protobuf framing + sha256 are encoded
        let exact_limit_bytes = vec![0x61u8; MAX_FRAME_SIZE_BYTES];
        let sha256 = artifact_store.store_artifact(&exact_limit_bytes).unwrap();

        let svc = AndroGridServiceImpl::with_artifact_store(scheduler, artifact_store);
        let err = svc
            .fetch_artifact(Request::new(FetchArtifactRequest {
                artifact_sha256: sha256,
                node_id: "node-test-01".to_string(),
            }))
            .await
            .expect_err("16 MB payload + protobuf framing must be rejected");
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    }

    #[tokio::test]
    async fn test_upload_artifact_rejects_16mb_envelope_overflow() {
        use crate::admin_service_server::AdminService;
        use crate::{UploadArtifactRequest, MAX_FRAME_SIZE_BYTES};

        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        let svc = AdminServiceImpl::with_artifact_store(
            scheduler,
            artifact_store,
            "http://127.0.0.1:50052".to_string(),
        );

        let err = svc
            .upload_artifact(Request::new(UploadArtifactRequest {
                bytecode: vec![0x61u8; MAX_FRAME_SIZE_BYTES],
                expected_sha256: String::new(),
            }))
            .await
            .expect_err("16 MB raw upload would overflow FetchArtifactResponse envelope");
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    }

    #[tokio::test]
    async fn test_submit_task_rejects_caller_group_spoofing() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let svc = AdminServiceImpl::new(scheduler);
        let mut spec = make_test_task("spoof-task");
        spec.group_id = "fake-group".to_string();
        spec.group_epoch = 1;

        let err = svc
            .submit_task(Request::new(SubmitTaskRequest { task: Some(spec) }))
            .await
            .expect_err("Standalone SubmitTask must reject caller-supplied group_id/group_epoch");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_admin_task_group_round_trip_and_authz_guard() {
        use crate::{ReplaceStrategy, TaskGroupSpec, TaskPartitionSpec, TaskTemplate};

        let scheduler = Arc::new(ClusterScheduler::new());
        let artifact_store = Arc::new(ArtifactStore::new());
        let sha = artifact_store
            .store_artifact(b"\x00asm\x01\x00\x00\x00")
            .unwrap();
        let svc = AdminServiceImpl::with_artifact_store(
            scheduler,
            artifact_store,
            "http://127.0.0.1:50052".to_string(),
        );

        let mut worker_req = Request::new(ListTaskGroupsRequest { state_filter: 0 });
        worker_req.extensions_mut().insert(AuthenticatedPeer {
            common_name: Some("worker-01".to_string()),
            organizational_unit: Some("Worker".to_string()),
            not_before_epoch_secs: 0,
            not_after_epoch_secs: i64::MAX,
            leaf_fingerprint_sha256: [0u8; 32],
            spki_der: Vec::new(),
        });
        let authz_err = svc
            .list_task_groups(worker_req)
            .await
            .expect_err("Worker OU must be denied on AdminService TaskGroup RPCs");
        assert_eq!(authz_err.code(), tonic::Code::PermissionDenied);

        let mut admin_req = Request::new(SubmitTaskGroupRequest {
            group: Some(TaskGroupSpec {
                group_id: "adm-grp".to_string(),
                template: Some(TaskTemplate {
                    engine: crate::ExecutionEngine::WasmWasi as i32,
                    timeout_seconds: 60,
                    entrypoint: "_start".to_string(),
                    env_vars: std::collections::HashMap::new(),
                    artifact_sha256: sha.clone(),
                    shell_command: String::new(),
                    requires_unmetered: false,
                    node_group_affinity: String::new(),
                }),
                partitions: vec![TaskPartitionSpec::default()],
            }),
        });
        admin_req.extensions_mut().insert(AuthenticatedPeer {
            common_name: Some("ops".to_string()),
            organizational_unit: Some("Control Plane".to_string()),
            not_before_epoch_secs: 0,
            not_after_epoch_secs: i64::MAX,
            leaf_fingerprint_sha256: [0u8; 32],
            spki_der: Vec::new(),
        });
        let sub_res = svc.submit_task_group(admin_req).await.unwrap().into_inner();
        assert!(sub_res.accepted);
        assert_eq!(sub_res.group_epoch, 1);

        let rep_res = svc
            .replace_task_group(Request::new(ReplaceTaskGroupRequest {
                group_id: "adm-grp".to_string(),
                expected_epoch: Some(1),
                template: Some(TaskTemplate {
                    engine: crate::ExecutionEngine::WasmWasi as i32,
                    timeout_seconds: 60,
                    entrypoint: "_start".to_string(),
                    env_vars: std::collections::HashMap::new(),
                    artifact_sha256: sha,
                    shell_command: String::new(),
                    requires_unmetered: false,
                    node_group_affinity: String::new(),
                }),
                partitions: vec![TaskPartitionSpec::default(), TaskPartitionSpec::default()],
                strategy: ReplaceStrategy::Immediate as i32,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(rep_res.new_epoch, 2);
        assert_eq!(rep_res.created_tasks, 2);
    }

    #[tokio::test]
    async fn test_admin_service_node_group_and_fleet_ops_mtls() {
        let scheduler = Arc::new(ClusterScheduler::new());
        let svc = AdminServiceImpl::new(Arc::clone(&scheduler));

        // Non-admin peer rejected across all NodeGroup / fleet RPCs
        let worker_peer = AuthenticatedPeer {
            common_name: Some("pixel-1".to_string()),
            organizational_unit: Some("Edge Compute".to_string()),
            not_before_epoch_secs: 0,
            not_after_epoch_secs: i64::MAX,
            leaf_fingerprint_sha256: [0u8; 32],
            spki_der: Vec::new(),
        };
        let mut worker_req = Request::new(CreateNodeGroupRequest {
            group_id: "rack-1".to_string(),
            description: String::new(),
            label_selector: std::collections::HashMap::new(),
            initial_node_ids: Vec::new(),
        });
        worker_req.extensions_mut().insert(worker_peer.clone());
        assert_eq!(
            svc.create_node_group(worker_req).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );

        let mut r_upd = Request::new(UpdateNodeGroupRequest {
            group_id: "rack-1".to_string(),
            description: None,
            label_selector: std::collections::HashMap::new(),
            replace_label_selector: false,
        });
        r_upd.extensions_mut().insert(worker_peer.clone());
        assert_eq!(
            svc.update_node_group(r_upd).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );

        let mut r_list = Request::new(ListNodeGroupsRequest {});
        r_list.extensions_mut().insert(worker_peer.clone());
        assert_eq!(
            svc.list_node_groups(r_list).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );

        let mut r_assign = Request::new(AssignNodesToGroupRequest {
            group_id: "rack-1".to_string(),
            node_ids: vec!["pixel-1".to_string()],
            unassign: false,
        });
        r_assign.extensions_mut().insert(worker_peer.clone());
        assert_eq!(
            svc.assign_nodes_to_group(r_assign)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::PermissionDenied
        );

        let mut r_drain = Request::new(DrainNodesRequest {
            node_group: "rack-1".to_string(),
            node_ids: Vec::new(),
            drain: true,
        });
        r_drain.extensions_mut().insert(worker_peer.clone());
        assert_eq!(
            svc.drain_nodes(r_drain).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );

        let mut r_upg = Request::new(UpgradeNodesRequest {
            node_group: "rack-1".to_string(),
            node_ids: Vec::new(),
            target_version: "0.2.0".to_string(),
            drain_before_upgrade: false,
        });
        r_upg.extensions_mut().insert(worker_peer);
        assert_eq!(
            svc.upgrade_nodes(r_upg).await.unwrap_err().code(),
            tonic::Code::PermissionDenied
        );

        // Admin peer creates group
        let mut admin_req = Request::new(CreateNodeGroupRequest {
            group_id: "rack-1".to_string(),
            description: "Rack 1".to_string(),
            label_selector: std::collections::HashMap::new(),
            initial_node_ids: Vec::new(),
        });
        admin_req.extensions_mut().insert(AuthenticatedPeer {
            common_name: Some("ops".to_string()),
            organizational_unit: Some("Control Plane".to_string()),
            not_before_epoch_secs: 0,
            not_after_epoch_secs: i64::MAX,
            leaf_fingerprint_sha256: [0u8; 32],
            spki_der: Vec::new(),
        });
        let created = svc.create_node_group(admin_req).await.unwrap().into_inner();
        assert!(created.created);

        // SubmitTask with unknown node_group_affinity returns NOT_FOUND
        let mut bad_task = make_test_task("t-missing-grp");
        bad_task.node_group_affinity = "does-not-exist".to_string();
        let task_err = svc
            .submit_task(Request::new(SubmitTaskRequest {
                task: Some(bad_task),
            }))
            .await
            .unwrap_err();
        assert_eq!(task_err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_submit_task_rejects_apk_blob_kind_for_wasm_execution() {
        use crate::admin_service_server::AdminService;
        use crate::artifact::store::{BlobKind, TempFileGuard};

        let temp_dir = std::env::temp_dir().join("androgrid_admin_apk_reject_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let artifact_store = Arc::new(ArtifactStore::with_directory(&temp_dir));
        let payload = b"fake-apk-archive-bytes";
        let sha = ArtifactStore::compute_sha256(payload);
        let p = artifact_store.next_temp_path();
        tokio::fs::write(&p, payload).await.unwrap();
        artifact_store
            .commit_temp_blob(
                TempFileGuard::new(p),
                sha.clone(),
                payload.len() as u64,
                BlobKind::Apk,
            )
            .await
            .unwrap();

        let svc = AdminServiceImpl::with_artifact_store(
            Arc::new(ClusterScheduler::new()),
            artifact_store,
            "http://127.0.0.1:50052".to_string(),
        );
        let mut task = make_test_task("t-apk-as-wasm");
        task.wasm_bytecode.clear();
        task.artifact_sha256 = sha;

        let err = svc
            .submit_task(Request::new(SubmitTaskRequest { task: Some(task) }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
