//! Cluster Observability State Snapshot Models
//!
//! Provides point-in-time snapshots of connected worker nodes, task queues,
//! and aggregate performance metrics for both HTTP dashboard and Terminal TUI.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

use serde::{Deserialize, Serialize};

/// High-level cluster health and workload metrics.
///
/// Traces to: [REQ-OP-001], [REQ-OP-004]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMetrics {
    pub total_nodes: usize,
    pub wifi_nodes: usize,
    pub cellular_nodes: usize,
    pub total_tasks: usize,
    pub pending_tasks: usize,
    pub leased_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub preempted_tasks: usize,
    pub timed_out_tasks: usize,
}

/// Worker node state snapshot.
///
/// Traces to: [REQ-OP-001], [INV-006]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerNodeSnapshot {
    pub node_id: String,
    pub is_rooted: bool,
    pub transport: String,
    pub battery_level_pct: u32,
    pub is_charging: bool,
    pub thermal_status: u32,
    pub thermal_badge: String,
    pub cpu_cores: u32,
    pub ram_mb: u64,
    pub os_version: String,
    pub abi: String,
    pub client_version: String,
    pub last_seen_epoch_ms: u64,
}

/// Compute task state snapshot.
///
/// Traces to: [REQ-OP-004], [INV-004], [INV-005]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    pub state: String,
    pub engine: String,
    pub run_attempt: u32,
    pub assigned_node_id: Option<String>,
    pub speculative_node_id: Option<String>,
    pub timeout_seconds: u32,
    pub requires_unmetered: bool,
    pub exit_code: Option<i32>,
    pub execution_duration_ms: Option<u64>,
    pub error_message: Option<String>,
}

/// Comprehensive cluster state snapshot for dashboard and TUI consumption.
///
/// Traces to: [REQ-OP-001], [REQ-OP-004], [INV-004], [INV-005]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSnapshot {
    pub nodes: Vec<WorkerNodeSnapshot>,
    pub tasks: Vec<TaskSnapshot>,
    pub metrics: ClusterMetrics,
    pub timestamp_epoch_ms: u64,
}
