//! Mutual TLS 1.3 configuration helpers for Tonic servers and clients.
//!
//! Enforces strict bidirectional cryptographic identity verification.
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001]

use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};
use tonic::{Request, Status};

/// Constructs a ServerTlsConfig that enforces TLS 1.3 mutual authentication (mTLS).
///
/// Traces to: [INV-002], [NFR-SEC-001]
///
/// # Arguments
/// * `server_cert_pem` - PEM-encoded server X.509 certificate.
/// * `server_key_pem` - PEM-encoded server private key.
/// * `client_ca_pem` - PEM-encoded root CA certificate used to verify connecting clients.
pub fn build_server_mtls_config(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    client_ca_pem: &[u8],
) -> ServerTlsConfig {
    let identity = Identity::from_pem(server_cert_pem, server_key_pem);
    let client_ca = Certificate::from_pem(client_ca_pem);

    ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca)
}

/// Constructs a ServerTlsConfig for a shared control-plane + bootstrap enrollment listener.
///
/// Enables `client_auth_optional(true)` so unprovisioned worker nodes can establish a
/// Root-CA-pinned TLS 1.3 session to `EnrollmentService` without presenting a client
/// certificate, while validated client certificates are still verified against `client_ca_pem`.
/// Operational services hosted on the same listener MUST enforce client certificates via
/// [`require_client_mtls_interceptor`].
///
/// Traces to: [REQ-OP-001], [INV-002], [NFR-SEC-001]
pub fn build_server_bootstrap_tls_config(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    client_ca_pem: &[u8],
) -> ServerTlsConfig {
    let identity = Identity::from_pem(server_cert_pem, server_key_pem);
    let client_ca = Certificate::from_pem(client_ca_pem);

    ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca)
        .client_auth_optional(true)
}

/// Tonic request interceptor enforcing that a peer presented a verified client mTLS certificate.
///
/// Used on operational (`AndroGridService`) and administrative (`AdminService`) endpoints when
/// the shared listener runs with [`build_server_bootstrap_tls_config`].
///
/// Traces to: [INV-002], [NFR-SEC-001]
pub fn require_client_mtls_interceptor(request: Request<()>) -> Result<Request<()>, Status> {
    match request.peer_certs() {
        Some(certs) if !certs.is_empty() => Ok(request),
        _ => Err(Status::unauthenticated(
            "Client mTLS certificate required [INV-002]",
        )),
    }
}

/// Constructs a ClientTlsConfig for worker nodes connecting via mTLS.
///
/// Traces to: [INV-002], [NFR-SEC-001]
///
/// # Arguments
/// * `client_cert_pem` - PEM-encoded client X.509 certificate.
/// * `client_key_pem` - PEM-encoded client private key.
/// * `server_ca_pem` - PEM-encoded root CA certificate to verify the orchestrator.
/// * `domain_name` - Expected SAN domain or IP for TLS validation.
pub fn build_client_mtls_config(
    client_cert_pem: &[u8],
    client_key_pem: &[u8],
    server_ca_pem: &[u8],
    domain_name: impl Into<String>,
) -> ClientTlsConfig {
    let identity = Identity::from_pem(client_cert_pem, client_key_pem);
    let server_ca = Certificate::from_pem(server_ca_pem);

    ClientTlsConfig::new()
        .domain_name(domain_name)
        .identity(identity)
        .ca_certificate(server_ca)
}
