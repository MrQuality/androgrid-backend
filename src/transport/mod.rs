//! Network transport layer: Tonic gRPC service and TLS 1.3 mTLS configuration.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, Section 4.1, Section 6.3, [INV-001], [INV-002], [NFR-SEC-001]

pub mod admin_service;
pub mod error;
pub mod service;
pub mod tls;

pub use admin_service::AdminServiceImpl;
pub use service::AndroGridServiceImpl;
pub use tls::{
    build_client_mtls_config, build_server_bootstrap_tls_config, build_server_mtls_config,
    require_client_mtls_interceptor,
};
