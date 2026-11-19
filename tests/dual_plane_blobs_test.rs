//! Integration tests verifying dual-plane TLS 1.3 mTLS (`50051`) gRPC + `/v1/blobs` REST
//! RBAC, streaming > 2 MiB uploads, RAII `.tmp` cleanup, single-range `206`/`416`, and `50052`
//! plaintext artifact stripping.
//!
//! Traces to: [INV-002], [NFR-PERF-001], [NFR-SEC-001], [REQ-OP-003], ADR-005, ADR-008

use std::collections::HashMap;
use std::sync::Arc;

use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::artifact::server::ArtifactServer;
use androgrid_backend::artifact::store::ArtifactStore;
use androgrid_backend::enrollment_service_client::EnrollmentServiceClient;
use androgrid_backend::enrollment_service_server::EnrollmentServiceServer;
use androgrid_backend::pairing::signer::CaSigner;
use androgrid_backend::transport::tls::{
    build_bootstrap_rustls_server_config, require_client_mtls_interceptor, serve_dual_plane_tls,
};
use androgrid_backend::{
    build_client_mtls_config, AndroGridServiceImpl, ClusterScheduler, EnrollWorkerRequest,
    EnrollmentServiceImpl, NodeCapabilities, PairingTokenStore, RegisterNodeRequest,
};
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair};
use tokio::net::TcpListener;
use tonic::service::Routes;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use tower::ServiceExt;

struct TestPki {
    ca_pem: String,
    ca_key_pem: String,
    srv_cert_pem: String,
    srv_key_pem: String,
    worker_cert_pem: String,
    worker_key_pem: String,
    admin_cert_pem: String,
    admin_key_pem: String,
}

fn generate_test_pki() -> TestPki {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "AndroGrid Test CA");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let srv_key = KeyPair::generate().unwrap();
    let mut srv_params =
        CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
    srv_params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    let srv_cert = srv_params.signed_by(&srv_key, &ca_cert, &ca_key).unwrap();

    let worker_key = KeyPair::generate().unwrap();
    let mut worker_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    worker_params
        .distinguished_name
        .push(DnType::CommonName, "worker-blob-01");
    worker_params
        .distinguished_name
        .push(DnType::OrganizationalUnitName, "Edge Compute");
    worker_params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    let worker_cert = worker_params
        .signed_by(&worker_key, &ca_cert, &ca_key)
        .unwrap();

    let admin_key = KeyPair::generate().unwrap();
    let mut admin_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    admin_params
        .distinguished_name
        .push(DnType::CommonName, "ops-admin");
    admin_params
        .distinguished_name
        .push(DnType::OrganizationalUnitName, "Control Plane");
    admin_params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    let admin_cert = admin_params
        .signed_by(&admin_key, &ca_cert, &ca_key)
        .unwrap();

    TestPki {
        ca_pem: ca_cert.pem(),
        ca_key_pem: ca_key.serialize_pem(),
        srv_cert_pem: srv_cert.pem(),
        srv_key_pem: srv_key.serialize_pem(),
        worker_cert_pem: worker_cert.pem(),
        worker_key_pem: worker_key.serialize_pem(),
        admin_cert_pem: admin_cert.pem(),
        admin_key_pem: admin_key.serialize_pem(),
    }
}

fn build_reqwest_mtls(ca_pem: &str, cert_pem: &str, key_pem: &str) -> reqwest::Client {
    let mut combined = cert_pem.to_string();
    combined.push_str(key_pem);
    let id = reqwest::Identity::from_pem(combined.as_bytes()).unwrap();
    let ca = reqwest::Certificate::from_pem(ca_pem.as_bytes()).unwrap();
    reqwest::Client::builder()
        .add_root_certificate(ca)
        .identity(id)
        .use_rustls_tls()
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_dual_plane_50051_rbac_streaming_range_and_50052_hardening() {
    let pki = generate_test_pki();
    let temp_dir = std::env::temp_dir().join(format!(
        "androgrid_dual_plane_blobs_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);

    let store = Arc::new(ArtifactStore::with_directory(&temp_dir));
    let scheduler = Arc::new(ClusterScheduler::new());
    let token_store = Arc::new(PairingTokenStore::new(true, 3));
    let signer = Arc::new(CaSigner::from_pem(&pki.ca_pem, &pki.ca_key_pem).unwrap());

    let tls_cfg = build_bootstrap_rustls_server_config(
        pki.srv_cert_pem.as_bytes(),
        pki.srv_key_pem.as_bytes(),
        pki.ca_pem.as_bytes(),
    )
    .unwrap();

    let service_server = tonic::codegen::InterceptedService::new(
        AndroGridServiceServer::new(AndroGridServiceImpl::with_artifact_store(
            Arc::clone(&scheduler),
            Arc::clone(&store),
        )),
        require_client_mtls_interceptor,
    );
    let enroll_server =
        EnrollmentServiceServer::new(EnrollmentServiceImpl::new(token_store, signer));

    let grpc_router = Routes::new(service_server)
        .add_service(enroll_server)
        .into_axum_router();
    let blob_router = ArtifactServer::blob_router(Arc::clone(&store), 20 * 1024 * 1024);
    let app_router = grpc_router.merge(blob_router);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        serve_dual_plane_tls(listener, tls_cfg, app_router, async {
            let _ = shutdown_rx.await;
        })
        .await;
    });

    let uri = format!("https://127.0.0.1:{}", addr.port());
    let unauth_ca = reqwest::Certificate::from_pem(pki.ca_pem.as_bytes()).unwrap();
    let unauth_http = reqwest::Client::builder()
        .add_root_certificate(unauth_ca)
        .use_rustls_tls()
        .build()
        .unwrap();
    let worker_http = build_reqwest_mtls(&pki.ca_pem, &pki.worker_cert_pem, &pki.worker_key_pem);
    let admin_http = build_reqwest_mtls(&pki.ca_pem, &pki.admin_cert_pem, &pki.admin_key_pem);

    // 1. Unauthenticated /v1/blobs -> 401
    let r_unauth = unauth_http
        .put(format!("{uri}/v1/blobs"))
        .body(b"abc".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r_unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Worker (OU=Edge Compute) PUT /v1/blobs -> 403 Forbidden
    let r_worker_put = worker_http
        .put(format!("{uri}/v1/blobs"))
        .body(b"\x00asm\x01\x00\x00\x00".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r_worker_put.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Admin (OU=Control Plane) PUT > 2 MiB WASM -> 201 Created, idempotent -> 200 OK, kind conflict -> 409
    let mut payload_3m = vec![0x42u8; 3 * 1024 * 1024];
    payload_3m[..8].copy_from_slice(b"\x00asm\x01\x00\x00\x00");
    let sha_3m = ArtifactStore::compute_sha256(&payload_3m);

    let r_put1 = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "wasm")
        .header("X-Expected-Sha256", sha_3m.to_ascii_uppercase())
        .body(payload_3m.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r_put1.status(), reqwest::StatusCode::CREATED);

    let r_put2 = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "wasm")
        .body(payload_3m.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r_put2.status(), reqwest::StatusCode::OK);

    let r_bad_magic = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "apk")
        .body(payload_3m.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(
        r_bad_magic.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    // 2b. APK > 16 MB end-to-end PUT + GET (Content-Type: application/vnd.android.package-archive)
    let mut apk_17m = vec![0x5Au8; 17 * 1024 * 1024];
    apk_17m[..4].copy_from_slice(b"PK\x03\x04");
    let sha_apk = ArtifactStore::compute_sha256(&apk_17m);
    let r_apk_put = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "apk")
        .header("X-Expected-Sha256", sha_apk.clone())
        .body(apk_17m.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r_apk_put.status(), reqwest::StatusCode::CREATED);

    let r_apk_get = worker_http
        .get(format!("{uri}/v1/blobs/{sha_apk}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r_apk_get.status(), reqwest::StatusCode::OK);
    assert_eq!(
        r_apk_get
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/vnd.android.package-archive"
    );
    assert_eq!(r_apk_get.bytes().await.unwrap().len(), apk_17m.len());

    // 2c. Mid-stream abort cleans up .tmp files via TempFileGuard Drop
    let failing_stream = tokio_stream::iter(vec![
        Ok::<Vec<u8>, std::io::Error>(b"\x00asm\x01\x00\x00\x00partial".to_vec()),
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "simulated client disconnect",
        )),
    ]);
    let _ = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "wasm")
        .body(reqwest::Body::wrap_stream(failing_stream))
        .send()
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let leftover_tmps: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
        .collect();
    assert!(
        leftover_tmps.is_empty(),
        "TempFileGuard must clean up .tmp files on mid-stream abort"
    );

    // 4. Bad X-Expected-Sha256 digest -> 422, oversized > 16 MiB WASM -> 413 with 0 leftover .tmp
    let r_bad_sha = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Expected-Sha256", "0".repeat(64))
        .body(b"\x00asm\x01\x00\x00\x00".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(
        r_bad_sha.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    let mut oversized_wasm = vec![0x99u8; 16 * 1024 * 1024 + 1];
    oversized_wasm[..8].copy_from_slice(b"\x00asm\x01\x00\x00\x00");
    let r_oversized = admin_http
        .put(format!("{uri}/v1/blobs"))
        .header("X-Artifact-Kind", "wasm")
        .body(oversized_wasm)
        .send()
        .await
        .unwrap();
    assert_eq!(r_oversized.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);

    let leftover_tmp: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("tmp"))
        .collect();
    assert!(leftover_tmp.is_empty());

    // 5. Worker GET full + single Range -> 206 + multi-range -> 416 + HEAD 416
    let r_get = worker_http
        .get(format!("{uri}/v1/blobs/{sha_3m}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r_get.status(), reqwest::StatusCode::OK);
    assert_eq!(
        r_get
            .headers()
            .get("x-artifact-size")
            .unwrap()
            .to_str()
            .unwrap(),
        payload_3m.len().to_string()
    );
    assert_eq!(
        r_get
            .headers()
            .get("x-artifact-sha256")
            .unwrap()
            .to_str()
            .unwrap(),
        sha_3m
    );
    assert_eq!(r_get.bytes().await.unwrap().len(), payload_3m.len());

    let r_range = worker_http
        .get(format!("{uri}/v1/blobs/{sha_3m}"))
        .header("Range", "bytes=100-199")
        .send()
        .await
        .unwrap();
    assert_eq!(r_range.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(r_range.bytes().await.unwrap().len(), 100);

    let r_multi = worker_http
        .head(format!("{uri}/v1/blobs/{sha_3m}"))
        .header("Range", "bytes=0-1,2-3")
        .send()
        .await
        .unwrap();
    assert_eq!(r_multi.status(), reqwest::StatusCode::RANGE_NOT_SATISFIABLE);

    // 6. gRPC on same 50051 listener: unauth reaches EnrollmentService, rejects AndroGridService; mTLS passes
    let unauth_grpc_ch = Channel::from_shared(uri.clone())
        .unwrap()
        .tls_config(
            ClientTlsConfig::new()
                .domain_name("localhost")
                .ca_certificate(Certificate::from_pem(pki.ca_pem.as_bytes())),
        )
        .unwrap()
        .connect()
        .await
        .unwrap();
    let enroll_res = EnrollmentServiceClient::new(unauth_grpc_ch.clone())
        .enroll_worker(EnrollWorkerRequest {
            pairing_token: "bad".to_string(),
            node_id: "w1".to_string(),
            csr_pem: String::new(),
        })
        .await;
    if let Err(s) = enroll_res {
        assert_ne!(s.code(), tonic::Code::Unimplemented);
        assert_ne!(s.code(), tonic::Code::Unauthenticated);
    }
    let grid_unauth = AndroGridServiceClient::new(unauth_grpc_ch)
        .register_node(RegisterNodeRequest {
            node_id: "w1".to_string(),
            capabilities: Some(NodeCapabilities::default()),
            client_version: "0.1.0".to_string(),
            labels: HashMap::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(grid_unauth.code(), tonic::Code::Unauthenticated);

    let worker_grpc_ch = Channel::from_shared(uri)
        .unwrap()
        .tls_config(build_client_mtls_config(
            pki.worker_cert_pem.as_bytes(),
            pki.worker_key_pem.as_bytes(),
            pki.ca_pem.as_bytes(),
            "localhost",
        ))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let reg_ok = AndroGridServiceClient::new(worker_grpc_ch)
        .register_node(RegisterNodeRequest {
            node_id: "worker-blob-01".to_string(),
            capabilities: Some(NodeCapabilities::default()),
            client_version: "0.1.0".to_string(),
            labels: HashMap::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!reg_ok.session_token.is_empty());

    // 7. Auxiliary 50052 router with expose_plaintext_artifacts=false strips /artifacts/* while keeping /health
    let aux_router = ArtifactServer::router_full_with_options(
        Arc::clone(&store),
        Arc::clone(&scheduler),
        None,
        false,
    );
    let aux_health = aux_router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(aux_health.status(), axum::http::StatusCode::OK);
    let health_bytes = axum::body::to_bytes(aux_health.into_body(), 64 * 1024)
        .await
        .unwrap();
    let health_json: serde_json::Value = serde_json::from_slice(&health_bytes).unwrap();
    assert!(
        health_json["cached_artifacts"].as_u64().unwrap_or(0) >= 2,
        "/health must reflect disk-backed .blob artifacts"
    );
    assert!(store.has_artifact(&sha_3m));
    assert_eq!(store.get_artifact_size(&sha_3m), Some(payload_3m.len()));
    let aux_artifact = aux_router
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/artifacts/{sha_3m}.wasm"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(aux_artifact.status(), axum::http::StatusCode::NOT_FOUND);

    let _ = shutdown_tx.send(());
    let _ = std::fs::remove_dir_all(&temp_dir);
}
