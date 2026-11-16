//! Integration & Policy Tests: In-Band mTLS Certificate Renewal (`RenewCertificate`)
//!
//! Verifies end-to-end Tonic TLS 1.3 mTLS renewal, reconnection with replacement
//! certificates, authorization boundaries, P-256 & key-rotation enforcement,
//! CA lifetime checks, single-flight coalescing, bounded reissuance, and rate limits.
//!
//! Traces to: AG-SPEC-2026-V2 Section 4.1, Section 5.1, Section 6.3,
//! [INV-002], [NFR-SEC-001], [REQ-OP-005]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};
use tonic::Code;

use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::artifact::store::ArtifactStore;
use androgrid_backend::transport::tls::AuthenticatedPeer;
use androgrid_backend::{
    build_client_mtls_config, build_server_mtls_config, require_client_mtls_interceptor,
    AndroGridServiceImpl, CaSigner, CertRenewalManager, CertRenewalPolicy, ClusterScheduler,
    ExecutionEngine, NetworkTransportType, NodeCapabilities, RegisterNodeRequest,
    RenewCertificateRequest, HEADER_NEXT_RENEWAL_EPOCH, HEADER_RETRY_AFTER_EPOCH,
};

struct RenewalTestPki {
    ca_cert_pem: String,
    ca_key_pem: String,
    server_cert_pem: String,
    server_key_pem: String,
    ca_signer: Arc<CaSigner>,
}

impl RenewalTestPki {
    fn generate_with_ca_days(ca_days: i64) -> Self {
        let now = OffsetDateTime::now_utc();
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.not_before = now - Duration::minutes(5);
        ca_params.not_after = now + Duration::days(ca_days);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Renewal Test CA");

        let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let ca_cert_pem = ca_cert.pem();
        let ca_key_pem = ca_key.serialize_pem();

        let mut server_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        server_params
            .distinguished_name
            .push(DnType::CommonName, "androgrid-orchestrator");
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let server_cert = server_params
            .signed_by(&server_key, &ca_cert, &ca_key)
            .unwrap();

        let ca_signer = Arc::new(CaSigner::from_pem(&ca_cert_pem, &ca_key_pem).unwrap());

        Self {
            ca_cert_pem,
            ca_key_pem,
            server_cert_pem: server_cert.pem(),
            server_key_pem: server_key.serialize_pem(),
            ca_signer,
        }
    }

    fn issue_cert(&self, cn: &str, ou: &str, days_valid: i64) -> (String, String, KeyPair) {
        let now = OffsetDateTime::now_utc();
        let ca_params = CertificateParams::from_ca_cert_pem(&self.ca_cert_pem).unwrap();
        let ca_key = KeyPair::from_pem(&self.ca_key_pem).unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, cn);
        dn.push(DnType::OrganizationName, "AndroGrid");
        dn.push(DnType::OrganizationalUnitName, ou);
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.not_before = now - Duration::minutes(5);
        params.not_after = now + Duration::days(days_valid);

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.signed_by(&key, &ca_cert, &ca_key).unwrap();
        (cert.pem(), key.serialize_pem(), key)
    }
}

fn build_p256_csr(cn: &str) -> (String, KeyPair) {
    let mut params = CertificateParams::default();
    params.distinguished_name.push(DnType::CommonName, cn);
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let csr = params.serialize_request(&key).unwrap();
    (csr.pem().unwrap(), key)
}

async fn spawn_mtls_server(
    pki: &RenewalTestPki,
    renewal_manager: Arc<CertRenewalManager>,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = TcpListenerStream::new(listener);

    let scheduler = Arc::new(ClusterScheduler::new());
    let artifact_store = Arc::new(ArtifactStore::new());
    let service = AndroGridServiceImpl::with_services(scheduler, artifact_store, renewal_manager);

    let tls = build_server_mtls_config(
        pki.server_cert_pem.as_bytes(),
        pki.server_key_pem.as_bytes(),
        pki.ca_cert_pem.as_bytes(),
    );

    let intercepted = tonic::codegen::InterceptedService::new(
        AndroGridServiceServer::new(service),
        require_client_mtls_interceptor,
    );

    tokio::spawn(async move {
        Server::builder()
            .tls_config(tls)
            .unwrap()
            .add_service(intercepted)
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    addr
}

async fn connect_mtls_client(
    addr: SocketAddr,
    ca_pem: &str,
    client_cert_pem: &str,
    client_key_pem: &str,
) -> AndroGridServiceClient<Channel> {
    let tls = build_client_mtls_config(
        client_cert_pem.as_bytes(),
        client_key_pem.as_bytes(),
        ca_pem.as_bytes(),
        "localhost",
    );
    let endpoint = Channel::from_shared(format!("https://{addr}"))
        .unwrap()
        .tls_config(tls)
        .unwrap();
    let channel = endpoint.connect().await.unwrap();
    AndroGridServiceClient::new(channel)
}

#[tokio::test]
async fn test_e2e_mtls_certificate_renewal_and_reconnect() {
    let pki = RenewalTestPki::generate_with_ca_days(3650);
    let (worker_cert, worker_key, _) = pki.issue_cert("worker-renew-01", "Edge Compute", 10);

    let manager = Arc::new(CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    ));
    let addr = spawn_mtls_server(&pki, manager).await;

    let mut client = connect_mtls_client(addr, &pki.ca_cert_pem, &worker_cert, &worker_key).await;

    let (csr_pem, new_key) = build_p256_csr("worker-renew-01");
    let resp = client
        .renew_certificate(RenewCertificateRequest { csr_pem })
        .await
        .expect("Renewal inside 30-day window must succeed")
        .into_inner();

    assert!(resp.client_cert_pem.contains("BEGIN CERTIFICATE"));
    assert_eq!(resp.ca_cert_pem.trim(), pki.ca_cert_pem.trim());
    let now_u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(resp.not_after_epoch_seconds > now_u64 + 300 * 86_400);

    // Reconnect over a fresh TLS 1.3 mTLS channel using the renewed certificate + new keypair
    let mut renewed_client = connect_mtls_client(
        addr,
        &pki.ca_cert_pem,
        &resp.client_cert_pem,
        &new_key.serialize_pem(),
    )
    .await;

    let reg = renewed_client
        .register_node(RegisterNodeRequest {
            node_id: "worker-renew-01".to_string(),
            capabilities: Some(NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 4,
                total_ram_bytes: 4_000_000_000,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                is_metered: false,
                current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
                max_cellular_task_duration_seconds: 180,
                battery_temperature_celsius: 25,
                thermal_status: 0,
            }),
            client_version: "0.1.0".to_string(),
            labels: Default::default(),
        })
        .await
        .expect("Renewed certificate must authenticate operational RPCs")
        .into_inner();
    assert!(!reg.session_token.is_empty());
}

#[tokio::test]
async fn test_e2e_rejects_control_plane_ou_and_outside_window_and_bad_csrs() {
    let pki = RenewalTestPki::generate_with_ca_days(3650);
    let manager = Arc::new(CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    ));
    let addr = spawn_mtls_server(&pki, manager).await;

    // 1. Operator cert (OU=Control Plane) must be denied
    let (admin_cert, admin_key, _) = pki.issue_cert("ops-admin", "Control Plane", 10);
    let mut admin_client =
        connect_mtls_client(addr, &pki.ca_cert_pem, &admin_cert, &admin_key).await;
    let (admin_csr, _) = build_p256_csr("ops-admin");
    let err = admin_client
        .renew_certificate(RenewCertificateRequest { csr_pem: admin_csr })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    // 2. Worker cert outside 30-day renewal window (90 days left) -> FailedPrecondition + header
    let (fresh_cert, fresh_key, _) = pki.issue_cert("worker-fresh", "Edge Compute", 90);
    let mut fresh_client =
        connect_mtls_client(addr, &pki.ca_cert_pem, &fresh_cert, &fresh_key).await;
    let (fresh_csr, _) = build_p256_csr("worker-fresh");
    let err = fresh_client
        .renew_certificate(RenewCertificateRequest { csr_pem: fresh_csr })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(err.metadata().get(HEADER_NEXT_RENEWAL_EPOCH).is_some());

    // 3. Worker cert inside window: reject P-384 CSR, same-key CSR, and CN mismatch
    let (win_cert, win_key, win_kp) = pki.issue_cert("worker-win", "Edge Compute", 10);
    let mut win_client = connect_mtls_client(addr, &pki.ca_cert_pem, &win_cert, &win_key).await;

    let mut p384_params = CertificateParams::default();
    p384_params
        .distinguished_name
        .push(DnType::CommonName, "worker-win");
    let p384_key = KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384).unwrap();
    let p384_csr = p384_params
        .serialize_request(&p384_key)
        .unwrap()
        .pem()
        .unwrap();
    let err = win_client
        .renew_certificate(RenewCertificateRequest { csr_pem: p384_csr })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // Same-key CSR
    let mut same_params = CertificateParams::default();
    same_params
        .distinguished_name
        .push(DnType::CommonName, "worker-win");
    let same_csr = same_params
        .serialize_request(&win_kp)
        .unwrap()
        .pem()
        .unwrap();
    let err = win_client
        .renew_certificate(RenewCertificateRequest { csr_pem: same_csr })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // CN spoofing CSR
    let (spoof_csr, _) = build_p256_csr("other-worker");
    let err = win_client
        .renew_certificate(RenewCertificateRequest { csr_pem: spoof_csr })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn test_renewal_manager_policy_edge_cases() {
    let pki = RenewalTestPki::generate_with_ca_days(3650);
    let (worker_cert, _, worker_kp) = pki.issue_cert("worker-policy", "Edge Compute", 10);
    let leaf_der = pem::parse(worker_cert).unwrap().into_contents();
    let (_, parsed_leaf) = x509_parser::parse_x509_certificate(&leaf_der).unwrap();
    let leaf_fp: [u8; 32] = Sha256::digest(&leaf_der).into();
    let _ = worker_kp;

    let valid_peer = AuthenticatedPeer {
        common_name: Some("worker-policy".to_string()),
        organizational_unit: Some("Edge Compute".to_string()),
        not_before_epoch_secs: parsed_leaf.validity().not_before.timestamp(),
        not_after_epoch_secs: parsed_leaf.validity().not_after.timestamp(),
        leaf_fingerprint_sha256: leaf_fp,
        spki_der: parsed_leaf.public_key().raw.to_vec(),
    };

    // 1. Missing AuthenticatedPeer (non-mTLS) -> Unauthenticated
    let mgr = CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    );
    let (csr1, _) = build_p256_csr("worker-policy");
    let err = mgr.renew_certificate(None, &csr1).await.unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);

    // 2. Expired leaf cert on already-open HTTP/2 session -> Unauthenticated
    let mut expired_peer = valid_peer.clone();
    expired_peer.not_after_epoch_secs = valid_peer.not_before_epoch_secs + 1;
    let err = mgr
        .renew_certificate(Some(&expired_peer), &csr1)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);

    // 3. Missing CaSigner -> FailedPrecondition
    let no_signer_mgr = CertRenewalManager::new(None, CertRenewalPolicy::default());
    let err = no_signer_mgr
        .renew_certificate(Some(&valid_peer), &csr1)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);

    // 4. Disabled renewal -> FailedPrecondition
    let disabled_mgr = CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy {
            allow_cert_renewal: false,
            ..CertRenewalPolicy::default()
        },
    );
    let err = disabled_mgr
        .renew_certificate(Some(&valid_peer), &csr1)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);

    // 5. CA lifetime insufficient (CA expires in 20d < 30+7d) -> FailedPrecondition (never clamped)
    let short_ca_pki = RenewalTestPki::generate_with_ca_days(20);
    let short_ca_mgr = CertRenewalManager::new(
        Some(Arc::clone(&short_ca_pki.ca_signer)),
        CertRenewalPolicy::default(),
    );
    let err = short_ca_mgr
        .renew_certificate(Some(&valid_peer), &csr1)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(err.message().contains("CA_LIFETIME_INSUFFICIENT"));

    // 6. Concurrent identical requests coalesced + cache hit re-checks renewal denial
    let shared_mgr = Arc::new(CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    ));
    let (r1, r2) = tokio::join!(
        shared_mgr.renew_certificate(Some(&valid_peer), &csr1),
        shared_mgr.renew_certificate(Some(&valid_peer), &csr1),
    );
    let res1 = r1.unwrap();
    let res2 = r2.unwrap();
    assert_eq!(res1.client_cert_pem, res2.client_cert_pem);

    // Denylist blocks even cached hits
    shared_mgr.deny_node("worker-policy");
    let denied_err = shared_mgr
        .renew_certificate(Some(&valid_peer), &csr1)
        .await
        .unwrap_err();
    assert_eq!(denied_err.code(), Code::PermissionDenied);

    // 7. Rolling burst rate limit (3 distinct CSRs allowed, 4th returns ResourceExhausted)
    let rl_mgr = CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    );
    for _ in 0..3 {
        let (csr, _) = build_p256_csr("worker-policy");
        rl_mgr
            .renew_certificate(Some(&valid_peer), &csr)
            .await
            .unwrap();
    }
    let (csr4, _) = build_p256_csr("worker-policy");
    let rl_err = rl_mgr
        .renew_certificate(Some(&valid_peer), &csr4)
        .await
        .unwrap_err();
    assert_eq!(rl_err.code(), Code::ResourceExhausted);
    assert!(rl_err.metadata().get(HEADER_RETRY_AFTER_EPOCH).is_some());
}

#[tokio::test]
async fn test_single_flight_failure_cancellation_fingerprint_denial_and_config_bounds() {
    let pki = RenewalTestPki::generate_with_ca_days(3650);
    let (worker_cert, _, _) = pki.issue_cert("worker-sf", "Edge Compute", 10);
    let leaf_der = pem::parse(worker_cert).unwrap().into_contents();
    let (_, parsed_leaf) = x509_parser::parse_x509_certificate(&leaf_der).unwrap();
    let leaf_fp: [u8; 32] = Sha256::digest(&leaf_der).into();

    let valid_peer = AuthenticatedPeer {
        common_name: Some("worker-sf".to_string()),
        organizational_unit: Some("Edge Compute".to_string()),
        not_before_epoch_secs: parsed_leaf.validity().not_before.timestamp(),
        not_after_epoch_secs: parsed_leaf.validity().not_after.timestamp(),
        leaf_fingerprint_sha256: leaf_fp,
        spki_der: parsed_leaf.public_key().raw.to_vec(),
    };

    // 1. Single-flight leader failure propagates directly without re-executing
    let mgr = Arc::new(CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    ));
    let (bad_csr, _) = build_p256_csr("wrong-cn");
    let (r1, r2) = tokio::join!(
        mgr.renew_certificate(Some(&valid_peer), &bad_csr),
        mgr.renew_certificate(Some(&valid_peer), &bad_csr),
    );
    assert_eq!(r1.unwrap_err().code(), Code::InvalidArgument);
    assert_eq!(r2.unwrap_err().code(), Code::InvalidArgument);

    // 2. Cancelled leader cleans up in_flight guard so subsequent request succeeds
    let (good_csr, _) = build_p256_csr("worker-sf");
    let mgr_clone = Arc::clone(&mgr);
    let peer_clone = valid_peer.clone();
    let csr_clone = good_csr.clone();
    let handle = tokio::spawn(async move {
        mgr_clone
            .renew_certificate(Some(&peer_clone), &csr_clone)
            .await
    });
    handle.abort();
    let _ = handle.await;
    let after_abort = mgr.renew_certificate(Some(&valid_peer), &good_csr).await;
    assert!(after_abort.is_ok());

    // 3. deny_fingerprint blocks renewal even for a cached CSR
    let fp_mgr = CertRenewalManager::new(
        Some(Arc::clone(&pki.ca_signer)),
        CertRenewalPolicy::default(),
    );
    let (csr_fp, _) = build_p256_csr("worker-sf");
    fp_mgr
        .renew_certificate(Some(&valid_peer), &csr_fp)
        .await
        .unwrap();
    fp_mgr.deny_fingerprint(leaf_fp);
    let fp_err = fp_mgr
        .renew_certificate(Some(&valid_peer), &csr_fp)
        .await
        .unwrap_err();
    assert_eq!(fp_err.code(), Code::PermissionDenied);

    // 4. ClusterConfig renewal policy bounds validation
    let cfg_zero = androgrid_backend::ClusterConfig {
        cert_renewal_validity_days: 0,
        ..Default::default()
    };
    assert!(cfg_zero.validate_renewal_policy().is_err());

    let cfg_window = androgrid_backend::ClusterConfig {
        cert_renewal_validity_days: 30,
        cert_renewal_window_days: 30,
        ..Default::default()
    };
    assert!(cfg_window.validate_renewal_policy().is_err());
}
