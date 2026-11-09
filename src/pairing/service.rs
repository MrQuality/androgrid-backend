//! Orchestrator Bootstrap Enrollment Service (gRPC & HTTP)
//!
//! Provides the single non-mTLS bootstrap exchange where unprovisioned worker
//! nodes redeem an ephemeral pairing token and PKCS#10 CSR to receive their mTLS credentials.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tonic::{Request, Response as TonicResponse, Status};
use tracing::{error, info, warn};

use crate::pairing::signer::CaSigner;
use crate::pairing::token::{PairingError, PairingTokenStore};
use crate::proto::androgrid::v1::enrollment_service_server::EnrollmentService;
use crate::proto::androgrid::v1::{EnrollWorkerRequest, EnrollWorkerResponse};

/// HTTP JSON request body for device bootstrap enrollment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpEnrollRequest {
    /// Single-use ephemeral pairing token.
    pub pairing_token: String,

    /// Mobile worker node identifier.
    pub node_id: String,

    /// PEM-encoded PKCS#10 Certificate Signing Request.
    pub csr_pem: String,
}

/// HTTP JSON response body for device bootstrap enrollment.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HttpEnrollResponse {
    /// Whether enrollment and certificate issuance succeeded.
    pub success: bool,

    /// PEM-encoded signed client certificate (if successful).
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub client_cert_pem: String,

    /// PEM-encoded Root CA certificate for pinning (if successful).
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub ca_cert_pem: String,

    /// Error description if enrollment failed.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub error_message: String,
}

/// Shared application state for bootstrap enrollment services.
#[derive(Clone)]
pub struct EnrollmentContext {
    pub token_store: Arc<PairingTokenStore>,
    pub signer: Arc<CaSigner>,
}

/// Tonic gRPC implementation of the EnrollmentService.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
pub struct EnrollmentServiceImpl {
    context: EnrollmentContext,
}

impl EnrollmentServiceImpl {
    /// Constructs a new EnrollmentServiceImpl with the provided context.
    pub fn new(token_store: Arc<PairingTokenStore>, signer: Arc<CaSigner>) -> Self {
        Self {
            context: EnrollmentContext {
                token_store,
                signer,
            },
        }
    }
}

#[tonic::async_trait]
impl EnrollmentService for EnrollmentServiceImpl {
    async fn enroll_worker(
        &self,
        request: Request<EnrollWorkerRequest>,
    ) -> Result<TonicResponse<EnrollWorkerResponse>, Status> {
        let req = request.into_inner();
        let token = req.pairing_token.trim();
        let node_id = req.node_id.trim();
        let csr_pem = req.csr_pem.trim();

        if token.is_empty() {
            return Ok(TonicResponse::new(EnrollWorkerResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: "Pairing token cannot be empty".to_string(),
            }));
        }

        if node_id.is_empty() {
            return Ok(TonicResponse::new(EnrollWorkerResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: "Node identifier cannot be empty".to_string(),
            }));
        }

        if csr_pem.is_empty() {
            return Ok(TonicResponse::new(EnrollWorkerResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: "CSR PEM cannot be empty".to_string(),
            }));
        }

        info!(node_id = %node_id, "Processing gRPC worker enrollment request [REQ-OP-001]");

        // Step 1: Validate and redeem pairing token (enforcing single-use semantics)
        if let Err(e) = self.context.token_store.redeem_token(token, node_id) {
            warn!(node_id = %node_id, error = %e, "Enrollment token redemption rejected");
            return Ok(TonicResponse::new(EnrollWorkerResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: e.to_string(),
            }));
        }

        // Step 2: Validate CSR and sign client certificate with Root CA
        match self.context.signer.sign_worker_csr(csr_pem, node_id) {
            Ok(signed) => {
                info!(node_id = %node_id, "Successfully issued mTLS client certificate [INV-002]");
                Ok(TonicResponse::new(EnrollWorkerResponse {
                    success: true,
                    client_cert_pem: signed.client_cert_pem,
                    ca_cert_pem: signed.ca_cert_pem,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                error!(node_id = %node_id, error = %e, "Failed to sign worker CSR");
                Ok(TonicResponse::new(EnrollWorkerResponse {
                    success: false,
                    client_cert_pem: String::new(),
                    ca_cert_pem: String::new(),
                    error_message: e.to_string(),
                }))
            }
        }
    }
}

/// Builds an Axum Router exposing the HTTP enrollment endpoint.
///
/// Traces to: [INV-002], [NFR-SEC-001], [REQ-OP-001]
pub fn enrollment_http_router(context: EnrollmentContext) -> Router {
    Router::new()
        .route("/api/v1/enroll", post(handle_http_enroll))
        .route("/api/v1/pairing/enroll", post(handle_http_enroll))
        .with_state(context)
}

/// Axum HTTP handler processing bootstrap enrollment submissions.
async fn handle_http_enroll(
    State(context): State<EnrollmentContext>,
    Json(payload): Json<HttpEnrollRequest>,
) -> Response {
    let token = payload.pairing_token.trim();
    let node_id = payload.node_id.trim();
    let csr_pem = payload.csr_pem.trim();

    if token.is_empty() || node_id.is_empty() || csr_pem.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(HttpEnrollResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: "pairing_token, node_id, and csr_pem must all be non-empty"
                    .to_string(),
            }),
        )
            .into_response();
    }

    info!(node_id = %node_id, "Processing HTTP worker enrollment request [REQ-OP-001]");

    // Step 1: Validate and redeem pairing token
    if let Err(e) = context.token_store.redeem_token(token, node_id) {
        warn!(node_id = %node_id, error = %e, "HTTP enrollment token redemption rejected");
        let status = match e {
            PairingError::RateLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            PairingError::TokenExpired { .. } => StatusCode::GONE,
            PairingError::TokenAlreadyRedeemed => StatusCode::CONFLICT,
            PairingError::PairingDisabled => StatusCode::FORBIDDEN,
            _ => StatusCode::UNAUTHORIZED,
        };

        return (
            status,
            Json(HttpEnrollResponse {
                success: false,
                client_cert_pem: String::new(),
                ca_cert_pem: String::new(),
                error_message: e.to_string(),
            }),
        )
            .into_response();
    }

    // Step 2: Validate CSR and sign client certificate with Root CA
    match context.signer.sign_worker_csr(csr_pem, node_id) {
        Ok(signed) => {
            info!(node_id = %node_id, "Successfully issued mTLS client certificate via HTTP [INV-002]");
            (
                StatusCode::OK,
                Json(HttpEnrollResponse {
                    success: true,
                    client_cert_pem: signed.client_cert_pem,
                    ca_cert_pem: signed.ca_cert_pem,
                    error_message: String::new(),
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(node_id = %node_id, error = %e, "Failed to sign worker CSR via HTTP");
            (
                StatusCode::BAD_REQUEST,
                Json(HttpEnrollResponse {
                    success: false,
                    client_cert_pem: String::new(),
                    ca_cert_pem: String::new(),
                    error_message: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

    fn create_test_context() -> (EnrollmentContext, KeyPair, String) {
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Test Root CA");
        let ca_key = KeyPair::generate().expect("CA key generation");
        let ca_cert = ca_params.self_signed(&ca_key).expect("Self-sign CA cert");

        let signer = Arc::new(
            CaSigner::from_pem(&ca_cert.pem(), &ca_key.serialize_pem())
                .expect("CaSigner initialization"),
        );
        let token_store = Arc::new(PairingTokenStore::default());

        (
            EnrollmentContext {
                token_store,
                signer,
            },
            ca_key,
            ca_cert.pem(),
        )
    }

    fn generate_test_csr(node_id: &str) -> (String, KeyPair) {
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, node_id);
        let key = KeyPair::generate().expect("Worker key generation");
        let csr = params.serialize_request(&key).expect("CSR serialization");
        (csr.pem().expect("CSR PEM"), key)
    }

    #[tokio::test]
    async fn test_grpc_enrollment_lifecycle_and_single_use_enforcement() {
        let (ctx, _ca_key, _ca_pem) = create_test_context();
        let service =
            EnrollmentServiceImpl::new(Arc::clone(&ctx.token_store), Arc::clone(&ctx.signer));

        let node_id = "test-worker-alpha";
        let record = ctx
            .token_store
            .generate_token(Some(node_id.to_string()), Some(300))
            .expect("Token generation");

        let (csr_pem, _worker_key) = generate_test_csr(node_id);

        let req = tonic::Request::new(EnrollWorkerRequest {
            pairing_token: record.token.clone(),
            node_id: node_id.to_string(),
            csr_pem: csr_pem.clone(),
        });

        // 1. Initial enrollment must succeed
        let resp = service.enroll_worker(req).await.expect("RPC call");
        let body = resp.into_inner();
        assert!(
            body.success,
            "Enrollment must succeed: {}",
            body.error_message
        );
        assert!(!body.client_cert_pem.is_empty());
        assert!(!body.ca_cert_pem.is_empty());

        // 2. Second enrollment with same token must fail (single-use invariant)
        let repeat_req = tonic::Request::new(EnrollWorkerRequest {
            pairing_token: record.token,
            node_id: node_id.to_string(),
            csr_pem,
        });

        let repeat_resp = service.enroll_worker(repeat_req).await.expect("RPC call");
        let repeat_body = repeat_resp.into_inner();
        assert!(!repeat_body.success, "Repeated enrollment must be rejected");
        assert!(repeat_body.error_message.contains("already been redeemed"));
    }

    #[tokio::test]
    async fn test_grpc_enrollment_mismatched_node_id_rejected() {
        let (ctx, _ca_key, _ca_pem) = create_test_context();
        let service =
            EnrollmentServiceImpl::new(Arc::clone(&ctx.token_store), Arc::clone(&ctx.signer));

        let record = ctx
            .token_store
            .generate_token(Some("expected-worker".to_string()), Some(300))
            .expect("Token generation");

        let (csr_pem, _worker_key) = generate_test_csr("impostor-worker");

        let req = tonic::Request::new(EnrollWorkerRequest {
            pairing_token: record.token,
            node_id: "impostor-worker".to_string(),
            csr_pem,
        });

        let resp = service.enroll_worker(req).await.expect("RPC call");
        let body = resp.into_inner();
        assert!(!body.success);
        assert!(body.error_message.contains("mismatch"));
    }
}
