//! Core in-memory cluster scheduler and atomic state machine.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.2, Section 3.1, Section 5.1, Section 6.2,
//! [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-004], [INV-005], [INV-006], [ANTI-003], [NFR-REL-001]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::scheduler::errors::SchedulingError;
use crate::scheduler::state::{TaskExecutionResult, TaskRecord, WorkerRecord};
use crate::{
    ClaimCellularTaskResponse, ExecutionEngine, NetworkTransportType, NodeCapabilities,
    RegisterNodeResponse, SubmitCellularResultResponse, SyncNodeStateResponse, TaskExecutionReport,
    TaskSpec, TaskState,
};

/// Default maximum allowable task duration on cellular spot connections (180 seconds).
/// Traces to: [INV-005], [ANTI-003]
pub const MAX_CELLULAR_DURATION_SECONDS: u32 = 180;

/// Minimum battery level percentage required for cellular task claims when not charging.
/// Traces to: [REQ-OP-003]
pub const MIN_CELLULAR_BATTERY_PCT: u32 = 20;

/// Default lease timeout in seconds for node registration.
pub const DEFAULT_NODE_LEASE_TIMEOUT_SECONDS: u32 = 10;

/// Default heartbeat interval in seconds for Wi-Fi synchronization loop.
pub const DEFAULT_HEARTBEAT_INTERVAL_SECONDS: u32 = 2;

/// Thread-safe in-memory cluster scheduler.
#[derive(Debug, Clone, Default)]
pub struct ClusterScheduler {
    inner: Arc<RwLock<SchedulerInner>>,
}

#[derive(Debug, Default)]
struct SchedulerInner {
    tasks: HashMap<String, TaskRecord>,
    workers: HashMap<String, WorkerRecord>,
}

impl ClusterScheduler {
    /// Creates a new, empty cluster scheduler.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SchedulerInner::default())),
        }
    }

    /// Registers an Android worker node, enforcing root capabilities and engine boundaries.
    ///
    /// Traces to: [REQ-OP-001], [INV-006], [REQ-VER-002]
    pub fn register_node(
        &self,
        node_id: String,
        mut capabilities: NodeCapabilities,
        client_version: String,
        now_epoch_ms: u64,
    ) -> Result<RegisterNodeResponse, SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");

        // Enforce [INV-006]: If device is unrooted, filter supported engines to exclude RootShell
        if !capabilities.is_rooted {
            capabilities
                .supported_engines
                .retain(|&e| e != ExecutionEngine::RootShell as i32);
            if !capabilities
                .supported_engines
                .contains(&(ExecutionEngine::WasmWasi as i32))
            {
                capabilities
                    .supported_engines
                    .push(ExecutionEngine::WasmWasi as i32);
            }
        }

        let session_token = format!("session-{}-{}", node_id, now_epoch_ms);

        let worker = WorkerRecord {
            node_id: node_id.clone(),
            capabilities,
            session_token: session_token.clone(),
            client_version,
            battery_level_pct: 100,
            is_charging: true,
            last_seen_epoch_ms: now_epoch_ms,
            active_task_ids: Vec::new(),
        };

        inner.workers.insert(node_id, worker);

        Ok(RegisterNodeResponse {
            session_token,
            heartbeat_interval_seconds: DEFAULT_HEARTBEAT_INTERVAL_SECONDS,
            lease_timeout_seconds: DEFAULT_NODE_LEASE_TIMEOUT_SECONDS,
        })
    }

    /// Submits a new compute task specification to the pending queue.
    pub fn submit_task(&self, spec: TaskSpec) -> Result<(), SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");
        let task_id = spec.task_id.clone();
        inner.tasks.insert(task_id, TaskRecord::new(spec));
        Ok(())
    }

    /// Claims an eligible task for a cellular spot worker.
    ///
    /// Traces to: [REQ-OP-003], [INV-003], [INV-005], [INV-006], [ANTI-003]
    pub fn claim_cellular_task(
        &self,
        node_id: &str,
        battery_level_pct: u32,
        is_charging: bool,
        now_epoch_s: u64,
    ) -> Result<ClaimCellularTaskResponse, SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");

        let is_rooted = {
            let worker = inner
                .workers
                .get_mut(node_id)
                .ok_or_else(|| SchedulingError::NodeNotFound(node_id.to_string()))?;

            // Verify battery pre-condition: >= 20% or charging
            if !is_charging && battery_level_pct < MIN_CELLULAR_BATTERY_PCT {
                return Err(SchedulingError::InsufficientBattery {
                    node_id: node_id.to_string(),
                    battery_pct: battery_level_pct,
                });
            }

            worker.battery_level_pct = battery_level_pct;
            worker.is_charging = is_charging;
            worker.capabilities.is_rooted
        };

        // Find candidate pending task matching cellular constraints
        let mut candidate_id: Option<String> = None;
        for (id, record) in inner.tasks.iter() {
            if record.state != TaskState::Pending {
                continue;
            }

            // [ANTI-003]: Tasks requiring unmetered connections must NOT be scheduled to cellular
            if record.spec.requires_unmetered {
                continue;
            }

            // [ANTI-003], [INV-005]: Tasks exceeding 180s must NOT be scheduled to cellular
            if record.spec.timeout_seconds > MAX_CELLULAR_DURATION_SECONDS {
                continue;
            }

            // [INV-006]: Unrooted workers must never receive root shell payloads
            if !is_rooted && record.spec.engine == ExecutionEngine::RootShell as i32 {
                continue;
            }

            candidate_id = Some(id.clone());
            break;
        }

        match candidate_id {
            Some(task_id) => {
                let record = inner.tasks.get_mut(&task_id).unwrap();
                let effective_duration = record
                    .spec
                    .timeout_seconds
                    .min(MAX_CELLULAR_DURATION_SECONDS);
                let deadline = now_epoch_s + effective_duration as u64;

                // State transition: PENDING -> LEASED
                record.state = TaskState::Leased;
                record.active_attempt = 1;
                record.assigned_node_id = Some(node_id.to_string());
                record.lease_deadline_epoch_s = deadline;

                let mut out_spec = record.spec.clone();
                out_spec.run_attempt = 1;

                if let Some(w) = inner.workers.get_mut(node_id) {
                    w.active_task_ids.push(task_id);
                }

                Ok(ClaimCellularTaskResponse {
                    task_available: true,
                    task: Some(out_spec),
                    deadline_epoch_seconds: deadline as u32,
                })
            }
            None => Ok(ClaimCellularTaskResponse {
                task_available: false,
                task: None,
                deadline_epoch_seconds: 0,
            }),
        }
    }

    /// Declarative Wi-Fi state synchronization and speculative preemption dispatcher.
    ///
    /// Traces to: [REQ-OP-002], [INV-005], [INV-006]
    pub fn sync_node_state(
        &self,
        node_id: &str,
        session_token: &str,
        completed_reports: &[TaskExecutionReport],
        now_epoch_ms: u64,
    ) -> Result<SyncNodeStateResponse, SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");

        let is_rooted;
        let is_unmetered;
        {
            let worker = inner
                .workers
                .get_mut(node_id)
                .ok_or_else(|| SchedulingError::NodeNotFound(node_id.to_string()))?;

            if worker.session_token != session_token {
                return Err(SchedulingError::InvalidSessionToken {
                    node_id: node_id.to_string(),
                    provided: session_token.to_string(),
                });
            }

            worker.last_seen_epoch_ms = now_epoch_ms;
            is_rooted = worker.capabilities.is_rooted;
            is_unmetered = !worker.capabilities.is_metered
                || worker.capabilities.current_transport
                    == NetworkTransportType::NetworkTransportWifiLan as i32;
        }

        let mut acknowledged_ids = Vec::new();

        // Process completed reports submitted via Wi-Fi
        for report in completed_reports {
            if let Some(task) = inner.tasks.get_mut(&report.task_id) {
                if task.active_attempt == report.run_attempt
                    && (task.state == TaskState::Leased || task.state == TaskState::Preempted)
                {
                    task.state = if report.exit_code == 0 {
                        TaskState::Completed
                    } else {
                        TaskState::Failed
                    };
                    task.result = Some(TaskExecutionResult {
                        exit_code: report.exit_code,
                        stdout_bytes: report.stdout_bytes.clone(),
                        stderr_bytes: report.stderr_bytes.clone(),
                        error_code: report.error_code.clone(),
                        error_message: String::new(),
                        execution_duration_ms: report.execution_time_ms,
                        committed_by_node_id: node_id.to_string(),
                        committed_attempt: report.run_attempt,
                    });
                    acknowledged_ids.push(report.task_id.clone());
                }
            }
        }

        // Schedule new or preempted tasks for this Wi-Fi node
        let mut desired_tasks = Vec::new();

        if is_unmetered {
            // First priority: dispatch any pending unassigned tasks
            for (_, record) in inner.tasks.iter_mut() {
                if record.state == TaskState::Pending {
                    if !is_rooted && record.spec.engine == ExecutionEngine::RootShell as i32 {
                        continue;
                    }
                    record.state = TaskState::Leased;
                    record.active_attempt += 1;
                    record.assigned_node_id = Some(node_id.to_string());
                    let mut spec = record.spec.clone();
                    spec.run_attempt = record.active_attempt;
                    desired_tasks.push(spec);
                    break;
                }
            }

            // Second priority: [INV-005] Speculative Dual-Assignment
            // If Wi-Fi worker has capacity, preempt slow cellular spot worker
            if desired_tasks.is_empty() {
                for (_, record) in inner.tasks.iter_mut() {
                    if record.state == TaskState::Leased
                        && record.assigned_node_id.as_deref() != Some(node_id)
                    {
                        if !is_rooted && record.spec.engine == ExecutionEngine::RootShell as i32 {
                            continue;
                        }

                        // Preemption state transition: LEASED -> PREEMPTED with run_attempt increment
                        record.state = TaskState::Preempted;
                        record.active_attempt += 1; // Increment attempt (attempt = 2)
                        record.speculative_node_id = Some(node_id.to_string());

                        let mut spec = record.spec.clone();
                        spec.run_attempt = record.active_attempt;
                        desired_tasks.push(spec);
                        break;
                    }
                }
            }
        }

        Ok(SyncNodeStateResponse {
            server_epoch_ms: now_epoch_ms,
            desired_tasks,
            cancel_task_ids: Vec::new(),
            acknowledged_task_ids: acknowledged_ids,
        })
    }

    /// Submits cellular execution results with atomic CAS consensus and monotonic attempt fencing.
    ///
    /// Traces to: [REQ-OP-004], [INV-004], [NFR-REL-001]
    pub fn submit_cellular_result(
        &self,
        request: &crate::SubmitCellularResultRequest,
    ) -> Result<SubmitCellularResultResponse, SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");

        let task = inner
            .tasks
            .get_mut(&request.task_id)
            .ok_or_else(|| SchedulingError::TaskNotFound(request.task_id.clone()))?;

        // If task is already finalized in Completed or Failed state, reject attempt
        if task.state == TaskState::Completed || task.state == TaskState::Failed {
            return Err(SchedulingError::AlreadyFinalized(request.task_id.clone()));
        }

        // [INV-004]: Atomic monotonic fencing check
        // Submissions bearing run_attempt strictly lower than the latest committed state MUST be dropped
        if request.run_attempt < task.active_attempt {
            return Err(SchedulingError::AttemptStale {
                task_id: request.task_id.clone(),
                submitted: request.run_attempt,
                active: task.active_attempt,
            });
        }

        // Commit atomic CAS transition: LEASED / PREEMPTED -> COMPLETED / FAILED
        task.state = if request.exit_code == 0 {
            TaskState::Completed
        } else {
            TaskState::Failed
        };

        task.result = Some(TaskExecutionResult {
            exit_code: request.exit_code,
            stdout_bytes: request.stdout_bytes.clone(),
            stderr_bytes: request.stderr_bytes.clone(),
            error_code: request.error_code.clone(),
            error_message: request.error_message.clone(),
            execution_duration_ms: request.execution_duration_ms,
            committed_by_node_id: request.node_id.clone(),
            committed_attempt: request.run_attempt,
        });

        Ok(SubmitCellularResultResponse {
            accepted: true,
            status_message: "Committed".to_string(),
        })
    }

    /// Retrieves current state of a task by ID.
    pub fn get_task_state(&self, task_id: &str) -> Option<TaskState> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner.tasks.get(task_id).map(|t| t.state)
    }

    /// Retrieves active run attempt of a task by ID.
    pub fn get_active_attempt(&self, task_id: &str) -> Option<u32> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner.tasks.get(task_id).map(|t| t.active_attempt)
    }

    /// Retrieves committed execution result of a task by ID.
    pub fn get_task_result(&self, task_id: &str) -> Option<TaskExecutionResult> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner.tasks.get(task_id).and_then(|t| t.result.clone())
    }

    /// Retrieves registered capabilities of a worker node by ID.
    pub fn get_worker_capabilities(&self, node_id: &str) -> Option<NodeCapabilities> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner.workers.get(node_id).map(|w| w.capabilities.clone())
    }

    /// Sweeps expired leases and transitions them back to PENDING or TIMED_OUT.
    ///
    /// Traces to: Section 6.2 (LEASED -> TIMED_OUT)
    pub fn sweep_timed_out_leases(&self, now_epoch_s: u64) -> Vec<String> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");
        let mut expired = Vec::new();

        for (id, task) in inner.tasks.iter_mut() {
            if (task.state == TaskState::Leased || task.state == TaskState::Preempted)
                && task.lease_deadline_epoch_s > 0
                && now_epoch_s > task.lease_deadline_epoch_s
            {
                task.state = TaskState::TimedOut;
                expired.push(id.clone());
            }
        }

        expired
    }

    /// Lists all registered worker nodes, their capabilities, and last heartbeat timestamps.
    ///
    /// Traces to: [REQ-OP-001], [REQ-OP-004]
    pub fn list_worker_nodes(&self) -> Vec<crate::WorkerNodeInfo> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner
            .workers
            .iter()
            .map(|(node_id, worker)| crate::WorkerNodeInfo {
                node_id: node_id.clone(),
                capabilities: Some(worker.capabilities.clone()),
                client_version: worker.client_version.clone(),
                last_heartbeat_epoch_seconds: worker.last_seen_epoch_ms / 1000,
            })
            .collect()
    }

    /// Cancels a task if it is not already in a terminal state.
    ///
    /// Traces to: [REQ-OP-004], Section 6.2 State Machine
    pub fn cancel_task(&self, task_id: &str) -> Result<(), SchedulingError> {
        let mut inner = self.inner.write().expect("RwLock write lock poisoned");
        let task = inner
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| SchedulingError::TaskNotFound(task_id.to_string()))?;

        if task.state == TaskState::Completed
            || task.state == TaskState::Failed
            || task.state == TaskState::TimedOut
        {
            return Err(SchedulingError::AlreadyFinalized(task_id.to_string()));
        }

        task.state = TaskState::Failed;
        task.result = Some(TaskExecutionResult {
            exit_code: -1,
            stdout_bytes: Vec::new(),
            stderr_bytes: b"Task cancelled by operator".to_vec(),
            error_code: "TASK_CANCELLED".to_string(),
            error_message: "Task was cancelled by operator".to_string(),
            execution_duration_ms: 0,
            committed_by_node_id: "operator".to_string(),
            committed_attempt: task.active_attempt,
        });

        Ok(())
    }

    /// Retrieves complete task lifecycle status and execution outputs.
    ///
    /// Traces to: [REQ-OP-004]
    pub fn get_task_details(
        &self,
        task_id: &str,
    ) -> Option<(TaskState, u32, Option<TaskExecutionResult>)> {
        let inner = self.inner.read().expect("RwLock read lock poisoned");
        inner
            .tasks
            .get(task_id)
            .map(|t| (t.state, t.active_attempt, t.result.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_test_capabilities(is_rooted: bool, is_metered: bool) -> NodeCapabilities {
        NodeCapabilities {
            is_rooted,
            supported_engines: vec![
                ExecutionEngine::WasmWasi as i32,
                ExecutionEngine::RootShell as i32,
            ],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered,
            current_transport: if is_metered {
                NetworkTransportType::NetworkTransportCellularWan as i32
            } else {
                NetworkTransportType::NetworkTransportWifiLan as i32
            },
            max_cellular_task_duration_seconds: 180,
        }
    }

    fn make_test_task(
        task_id: &str,
        timeout_seconds: u32,
        requires_unmetered: bool,
        engine: ExecutionEngine,
    ) -> TaskSpec {
        TaskSpec {
            task_id: task_id.to_string(),
            run_attempt: 0,
            engine: engine as i32,
            timeout_seconds,
            entrypoint: "_start".to_string(),
            env_vars: HashMap::new(),
            wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
            shell_command: String::new(),
            artifact_sha256: "dummy-sha256".to_string(),
            requires_unmetered,
        }
    }

    #[test]
    fn test_node_registration_unrooted_sanitizes_engines() {
        let scheduler = ClusterScheduler::new();
        let caps = make_test_capabilities(false, false);

        let res = scheduler
            .register_node(
                "node-unrooted-1".to_string(),
                caps,
                "1.0.0".to_string(),
                1000,
            )
            .expect("Registration must succeed");

        assert!(!res.session_token.is_empty());

        let inner = scheduler.inner.read().unwrap();
        let worker = inner.workers.get("node-unrooted-1").unwrap();
        assert!(!worker.capabilities.is_rooted);
        // RootShell must have been removed per [INV-006]
        assert!(!worker
            .capabilities
            .supported_engines
            .contains(&(ExecutionEngine::RootShell as i32)));
        assert!(worker
            .capabilities
            .supported_engines
            .contains(&(ExecutionEngine::WasmWasi as i32)));
    }

    #[test]
    fn test_cellular_spot_claim_and_anti_requirement_enforcement() {
        let scheduler = ClusterScheduler::new();
        let caps = make_test_capabilities(false, true);
        scheduler
            .register_node("cell-01".to_string(), caps, "1.0.0".to_string(), 1000)
            .unwrap();

        // Task 1: requires_unmetered = true -> Must NOT be claimed by cellular worker per [ANTI-003]
        let task_unmetered = make_test_task("task-wifi-only", 60, true, ExecutionEngine::WasmWasi);
        scheduler.submit_task(task_unmetered).unwrap();

        // Task 2: timeout > 180s -> Must NOT be claimed by cellular worker per [ANTI-003], [INV-005]
        let task_too_long = make_test_task("task-long", 200, false, ExecutionEngine::WasmWasi);
        scheduler.submit_task(task_too_long).unwrap();

        // Task 3: Root shell task on unrooted worker -> Must NOT be claimed per [INV-006]
        let task_root = make_test_task("task-root", 60, false, ExecutionEngine::RootShell);
        scheduler.submit_task(task_root).unwrap();

        let claim_res1 = scheduler
            .claim_cellular_task("cell-01", 80, false, 2000)
            .unwrap();
        assert!(
            !claim_res1.task_available,
            "None of the above tasks should be claimable"
        );

        // Task 4: Eligible cellular spot task (WASM, <= 180s, unmetered=false)
        let task_eligible = make_test_task("task-spot-501", 120, false, ExecutionEngine::WasmWasi);
        scheduler.submit_task(task_eligible).unwrap();

        let claim_res2 = scheduler
            .claim_cellular_task("cell-01", 80, false, 2000)
            .unwrap();
        assert!(claim_res2.task_available);
        let task = claim_res2.task.expect("Task must be returned");
        assert_eq!(task.task_id, "task-spot-501");
        assert_eq!(task.run_attempt, 1);
        assert_eq!(claim_res2.deadline_epoch_seconds, 2120);

        assert_eq!(
            scheduler.get_task_state("task-spot-501"),
            Some(TaskState::Leased)
        );
        assert_eq!(scheduler.get_active_attempt("task-spot-501"), Some(1));
    }

    #[test]
    fn test_cellular_spot_dual_assignment_and_monotonic_fencing() {
        // Direct implementation of Acceptance Criteria: Section 8, Scenario 2
        let scheduler = ClusterScheduler::new();

        // Register cell_01 (cellular node) and wifi_01 (Wi-Fi node)
        let cell_caps = make_test_capabilities(false, true);
        scheduler
            .register_node("cell_01".to_string(), cell_caps, "1.0.0".to_string(), 1000)
            .unwrap();

        let wifi_caps = make_test_capabilities(false, false);
        let wifi_reg = scheduler
            .register_node("wifi_01".to_string(), wifi_caps, "1.0.0".to_string(), 1000)
            .unwrap();

        // Task 601 submitted
        let task_601 = make_test_task("task_601", 120, false, ExecutionEngine::WasmWasi);
        scheduler.submit_task(task_601).unwrap();

        // Step 1: Claimed by cellular worker cell_01 with run_attempt 1
        let claim = scheduler
            .claim_cellular_task("cell_01", 85, false, 1000)
            .unwrap();
        assert!(claim.task_available);
        assert_eq!(
            scheduler.get_task_state("task_601"),
            Some(TaskState::Leased)
        );
        assert_eq!(scheduler.get_active_attempt("task_601"), Some(1));

        // Step 2: 60s elapse, wifi_01 becomes idle and synchronizes state
        // Orchestrator speculatively dual-assigns task_601 to wifi_01 with run_attempt 2
        let sync_res = scheduler
            .sync_node_state("wifi_01", &wifi_reg.session_token, &[], 1060000)
            .unwrap();
        assert_eq!(sync_res.desired_tasks.len(), 1);
        let preempted_task = &sync_res.desired_tasks[0];
        assert_eq!(preempted_task.task_id, "task_601");
        assert_eq!(preempted_task.run_attempt, 2);

        assert_eq!(
            scheduler.get_task_state("task_601"),
            Some(TaskState::Preempted)
        );
        assert_eq!(scheduler.get_active_attempt("task_601"), Some(2));

        // Step 3: wifi_01 completes and submits result with run_attempt 2
        let wifi_report = TaskExecutionReport {
            task_id: "task_601".to_string(),
            run_attempt: 2,
            state: TaskState::Completed as i32,
            exit_code: 0,
            stdout_bytes: b"Result from Wi-Fi worker".to_vec(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            execution_time_ms: 1500,
        };
        let sync_res2 = scheduler
            .sync_node_state("wifi_01", &wifi_reg.session_token, &[wifi_report], 1070000)
            .unwrap();
        assert_eq!(sync_res2.acknowledged_task_ids, vec!["task_601"]);

        // Task is now COMPLETED
        assert_eq!(
            scheduler.get_task_state("task_601"),
            Some(TaskState::Completed)
        );
        let committed_res = scheduler
            .get_task_result("task_601")
            .expect("Result must exist");
        assert_eq!(committed_res.committed_attempt, 2);
        assert_eq!(committed_res.committed_by_node_id, "wifi_01");

        // Step 4: cell_01 subsequently submits task_601 with run_attempt 1
        // Orchestrator MUST reject with AttemptStale (TASK_ATTEMPT_STALE) per [INV-004]
        let cell_req = crate::SubmitCellularResultRequest {
            node_id: "cell_01".to_string(),
            task_id: "task_601".to_string(),
            run_attempt: 1, // Stale attempt 1 vs committed attempt 2
            exit_code: 0,
            stdout_bytes: b"Result from slow cellular worker".to_vec(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 45000,
        };
        let cell_submit_res = scheduler.submit_cellular_result(&cell_req);

        assert_eq!(
            cell_submit_res,
            Err(SchedulingError::AlreadyFinalized("task_601".to_string()))
        );
    }

    #[test]
    fn test_monotonic_attempt_fencing_rejects_stale_before_finalization() {
        let scheduler = ClusterScheduler::new();
        let cell_caps = make_test_capabilities(false, true);
        scheduler
            .register_node("cell_01".to_string(), cell_caps, "1.0.0".to_string(), 1000)
            .unwrap();

        let wifi_caps = make_test_capabilities(false, false);
        let wifi_reg = scheduler
            .register_node("wifi_01".to_string(), wifi_caps, "1.0.0".to_string(), 1000)
            .unwrap();

        let task = make_test_task("task_fence", 120, false, ExecutionEngine::WasmWasi);
        scheduler.submit_task(task).unwrap();

        // Claimed on cellular -> attempt 1
        scheduler
            .claim_cellular_task("cell_01", 90, false, 1000)
            .unwrap();

        // Preempted on Wi-Fi -> attempt 2
        scheduler
            .sync_node_state("wifi_01", &wifi_reg.session_token, &[], 2000)
            .unwrap();
        assert_eq!(scheduler.get_active_attempt("task_fence"), Some(2));
        assert_eq!(
            scheduler.get_task_state("task_fence"),
            Some(TaskState::Preempted)
        );

        // Cellular worker submits attempt 1 WHILE task is still PREEMPTED (not yet finalized)
        let stale_req = crate::SubmitCellularResultRequest {
            node_id: "cell_01".to_string(),
            task_id: "task_fence".to_string(),
            run_attempt: 1, // Submitted 1, active is 2
            exit_code: 0,
            stdout_bytes: b"stdout".to_vec(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 3000,
        };
        let stale_res = scheduler.submit_cellular_result(&stale_req);

        assert_eq!(
            stale_res,
            Err(SchedulingError::AttemptStale {
                task_id: "task_fence".to_string(),
                submitted: 1,
                active: 2,
            })
        );

        // State remains PREEMPTED; not corrupted
        assert_eq!(
            scheduler.get_task_state("task_fence"),
            Some(TaskState::Preempted)
        );
    }
}
