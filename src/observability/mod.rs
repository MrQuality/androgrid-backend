//! Embedded Cluster Observability, Web Dashboard, and Terminal TUI Subsystem
//!
//! Provides point-in-time state snapshots, Server-Sent Events (SSE) broadcasting,
//! embedded single-page web dashboard, and interactive terminal monitoring.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

pub mod events;
pub use events::*;

pub mod snapshot;
pub use snapshot::*;

pub mod dashboard;
pub use dashboard::*;

pub mod tui;
pub use tui::*;
