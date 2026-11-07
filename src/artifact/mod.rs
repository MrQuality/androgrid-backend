//! WebAssembly Artifact Distribution and Toolchain Subsystem
//!
//! Exposes content-addressed artifact storage, embedded HTTP distribution server,
//! binary WASI validator, and CLI packaging primitives.
//!
//! Traces to: AG-SPEC-2026-V2, [NFR-PERF-001], [NFR-POW-001], [INV-003], [REQ-OP-003]

pub mod packager;
pub mod server;
pub mod store;
pub mod wasi_validator;

pub use packager::*;
pub use server::*;
pub use store::*;
pub use wasi_validator::*;
