//! In-memory state models for tasks, worker nodes, and execution results.
//!
//! Traces to: AG-SPEC-2026-V2 Section 4.1, Section 6.1, Section 6.2, [INV-004], [INV-005], [INV-006]

use crate::{ExecutionEngine, NodeCapabilities, TaskGroupState, TaskSpec, TaskState, TaskTemplate};

/// Execution artifacts committed by a winning worker node.
///
/// Traces to: [REQ-OP-004], [INV-004]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExecutionResult {
    pub exit_code: i32,
    pub stdout_bytes: Vec<u8>,
    pub stderr_bytes: Vec<u8>,
    pub error_code: String,
    pub error_message: String,
    pub execution_duration_ms: u64,
    pub committed_by_node_id: String,
    pub committed_attempt: u32,
}

/// In-memory representation of a compute task lifecycle.
///
/// Traces to: Section 6.2, [INV-004], [INV-005], [INV-006]
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub spec: TaskSpec,
    pub state: TaskState,
    pub active_attempt: u32,
    pub assigned_node_id: Option<String>,
    pub speculative_node_id: Option<String>,
    pub lease_deadline_epoch_s: u64,
    pub result: Option<TaskExecutionResult>,
    pub group_id: Option<String>,
    pub group_epoch: Option<u64>,
}

impl TaskRecord {
    /// Constructs a new task record in the PENDING state.
    pub fn new(mut spec: TaskSpec) -> Self {
        spec.run_attempt = 0;
        let group_id = if spec.group_id.is_empty() {
            None
        } else {
            Some(spec.group_id.clone())
        };
        let group_epoch = if spec.group_epoch == 0 {
            None
        } else {
            Some(spec.group_epoch)
        };
        Self {
            spec,
            state: TaskState::Pending,
            active_attempt: 0,
            assigned_node_id: None,
            speculative_node_id: None,
            lease_deadline_epoch_s: 0,
            result: None,
            group_id,
            group_epoch,
        }
    }

    /// Checks whether the task is eligible for assignment to an unrooted worker node.
    ///
    /// Traces to: [INV-006]
    pub fn is_compatible_with_unrooted(&self) -> bool {
        self.spec.engine != ExecutionEngine::RootShell as i32
    }

    /// Checks whether the task is eligible for cellular spot leasing.
    ///
    /// Traces to: [INV-005], [ANTI-003]
    pub fn is_eligible_for_cellular(&self, max_cellular_duration_s: u32) -> bool {
        !self.spec.requires_unmetered && self.spec.timeout_seconds <= max_cellular_duration_s
    }
}

/// Registered worker node state in the cluster registry.
///
/// Traces to: [REQ-OP-001], [REQ-VER-002]
#[derive(Debug, Clone)]
pub struct WorkerRecord {
    pub node_id: String,
    pub capabilities: NodeCapabilities,
    pub session_token: String,
    pub client_version: String,
    pub battery_level_pct: u32,
    pub is_charging: bool,
    pub battery_temperature_celsius: i32,
    pub thermal_status: u32,
    pub last_seen_epoch_ms: u64,
    pub active_task_ids: Vec<String>,
}

impl WorkerRecord {
    /// Verifies if the worker has supported engines matching unrooted sandbox boundaries.
    ///
    /// Traces to: [INV-006]
    pub fn allows_root_execution(&self) -> bool {
        self.capabilities.is_rooted
    }
}

/// In-memory coordination record for a partitioned TaskGroup and its monotonic epoch.
///
/// Traces to: [REQ-OP-004], [INV-004], [NFR-REL-001]
#[derive(Debug, Clone)]
pub struct TaskGroupRecord {
    pub group_id: String,
    pub group_epoch: u64,
    pub state: TaskGroupState,
    pub template: TaskTemplate,
    pub current_task_ids: Vec<String>,
    pub draining_task_ids: Vec<String>,
    pub created_at_epoch_ms: u64,
    pub updated_at_epoch_ms: u64,
}
