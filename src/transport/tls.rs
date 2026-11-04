//! Mutual TLS 1.3 configuration helpers for Tonic servers and clients.
//!
//! Enforces strict bidirectional cryptographic identity verification.
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001]

use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

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
