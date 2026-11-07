//! In-memory cluster scheduler, monotonic attempt fencing, and speculative preemption.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.2, Section 3.1, Section 6.2,
//! [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-004], [INV-005], [INV-006], [ANTI-003], [NFR-REL-001]

pub mod engine;
pub mod errors;
pub mod state;
pub mod thermal;

pub use engine::{
    ClusterScheduler, DEFAULT_HEARTBEAT_INTERVAL_SECONDS, DEFAULT_NODE_LEASE_TIMEOUT_SECONDS,
    MAX_CELLULAR_DURATION_SECONDS, MIN_CELLULAR_BATTERY_PCT,
};
pub use errors::SchedulingError;
pub use state::{TaskExecutionResult, TaskRecord, WorkerRecord};
pub use thermal::{ThermalAction, ThermalPolicy};
