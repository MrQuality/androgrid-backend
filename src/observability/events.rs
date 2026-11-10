//! Cluster Observability Real-Time Event Models
//!
//! Exposes strongly-typed events emitted across the cluster scheduler lifecycle,
//! including worker node registration, heartbeat updates, task leasing, completions,
//! and speculative spot preemption resolution.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

use serde::{Deserialize, Serialize};

/// Broadcast cluster event emitted during state transitions.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ClusterEvent {
    /// Worker node registered with cluster.
    NodeRegistered {
        node_id: String,
        transport: String,
        is_rooted: bool,
        battery_pct: u32,
        is_charging: bool,
        thermal_status: u32,
        cpu_cores: u32,
        ram_mb: u64,
        os_version: String,
    },
    /// Worker node reported state or heartbeat.
    NodeHeartbeat {
        node_id: String,
        battery_pct: u32,
        is_charging: bool,
        thermal_status: u32,
        last_seen_epoch_ms: u64,
    },
    /// New compute task submitted.
    TaskSubmitted {
        task_id: String,
        engine: String,
        timeout_seconds: u32,
        requires_unmetered: bool,
    },
    /// Task leased to a worker node.
    TaskLeased {
        task_id: String,
        node_id: String,
        run_attempt: u32,
    },
    /// Speculative spot preemption initiated per [INV-005].
    TaskPreempted {
        task_id: String,
        original_node_id: Option<String>,
        speculative_node_id: String,
        new_attempt: u32,
    },
    /// Task successfully finalized by a worker node.
    TaskCompleted {
        task_id: String,
        node_id: String,
        run_attempt: u32,
        exit_code: i32,
        execution_duration_ms: u64,
    },
    /// Task failed or terminated with non-zero exit code.
    TaskFailed {
        task_id: String,
        node_id: String,
        run_attempt: u32,
        error_code: String,
        error_message: String,
    },
    /// Task cancelled by operator.
    TaskCancelled { task_id: String },
    /// Task lease timed out.
    TaskTimedOut { task_id: String },
}
