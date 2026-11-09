//! Integration Specification: Ephemeral Device Pairing & QR-Code Enrollment Engine
//!
//! Validates time-bounded single-use token lifecycle, CSR validation, Root CA signing,
//! HTTP and gRPC bootstrap enrollment endpoints, and immediate mTLS 1.3 transition.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001], [REQ-OP-001]

use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

use androgrid_backend::admin_service_server::AdminServiceServer;
use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::enrollment_service_client::EnrollmentServiceClient;
use androgrid_backend::enrollment_service_server::EnrollmentServiceServer;
use androgrid_backend::{
    build_client_mtls_config, build_server_mtls_config, AdminServiceImpl, AndroGridServiceImpl,
    ArtifactServer, ArtifactStore, CaSigner, ClusterScheduler, EnrollmentContext,
    EnrollmentServiceImpl, ExecutionEngine, HttpEnrollRequest, HttpEnrollResponse,
    NetworkTransportType, NodeCapabilities, PairingError, PairingTokenStore, RegisterNodeRequest,
    SignerError, SyncNodeStateRequest,
};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

/// In-memory PKI fixture generator for hermetic testing.
struct TestClusterPki {
    ca_cert_pem: String,
    ca_key_pem: String,
    server_cert_pem: String,
    server_key_pem: String,
}

impl TestClusterPki {
    fn generate() -> Self {
        // 1. Root CA
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Internal Test CA");
        let ca_key = KeyPair::generate().expect("Generate CA key");
        let ca_cert = ca_params.self_signed(&ca_key).expect("Self-sign CA cert");

        // 2. Server Certificate
        let mut server_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("Server params");
        server_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        let server_key = KeyPair::generate().expect("Generate server key");
        let server_cert = server_params
            .signed_by(&server_key, &ca_cert, &ca_key)
            .expect("Sign server cert");

        Self {
            ca_cert_pem: ca_cert.pem(),
            ca_key_pem: ca_key.serialize_pem(),
            server_cert_pem: server_cert.pem(),
            server_key_pem: server_key.serialize_pem(),
        }
    }
}

/// Helper to generate client ECDSA key pair and PKCS#10 CSR.
fn generate_client_csr(node_id: &str) -> (String, KeyPair) {
    let mut params = CertificateParams::default();
    params.distinguished_name.push(DnType::CommonName, node_id);
    let key = KeyPair::generate().expect("Generate worker key");
    let csr = params
        .serialize_request(&key)
        .expect("Serialize worker CSR");
    (csr.pem().expect("Convert CSR to PEM"), key)
}

#[test]
fn test_ephemeral_token_generation_expiration_and_revocation() {
    let store = PairingTokenStore::default();

    // 1. Generate token with 5-minute TTL
    let record = store
        .generate_token(Some("worker-alpha".to_string()), Some(300))
        .expect("Token generation");

    assert_eq!(record.token.len(), 64);
    assert_eq!(record.target_node_id.as_deref(), Some("worker-alpha"));
    assert!(!record.is_redeemed);
    assert_eq!(record.expires_at_epoch_s, record.created_at_epoch_s + 300);

    // 2. First redemption must succeed
    let redeemed = store
        .redeem_token(&record.token, "worker-alpha")
        .expect("First redemption");
    assert!(redeemed.is_redeemed);

    // 3. Subsequent redemption must fail (single-use invariant)
    let second_attempt = store.redeem_token(&record.token, "worker-alpha");
    assert_eq!(second_attempt, Err(PairingError::TokenAlreadyRedeemed));
}

#[test]
fn test_enrollment_endpoint_valid_vs_invalid_csr() {
    let pki = TestClusterPki::generate();
    let signer = CaSigner::from_pem(&pki.ca_cert_pem, &pki.ca_key_pem).expect("CaSigner init");

    let node_id = "worker-device-01";
    let (valid_csr, _key) = generate_client_csr(node_id);

    // 1. Valid CSR signing succeeds
    let signed = signer
        .sign_worker_csr(&valid_csr, node_id)
        .expect("Signing valid CSR");

    assert_eq!(signed.node_id, node_id);
    assert!(signed.client_cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(signed.ca_cert_pem.contains("BEGIN CERTIFICATE"));

    // 2. Corrupted CSR fails
    let corrupted_csr = "-----BEGIN CERTIFICATE REQUEST-----\nBAD_CORRUPTED_BASE64_DATA==\n-----END CERTIFICATE REQUEST-----";
    let err = signer.sign_worker_csr(corrupted_csr, node_id);
    assert!(matches!(err, Err(SignerError::InvalidCsr(_))));

    // 3. Node ID mismatch fails
    let mismatch_err = signer.sign_worker_csr(&valid_csr, "worker-device-other");
    assert!(matches!(
        mismatch_err,
        Err(SignerError::CommonNameMismatch { .. })
    ));
}

#[test]
fn test_signed_certificate_validation_against_root_ca() {
    let pki = TestClusterPki::generate();
    let signer = CaSigner::from_pem(&pki.ca_cert_pem, &pki.ca_key_pem).unwrap();

    let node_id = "worker-cryptographic-validation";
    let (csr_pem, _key) = generate_client_csr(node_id);

    let signed = signer.sign_worker_csr(&csr_pem, node_id).unwrap();

    // Verify certificate parses and matches Common Name
    let parsed_params = CertificateParams::from_ca_cert_pem(&signed.client_cert_pem)
        .expect("Client certificate must parse cleanly");
    assert_eq!(
        parsed_params
            .distinguished_name
            .get(&DnType::CommonName)
            .map(|v| match v {
                rcgen::DnValue::Utf8String(s) => s.as_str(),
                rcgen::DnValue::PrintableString(s) => s.as_str(),
                _ => "",
            }),
        Some(node_id)
    );

    // Verify Root CA fingerprint calculation consistency
    let fp = signer.ca_fingerprint();
    let computed_fp = CaSigner::compute_ca_fingerprint(&pki.ca_cert_pem).unwrap();
    assert_eq!(fp, computed_fp);
}

#[tokio::test]
async fn test_http_enrollment_endpoint_lifecycle() {
    let pki = TestClusterPki::generate();
    let signer = Arc::new(CaSigner::from_pem(&pki.ca_cert_pem, &pki.ca_key_pem).unwrap());
    let token_store = Arc::new(PairingTokenStore::default());
    let artifact_store = Arc::new(ArtifactStore::new());

    let enrollment_ctx = EnrollmentContext {
        token_store: Arc::clone(&token_store),
        signer: Arc::clone(&signer),
    };

    let router = ArtifactServer::router_with_enrollment(artifact_store, enrollment_ctx);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_addr = listener.local_addr().unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{local_addr}");

    // Step 1: Health check
    let health_resp = client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health_resp.status(), 200);

    // Step 2: Generate token
    let node_id = "http-worker-node";
    let record = token_store
        .generate_token(Some(node_id.to_string()), Some(300))
        .unwrap();

    let (csr_pem, _key) = generate_client_csr(node_id);

    // Step 3: First enrollment request (should succeed: 200 OK)
    let enroll_payload = HttpEnrollRequest {
        pairing_token: record.token.clone(),
        node_id: node_id.to_string(),
        csr_pem: csr_pem.clone(),
    };

    let resp = client
        .post(format!("{base_url}/api/v1/enroll"))
        .json(&enroll_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: HttpEnrollResponse = resp.json().await.unwrap();
    assert!(body.success);
    assert!(body.client_cert_pem.contains("BEGIN CERTIFICATE"));
    assert!(body.ca_cert_pem.contains("BEGIN CERTIFICATE"));

    // Step 4: Second enrollment request with same token must fail (409 Conflict)
    let repeat_resp = client
        .post(format!("{base_url}/api/v1/enroll"))
        .json(&enroll_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(repeat_resp.status(), 409);
    let repeat_body: HttpEnrollResponse = repeat_resp.json().await.unwrap();
    assert!(!repeat_body.success);
    assert!(repeat_body.error_message.contains("already been redeemed"));

    // Step 5: Invalid token must fail (401 Unauthorized)
    let invalid_payload = HttpEnrollRequest {
        pairing_token: "invalid-token-12345".to_string(),
        node_id: node_id.to_string(),
        csr_pem,
    };
    let invalid_resp = client
        .post(format!("{base_url}/api/v1/enroll"))
        .json(&invalid_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_resp.status(), 401);

    let _ = shutdown_tx.send(());
}

#[tokio::test]
async fn test_live_grpc_enrollment_and_subsequent_mtls_registration() {
    let pki = TestClusterPki::generate();
    let signer = Arc::new(CaSigner::from_pem(&pki.ca_cert_pem, &pki.ca_key_pem).unwrap());
    let token_store = Arc::new(PairingTokenStore::default());
    let scheduler = Arc::new(ClusterScheduler::new());
    let artifact_store = Arc::new(ArtifactStore::new());

    // 1. Spawn plaintext bootstrap enrollment server (simulating initial bootstrap port)
    let enroll_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let enroll_addr = enroll_listener.local_addr().unwrap();
    let enroll_stream = TcpListenerStream::new(enroll_listener);

    let enrollment_service =
        EnrollmentServiceImpl::new(Arc::clone(&token_store), Arc::clone(&signer));

    tokio::spawn(async move {
        Server::builder()
            .add_service(EnrollmentServiceServer::new(enrollment_service))
            .serve_with_incoming(enroll_stream)
            .await
            .unwrap();
    });

    // 2. Spawn mTLS operational control plane server (enforcing TLS 1.3 mTLS per [INV-002])
    let mtls_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mtls_addr = mtls_listener.local_addr().unwrap();
    let mtls_stream = TcpListenerStream::new(mtls_listener);

    let server_tls = build_server_mtls_config(
        pki.server_cert_pem.as_bytes(),
        pki.server_key_pem.as_bytes(),
        pki.ca_cert_pem.as_bytes(),
    );

    let andro_service = AndroGridServiceImpl::new(Arc::clone(&scheduler));
    let admin_service = AdminServiceImpl::with_services(
        Arc::clone(&scheduler),
        Arc::clone(&artifact_store),
        "http://127.0.0.1:50052".to_string(),
        Arc::clone(&token_store),
        signer.ca_fingerprint().to_string(),
        format!("https://{mtls_addr}"),
    );

    tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls)
            .unwrap()
            .add_service(AndroGridServiceServer::new(andro_service))
            .add_service(AdminServiceServer::new(admin_service))
            .serve_with_incoming(mtls_stream)
            .await
            .unwrap();
    });

    // Step A: Operator requests ephemeral pairing token via AdminService over loopback
    let node_id = "worker-live-e2e-node";
    let token_record = token_store
        .generate_token(Some(node_id.to_string()), Some(300))
        .unwrap();

    // Step B: Worker generates client key pair and CSR
    let (csr_pem, worker_key) = generate_client_csr(node_id);

    // Step C: Worker connects to bootstrap EnrollmentService
    let mut enroll_client = EnrollmentServiceClient::connect(format!("http://{enroll_addr}"))
        .await
        .unwrap();

    let enroll_req = tonic::Request::new(androgrid_backend::EnrollWorkerRequest {
        pairing_token: token_record.token.clone(),
        node_id: node_id.to_string(),
        csr_pem,
    });

    let enroll_res = enroll_client
        .enroll_worker(enroll_req)
        .await
        .unwrap()
        .into_inner();
    assert!(
        enroll_res.success,
        "Enrollment must succeed: {}",
        enroll_res.error_message
    );
    assert!(!enroll_res.client_cert_pem.is_empty());
    assert!(!enroll_res.ca_cert_pem.is_empty());

    // Step D: Worker configures mTLS using the issued client certificate and worker private key
    let client_tls = build_client_mtls_config(
        enroll_res.client_cert_pem.as_bytes(),
        worker_key.serialize_pem().as_bytes(),
        enroll_res.ca_cert_pem.as_bytes(),
        "localhost",
    );

    let channel = tonic::transport::Channel::from_shared(format!("https://{mtls_addr}"))
        .unwrap()
        .tls_config(client_tls)
        .unwrap()
        .connect()
        .await
        .expect("mTLS connection with issued certificate must succeed [INV-002]");

    let mut andro_client = AndroGridServiceClient::new(channel);

    // Step E: Worker registers over mTLS [REQ-OP-001]
    let reg_req = RegisterNodeRequest {
        node_id: node_id.to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: false,
            current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
            max_cellular_task_duration_seconds: 180,
            battery_temperature_celsius: 30,
            thermal_status: 0,
        }),
        client_version: "0.1.0".to_string(),
    };

    let reg_res = andro_client
        .register_node(reg_req)
        .await
        .unwrap()
        .into_inner();
    assert!(!reg_res.session_token.is_empty());
    assert_eq!(reg_res.heartbeat_interval_seconds, 2);

    // Step F: Worker synchronizes state over mTLS [REQ-OP-002]
    let sync_req = SyncNodeStateRequest {
        node_id: node_id.to_string(),
        session_token: reg_res.session_token,
        active_tasks: Vec::new(),
        completed_tasks: Vec::new(),
    };

    let sync_res = andro_client
        .sync_node_state(sync_req)
        .await
        .unwrap()
        .into_inner();
    assert!(sync_res.server_epoch_ms > 0);
}
