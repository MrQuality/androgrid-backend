//! End-to-End Integration Specification: Operator Management and Cluster Observability
//!
//! Spawns an in-memory loopback orchestrator hosting both AndroGridService and AdminService
//! over Mutual TLS 1.3, verifies operator task ingestion, worker execution lifecycle,
//! real-time node inventory querying, and task cancellation.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-003], [REQ-OP-004], [INV-002], [NFR-SEC-001]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};

use androgrid_backend::admin_service_client::AdminServiceClient;
use androgrid_backend::admin_service_server::AdminServiceServer;
use androgrid_backend::andro_grid_service_client::AndroGridServiceClient;
use androgrid_backend::andro_grid_service_server::AndroGridServiceServer;
use androgrid_backend::{
    build_client_mtls_config, build_server_mtls_config, AdminServiceImpl, AndroGridServiceImpl,
    CancelTaskRequest, ClaimCellularTaskRequest, ClusterScheduler, ExecutionEngine,
    GetTaskStatusRequest, ListWorkerNodesRequest, NetworkTransportType, NodeCapabilities,
    RegisterNodeRequest, SubmitCellularResultRequest, SubmitTaskRequest, TaskSpec, TaskState,
};

struct TestPki {
    pub ca_cert_pem: Vec<u8>,
    pub server_cert_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_cert_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
}

impl TestPki {
    pub fn generate() -> Self {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid Admin Test CA");
        let ca_key = KeyPair::generate().expect("Failed to generate CA key");
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("Failed to self-sign CA cert");

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

        let mut client_params =
            CertificateParams::new(vec!["operator.androgrid.internal".to_string()])
                .expect("Failed to create client params");
        client_params
            .distinguished_name
            .push(DnType::CommonName, "operator");
        let client_key = KeyPair::generate().expect("Failed to generate client key");
        let client_cert = client_params
            .signed_by(&client_key, &ca_cert, &ca_key)
            .expect("Failed to sign client cert");

        Self {
            ca_cert_pem: ca_cert.pem().into_bytes(),
            server_cert_pem: server_cert.pem().into_bytes(),
            server_key_pem: server_key.serialize_pem().into_bytes(),
            client_cert_pem: client_cert.pem().into_bytes(),
            client_key_pem: client_key.serialize_pem().into_bytes(),
        }
    }
}

async fn spawn_test_server(
    scheduler: Arc<ClusterScheduler>,
    pki: &TestPki,
) -> (SocketAddr, tokio::sync::oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral loopback port");
    let local_addr = listener.local_addr().expect("Failed to get local address");

    let server_tls =
        build_server_mtls_config(&pki.server_cert_pem, &pki.server_key_pem, &pki.ca_cert_pem);

    let worker_service = AndroGridServiceImpl::new(Arc::clone(&scheduler));
    let worker_server = AndroGridServiceServer::new(worker_service);

    let admin_service = AdminServiceImpl::new(Arc::clone(&scheduler));
    let admin_server = AdminServiceServer::new(admin_service);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        Server::builder()
            .tls_config(server_tls)
            .expect("Valid TLS configuration")
            .add_service(worker_server)
            .add_service(admin_server)
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("Server failed");
    });

    (local_addr, shutdown_tx)
}

fn build_mtls_channel(addr: SocketAddr, pki: &TestPki) -> Channel {
    let tls_config = build_client_mtls_config(
        &pki.client_cert_pem,
        &pki.client_key_pem,
        &pki.ca_cert_pem,
        "localhost",
    );

    let endpoint_uri = format!("https://{}", addr);
    let endpoint = tonic::transport::Endpoint::from_shared(endpoint_uri)
        .expect("Valid URI")
        .tls_config(tls_config)
        .expect("Valid client TLS");

    endpoint.connect_lazy()
}

#[tokio::test]
async fn test_operator_end_to_end_lifecycle_and_observability() {
    let pki = TestPki::generate();
    let scheduler = Arc::new(ClusterScheduler::new());
    let (addr, shutdown_tx) = spawn_test_server(Arc::clone(&scheduler), &pki).await;

    let channel = build_mtls_channel(addr, &pki);
    let mut admin_client = AdminServiceClient::new(channel.clone());
    let mut worker_client = AndroGridServiceClient::new(channel);

    // 1. Initial State: No nodes registered
    let nodes_initial = admin_client
        .list_worker_nodes(ListWorkerNodesRequest {})
        .await
        .expect("List nodes should succeed")
        .into_inner();
    assert_eq!(nodes_initial.nodes.len(), 0);

    // 2. Operator Submits WASM Compute Task
    let task_wasm = TaskSpec {
        task_id: "task_operator_compute_01".to_string(),
        run_attempt: 1,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 60,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        shell_command: String::new(),
        artifact_sha256: "b45a...".to_string(),
        requires_unmetered: false,
    };

    let submit_res = admin_client
        .submit_task(SubmitTaskRequest {
            task: Some(task_wasm),
        })
        .await
        .expect("Task submission should succeed")
        .into_inner();
    assert!(submit_res.accepted);
    assert_eq!(submit_res.task_id, "task_operator_compute_01");

    // 3. Operator Queries Task Status (PENDING)
    let status_initial = admin_client
        .get_task_status(GetTaskStatusRequest {
            task_id: "task_operator_compute_01".to_string(),
        })
        .await
        .expect("Get status should succeed")
        .into_inner();
    assert_eq!(status_initial.state, TaskState::Pending as i32);
    assert_eq!(status_initial.active_attempt, 0);

    // 4. Android Worker Node Registers via Wi-Fi Loop
    let worker_caps = NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 8,
        total_ram_bytes: 6 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: true,
        current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
        max_cellular_task_duration_seconds: 180,
    };

    let reg_res = worker_client
        .register_node(RegisterNodeRequest {
            node_id: "android_node_pixel8".to_string(),
            capabilities: Some(worker_caps.clone()),
            client_version: "0.1.0".to_string(),
        })
        .await
        .expect("Worker registration should succeed")
        .into_inner();
    assert!(!reg_res.session_token.is_empty());

    // 5. Operator Queries Worker Inventory
    let nodes_updated = admin_client
        .list_worker_nodes(ListWorkerNodesRequest {})
        .await
        .expect("List nodes should succeed")
        .into_inner();
    assert_eq!(nodes_updated.nodes.len(), 1);
    assert_eq!(nodes_updated.nodes[0].node_id, "android_node_pixel8");
    let reported_caps = nodes_updated.nodes[0].capabilities.as_ref().unwrap();
    assert_eq!(reported_caps.cpu_cores, 8);
    assert_eq!(reported_caps.abi, "arm64-v8a");

    // 6. Worker Claims Task over Cellular WAN
    let claim_res = worker_client
        .claim_cellular_task(ClaimCellularTaskRequest {
            node_id: "android_node_pixel8".to_string(),
            capabilities: Some(worker_caps),
            battery_level_pct: 80,
            is_charging: false,
        })
        .await
        .expect("Claim cellular task should succeed")
        .into_inner();
    assert!(claim_res.task_available);
    let claimed_task = claim_res.task.expect("Task spec must be present");
    assert_eq!(claimed_task.task_id, "task_operator_compute_01");
    assert_eq!(claimed_task.run_attempt, 1);

    // 7. Worker Submits Execution Result
    let result_req = SubmitCellularResultRequest {
        node_id: "android_node_pixel8".to_string(),
        task_id: "task_operator_compute_01".to_string(),
        run_attempt: 1,
        exit_code: 0,
        stdout_bytes: b"matrix multiplication result 42".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 125,
    };

    let submit_result_res = worker_client
        .submit_cellular_result(result_req)
        .await
        .expect("Result submission should succeed")
        .into_inner();
    assert!(submit_result_res.accepted);

    // 8. Operator Verifies Task is COMPLETED with Execution Artifacts
    let status_completed = admin_client
        .get_task_status(GetTaskStatusRequest {
            task_id: "task_operator_compute_01".to_string(),
        })
        .await
        .expect("Status query must succeed")
        .into_inner();
    assert_eq!(status_completed.state, TaskState::Completed as i32);
    assert_eq!(status_completed.exit_code, 0);
    assert_eq!(
        status_completed.stdout_bytes,
        b"matrix multiplication result 42"
    );
    assert_eq!(status_completed.execution_duration_ms, 125);

    // 9. Operator Submits Second Task and Cancels It
    let task_cancel = TaskSpec {
        task_id: "task_operator_cancel_02".to_string(),
        run_attempt: 1,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 60,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d],
        shell_command: String::new(),
        artifact_sha256: "c12d...".to_string(),
        requires_unmetered: false,
    };

    admin_client
        .submit_task(SubmitTaskRequest {
            task: Some(task_cancel),
        })
        .await
        .unwrap();

    let cancel_res = admin_client
        .cancel_task(CancelTaskRequest {
            task_id: "task_operator_cancel_02".to_string(),
        })
        .await
        .expect("Cancel task should succeed")
        .into_inner();
    assert!(cancel_res.cancelled);

    // 10. Operator Verifies Cancelled Task State is FAILED
    let status_cancelled = admin_client
        .get_task_status(GetTaskStatusRequest {
            task_id: "task_operator_cancel_02".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status_cancelled.state, TaskState::Failed as i32);
    assert_eq!(status_cancelled.error_code, "TASK_CANCELLED");

    let _ = shutdown_tx.send(());
}
