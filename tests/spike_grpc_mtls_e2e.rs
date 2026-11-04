//! End-to-End Live Integration Spike: Tonic gRPC over Mutual TLS 1.3
//!
//! Spawns a live loopback Tonic server enforcing Rustls mTLS 1.3, executes all 4 canonical RPCs,
//! tests attempt fencing over wire gRPC, and verifies rejection of untrusted certificates.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, Section 4.1, Section 5.1, Section 6.3,
//! [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-001], [INV-002], [INV-004], [INV-005], [NFR-SEC-001]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};

use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::{
    build_client_mtls_config, build_server_mtls_config, AndroGridServiceImpl, ClusterScheduler,
    ExecutionEngine, NetworkTransportType, NodeCapabilities, RegisterNodeRequest,
    SubmitCellularResultRequest, SyncNodeStateRequest, TaskSpec,
};

/// Helper to generate in-memory X.509 v3 root CA, server cert, and client cert using rcgen.
struct TestPki {
    pub ca_cert_pem: Vec<u8>,
    pub server_cert_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_cert_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
    pub rogue_cert_pem: Vec<u8>,
    pub rogue_key_pem: Vec<u8>,
}

impl TestPki {
    pub fn generate() -> Self {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

        // 1. Root CA
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Test Root CA");
        let ca_key = KeyPair::generate().expect("Failed to generate CA key");
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("Failed to self-sign CA cert");

        // 2. Server Certificate signed by Root CA
        let mut server_params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()])
                .expect("Failed to create server params");
        server_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        let server_key = KeyPair::generate().expect("Failed to generate server key");
        let server_cert = server_params
            .signed_by(&server_key, &ca_cert, &ca_key)
            .expect("Failed to sign server cert");

        // 3. Client Certificate signed by Root CA (Valid Worker)
        let mut client_params = CertificateParams::default();
        client_params
            .distinguished_name
            .push(DnType::CommonName, "worker-node-pixel8");
        let client_key = KeyPair::generate().expect("Failed to generate client key");
        let client_cert = client_params
            .signed_by(&client_key, &ca_cert, &ca_key)
            .expect("Failed to sign client cert");

        // 4. Rogue Certificate signed by a Rogue CA (Untrusted Worker)
        let mut rogue_ca_params = CertificateParams::default();
        rogue_ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        rogue_ca_params
            .distinguished_name
            .push(DnType::CommonName, "Rogue Untrusted CA");
        let rogue_ca_key = KeyPair::generate().expect("Failed to generate rogue CA key");
        let rogue_ca_cert = rogue_ca_params
            .self_signed(&rogue_ca_key)
            .expect("Failed to self-sign rogue CA cert");

        let mut rogue_client_params = CertificateParams::default();
        rogue_client_params
            .distinguished_name
            .push(DnType::CommonName, "rogue-worker");
        let rogue_key = KeyPair::generate().expect("Failed to generate rogue client key");
        let rogue_cert = rogue_client_params
            .signed_by(&rogue_key, &rogue_ca_cert, &rogue_ca_key)
            .expect("Failed to sign rogue client cert");

        Self {
            ca_cert_pem: ca_cert.pem().into_bytes(),
            server_cert_pem: server_cert.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            client_cert_pem: client_cert.pem().into_bytes(),
            client_key_pem: client_key.serialize_pem().into_bytes(),
            rogue_cert_pem: rogue_cert.pem().into_bytes(),
            rogue_key_pem: rogue_key.serialize_pem().into_bytes(),
        }
    }
}

/// Helper to spin up a live Tonic mTLS server on an ephemeral loopback port.
async fn spawn_live_mtls_server(
    scheduler: Arc<ClusterScheduler>,
    pki: &TestPki,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral loopback port");
    let addr = listener.local_addr().unwrap();

    let server_tls =
        build_server_mtls_config(&pki.server_cert_pem, &pki.server_key_pem, &pki.ca_cert_pem);

    let service = AndroGridServiceImpl::new(scheduler);
    let server = Server::builder()
        .tls_config(server_tls)
        .expect("Failed to configure server TLS")
        .add_service(AndroGridServiceServer::new(service));

    let handle = tokio::spawn(async move {
        let stream = TcpListenerStream::new(listener);
        let res: Result<(), tonic::transport::Error> = server.serve_with_incoming(stream).await;
        let _ = res;
    });

    // Give server a moment to begin listening
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, handle)
}

/// Helper to connect a live Tonic mTLS client.
async fn connect_live_mtls_client(
    addr: SocketAddr,
    client_cert_pem: &[u8],
    client_key_pem: &[u8],
    server_ca_pem: &[u8],
) -> Result<AndroGridServiceClient<Channel>, tonic::transport::Error> {
    let client_tls =
        build_client_mtls_config(client_cert_pem, client_key_pem, server_ca_pem, "localhost");

    let endpoint = Channel::from_shared(format!("https://{}", addr))
        .unwrap()
        .tls_config(client_tls)?;

    let channel = endpoint.connect().await?;
    Ok(AndroGridServiceClient::new(channel))
}

#[tokio::test]
async fn spike_live_grpc_mtls_full_lifecycle_and_monotonic_fencing() {
    let pki = TestPki::generate();
    let scheduler = Arc::new(ClusterScheduler::new());

    // Spawn live mTLS server
    let (addr, _server_handle) = spawn_live_mtls_server(Arc::clone(&scheduler), &pki).await;

    // Connect valid mTLS client
    let mut client = connect_live_mtls_client(
        addr,
        &pki.client_cert_pem,
        &pki.client_key_pem,
        &pki.ca_cert_pem,
    )
    .await
    .expect("Valid mTLS client must connect successfully");

    // 1. Test RegisterNode RPC over live mTLS [REQ-OP-001]
    let reg_req = RegisterNodeRequest {
        node_id: "pixel8-worker-01".to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        }),
        client_version: "1.0.0".to_string(),
    };

    let reg_res = client
        .register_node(reg_req)
        .await
        .expect("RegisterNode must succeed over live mTLS")
        .into_inner();

    assert!(!reg_res.session_token.is_empty());
    assert_eq!(reg_res.heartbeat_interval_seconds, 2);

    // 2. Submit a pending task to the scheduler
    let task_spec = TaskSpec {
        task_id: "live-mtls-task-501".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 120,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
        shell_command: String::new(),
        artifact_sha256: "sha256-test".to_string(),
        requires_unmetered: false,
    };
    scheduler.submit_task(task_spec).unwrap();

    // 3. Test ClaimCellularTask RPC over live mTLS [REQ-OP-003]
    let claim_req = androgrid_backend::ClaimCellularTaskRequest {
        node_id: "pixel8-worker-01".to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        }),
        battery_level_pct: 90,
        is_charging: false,
    };

    let claim_res = client
        .claim_cellular_task(claim_req)
        .await
        .expect("ClaimCellularTask must succeed over live mTLS")
        .into_inner();

    assert!(claim_res.task_available);
    let leased_task = claim_res.task.expect("Task must be leased");
    assert_eq!(leased_task.task_id, "live-mtls-task-501");
    assert_eq!(leased_task.run_attempt, 1);

    // 4. Test SyncNodeState RPC for Wi-Fi Preemption over live mTLS [REQ-OP-002], [INV-005]
    let wifi_reg = scheduler
        .register_node(
            "wifi-node-desk".to_string(),
            NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 16,
                total_ram_bytes: 16 * 1024 * 1024 * 1024,
                os_version: "14".to_string(),
                abi: "x86_64".to_string(),
                is_metered: false,
                current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
                max_cellular_task_duration_seconds: 180,
            },
            "1.0.0".to_string(),
            1000,
        )
        .unwrap();

    let sync_req = SyncNodeStateRequest {
        node_id: "wifi-node-desk".to_string(),
        session_token: wifi_reg.session_token,
        active_tasks: Vec::new(),
        completed_tasks: Vec::new(),
    };

    let sync_res = client
        .sync_node_state(sync_req)
        .await
        .expect("SyncNodeState must succeed over live mTLS")
        .into_inner();

    // Must have speculatively dual-assigned task-501 to Wi-Fi node with run_attempt = 2
    assert_eq!(sync_res.desired_tasks.len(), 1);
    assert_eq!(sync_res.desired_tasks[0].task_id, "live-mtls-task-501");
    assert_eq!(sync_res.desired_tasks[0].run_attempt, 2);

    // 5. Wi-Fi worker submits result with run_attempt = 2 [REQ-OP-004]
    let wifi_submit_req = SubmitCellularResultRequest {
        node_id: "wifi-node-desk".to_string(),
        task_id: "live-mtls-task-501".to_string(),
        run_attempt: 2,
        exit_code: 0,
        stdout_bytes: b"Wi-Fi execution succeeded".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 1200,
    };

    let wifi_submit_res = client
        .submit_cellular_result(wifi_submit_req)
        .await
        .expect("SubmitCellularResult must succeed for active attempt")
        .into_inner();

    assert!(wifi_submit_res.accepted);

    // 6. Cellular worker subsequently submits result with stale run_attempt = 1 [INV-004]
    // Server MUST reject with FAILED_PRECONDITION (9) containing TASK_ATTEMPT_STALE
    let stale_submit_req = SubmitCellularResultRequest {
        node_id: "pixel8-worker-01".to_string(),
        task_id: "live-mtls-task-501".to_string(),
        run_attempt: 1, // Stale attempt 1
        exit_code: 0,
        stdout_bytes: b"Cellular execution finished late".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 45000,
    };

    let stale_err = client
        .submit_cellular_result(stale_submit_req)
        .await
        .expect_err("Stale attempt submission must be rejected with gRPC error");

    assert_eq!(stale_err.code(), tonic::Code::FailedPrecondition);
    assert!(
        stale_err.message().contains("TASK_ATTEMPT_STALE"),
        "Error message must contain TASK_ATTEMPT_STALE per Section 6.3"
    );

    println!(
        "Live mTLS E2E Spike: Verified all 4 RPCs and TASK_ATTEMPT_STALE over live TLS 1.3 mTLS."
    );
}

#[tokio::test]
async fn spike_live_grpc_mtls_rejects_untrusted_client_certificate() {
    let pki = TestPki::generate();
    let scheduler = Arc::new(ClusterScheduler::new());

    // Spawn live mTLS server requiring client certificates signed by pki.ca_cert_pem
    let (addr, _server_handle) = spawn_live_mtls_server(scheduler, &pki).await;

    // Connect with rogue client cert (signed by an untrusted rogue CA)
    let rogue_client_res = connect_live_mtls_client(
        addr,
        &pki.rogue_cert_pem,
        &pki.rogue_key_pem,
        &pki.ca_cert_pem,
    )
    .await;

    // Either connection handshake fails immediately or first RPC is rejected by TLS layer
    match rogue_client_res {
        Ok(mut client) => {
            let res = client
                .register_node(RegisterNodeRequest {
                    node_id: "rogue-node".to_string(),
                    capabilities: None,
                    client_version: "1.0.0".to_string(),
                })
                .await;
            assert!(
                res.is_err(),
                "Server MUST reject RPC when client presents untrusted certificate"
            );
        }
        Err(err) => {
            // Handshake rejection at TCP / TLS level confirms mutual TLS enforcement
            println!("Handshake rejected as expected: {}", err);
        }
    }

    println!("Live mTLS Security Spike: Untrusted rogue client successfully rejected by TLS 1.3.");
}
