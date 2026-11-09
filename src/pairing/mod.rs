//! Ephemeral Device Pairing and Bootstrap QR-Code Enrollment Subsystem
//!
//! Provides time-bounded single-use pairing tokens, ANSI UTF-8 QR code generation,
//! Root CA certificate signing for unprovisioned Android worker nodes, and
//! bootstrap enrollment endpoints over gRPC and HTTP.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

pub mod qr;
pub mod service;
pub mod signer;
pub mod token;

pub use qr::*;
pub use service::*;
pub use signer::*;
pub use token::*;
