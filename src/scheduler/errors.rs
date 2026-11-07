//! Domain error types for task scheduling, state transitions, and capability enforcement.
//!
//! Traces to: AG-SPEC-2026-V2 Section 3.1, Section 6.3, [INV-004], [INV-005], [INV-006], [ANTI-003]

use thiserror::Error;

/// Errors arising during cluster task scheduling, state transitions, and result consensus.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum SchedulingError {
    #[error("Task with ID '{0}' not found in cluster task pool")]
    TaskNotFound(String),

    #[error("Worker node with ID '{0}' not registered")]
    NodeNotFound(String),

    #[error("Session token '{provided}' is invalid for node '{node_id}'")]
    InvalidSessionToken { node_id: String, provided: String },

    #[error("Task '{0}' is already finalized in a terminal state")]
    AlreadyFinalized(String),

    /// Traces to: [INV-004], Section 6.3 error code TASK_ATTEMPT_STALE
    #[error("Task attempt {submitted} for task '{task_id}' is stale; active attempt is {active}")]
    AttemptStale {
        task_id: String,
        submitted: u32,
        active: u32,
    },

    /// Traces to: [INV-006]
    #[error("Unrooted worker '{node_id}' cannot be assigned root shell task '{task_id}'")]
    RootShellForbiddenForUnrootedNode { task_id: String, node_id: String },

    /// Traces to: [ANTI-003]
    #[error("Task '{0}' requires unmetered Wi-Fi and cannot be dispatched over cellular WAN")]
    UnmeteredNetworkRequired(String),

    /// Traces to: [ANTI-003], [INV-005]
    #[error("Task '{task_id}' duration of {duration_seconds}s exceeds cellular spot limit of {max_allowed}s")]
    CellularDurationExceeded {
        task_id: String,
        duration_seconds: u32,
        max_allowed: u32,
    },

    #[error("Invalid state transition for task '{task_id}' from {from} to {to}")]
    InvalidStateTransition {
        task_id: String,
        from: String,
        to: String,
    },

    #[error("Node '{node_id}' has insufficient battery ({battery_pct}%) and is not charging")]
    InsufficientBattery { node_id: String, battery_pct: u32 },

    /// Traces to: [NFR-POW-001], [REQ-OP-003]
    #[error("Node '{node_id}' is thermally throttled (battery temp: {temperature_celsius}C, thermal status: {thermal_status})")]
    ThermalThrottled {
        node_id: String,
        temperature_celsius: i32,
        thermal_status: u32,
    },

    /// Traces to: [INV-006]
    #[error("Execution of root shell tasks is disabled on this cluster without --allow-insecure-root-tasks")]
    RootShellExecutionDisabled,
}
