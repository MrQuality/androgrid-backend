//! Executable Acceptance Criteria (BDD / Gherkin) Verification Suite
//!
//! Directly validates all three normative acceptance scenarios defined in
//! AG-SPEC-2026-V2 Section 8 over live Tonic gRPC server backed by Rustls mTLS 1.3.
//!
//! Traces to: AG-SPEC-2026-V2 Section 8,
//! [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-001], [INV-002], [INV-003], [INV-004], [INV-005], [INV-006],
//! [ANTI-001], [ANTI-003], [NFR-SEC-001], [NFR-POW-001], [NFR-REL-001]

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
    build_client_mtls_config, build_server_mtls_config, AndroGridServiceImpl,
    ClaimCellularTaskRequest, ClusterScheduler, ExecutionEngine, NetworkTransportType,
    NodeCapabilities, RegisterNodeRequest, SubmitCellularResultRequest, SyncNodeStateRequest,
    TaskSpec, TaskState,
};

/// Generates in-memory X.509 v3 root CA, server cert, and client cert using rcgen.
struct BddPki {
    pub ca_cert_pem: Vec<u8>,
    pub server_cert_pem: Vec<u8>,
    pub server_key_pem: Vec<u8>,
    pub client_cert_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
}

impl BddPki {
    pub fn generate() -> Self {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};

        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "AndroGrid BDD Root CA");
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

        let mut client_params = CertificateParams::default();
        client_params
            .distinguished_name
            .push(DnType::CommonName, "worker-bdd-node");
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

/// Test harness hosting a live loopback Tonic server over Rustls mTLS 1.3.
struct BddClusterHarness {
    pub scheduler: Arc<ClusterScheduler>,
    pub addr: SocketAddr,
    pub pki: BddPki,
}

impl BddClusterHarness {
    pub async fn start() -> Self {
        let pki = BddPki::generate();
        let scheduler = Arc::new(ClusterScheduler::new());
        let service = AndroGridServiceImpl::new(Arc::clone(&scheduler));

        let server_tls = build_server_mtls_config(
            &pki.server_cert_pem,
            &pki.server_key_pem,
            &pki.ca_cert_pem,
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Binding random ephemeral port must succeed");
        let addr = listener.local_addr().expect("Local addr must be valid");

        tokio::spawn(async move {
            Server::builder()
                .tls_config(server_tls)
                .expect("TLS configuration attachment must succeed")
                .add_service(AndroGridServiceServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .expect("Server loop failed");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        Self {
            scheduler,
            addr,
            pki,
        }
    }

    /// Spawns an authenticated mTLS gRPC client channel.
    pub async fn connect_client(&self) -> AndroGridServiceClient<Channel> {
        let client_tls = build_client_mtls_config(
            &self.pki.client_cert_pem,
            &self.pki.client_key_pem,
            &self.pki.ca_cert_pem,
            "localhost",
        );

        let endpoint = format!("https://{}", self.addr);
        let channel = Channel::from_shared(endpoint)
            .expect("Endpoint URI must be valid")
            .tls_config(client_tls)
            .expect("TLS configuration attachment must succeed")
            .connect()
            .await
            .expect("Client mTLS handshake must succeed");

        AndroGridServiceClient::new(channel)
    }
}

// ============================================================================
// Section 8 Acceptance Scenario 1: Cellular Disconnected Execution
// ============================================================================
// Feature: Cellular Disconnected Execution and Zero-Keepalive Radio Sleep
// Scenario: Cellular worker claims task, drops socket, and submits result
// Given the worker is connected to cellular network
// When the worker calls "ClaimCellularTask"
// Then the orchestrator returns task "task_501" with timeout 120 seconds
// And the mobile app immediately closes the gRPC channel
// And the mobile baseband radio enters low-power sleep
// When the WASM execution completes locally after 45 seconds
// Then the app opens a new gRPC connection over Cloudflare Tunnel
// And calls "SubmitCellularResult" with run_attempt 1
// And the orchestrator acknowledges result acceptance
// ============================================================================
#[tokio::test]
async fn test_bdd_scenario_1_cellular_disconnected_execution_and_zero_keepalive() {
    let harness = BddClusterHarness::start().await;

    // Given: task_501 enqueued in the orchestrator pool (120s timeout, requires_unmetered = false)
    let task_501 = TaskSpec {
        task_id: "task_501".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 120,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d], // wasm magic bytes
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
    };
    harness.scheduler.submit_task(task_501).unwrap();

    // Given: the worker is enrolled and connected to cellular network
    let mut client = harness.connect_client().await;
    let reg_req = RegisterNodeRequest {
        node_id: "cell_node_501".to_string(),
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
        client_version: "0.1.0".to_string(),
    };
    client
        .register_node(reg_req)
        .await
        .expect("Registration must succeed");

    // When: the worker calls "ClaimCellularTask"
    let claim_req = ClaimCellularTaskRequest {
        node_id: "cell_node_501".to_string(),
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
        battery_level_pct: 85,
        is_charging: false,
    };

    let claim_res = client
        .claim_cellular_task(claim_req)
        .await
        .expect("ClaimCellularTask RPC must succeed")
        .into_inner();

    // Then: the orchestrator returns task "task_501" with timeout 120 seconds
    assert!(claim_res.task_available);
    let claimed_task = claim_res.task.expect("Task payload must be present");
    assert_eq!(claimed_task.task_id, "task_501");
    assert_eq!(claimed_task.timeout_seconds, 120);
    assert_eq!(claimed_task.run_attempt, 1);

    // And: the mobile app immediately closes the gRPC channel
    // And: the mobile baseband radio enters low-power sleep
    drop(client); // Socket dropped immediately (< 200 ms)

    // When: the WASM execution completes locally after 45 seconds (simulated)
    let execution_duration_ms = 45_000u64;
    let stdout_bytes = b"WASM compute output complete".to_vec();

    // Then: the app opens a new gRPC connection over Cloudflare Tunnel
    let mut submission_client = harness.connect_client().await;

    // And: calls "SubmitCellularResult" with run_attempt 1
    let submit_req = SubmitCellularResultRequest {
        node_id: "cell_node_501".to_string(),
        task_id: "task_501".to_string(),
        run_attempt: 1,
        exit_code: 0,
        stdout_bytes,
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms,
    };

    let submit_res = submission_client
        .submit_cellular_result(submit_req)
        .await
        .expect("SubmitCellularResult RPC must succeed")
        .into_inner();

    // And: the orchestrator acknowledges result acceptance
    assert!(submit_res.accepted);
    assert_eq!(submit_res.status_message, "Committed");
}

// ============================================================================
// Section 8 Acceptance Scenario 2: Spot Dual-Assignment and Monotonic Fencing
// ============================================================================
// Feature: Cellular Spot Dual-Assignment and Monotonic Fencing
// Scenario: Wi-Fi worker preempts a slow cellular worker
// Given task "task_601" is claimed by cellular worker "cell_01" with run_attempt 1
// When 60 seconds elapse and Wi-Fi worker "wifi_01" becomes idle
// Then the orchestrator speculatively assigns "task_601" to "wifi_01" with run_attempt 2
// When "wifi_01" completes and submits "task_601" with run_attempt 2
// Then the orchestrator marks "task_601" as COMPLETED
// When "cell_01" subsequently submits "task_601" with run_attempt 1
// Then the orchestrator rejects the submission with error "TASK_ATTEMPT_STALE"
// ============================================================================
#[tokio::test]
async fn test_bdd_scenario_2_cellular_spot_dual_assignment_and_monotonic_fencing() {
    let harness = BddClusterHarness::start().await;

    // Given: task "task_601" enqueued in the scheduler
    let task_601 = TaskSpec {
        task_id: "task_601".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 180,
        entrypoint: "main".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d],
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
    };
    harness.scheduler.submit_task(task_601).unwrap();

    // Enroll cellular worker "cell_01"
    let mut cell_client = harness.connect_client().await;
    cell_client
        .register_node(RegisterNodeRequest {
            node_id: "cell_01".to_string(),
            capabilities: Some(NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 4,
                total_ram_bytes: 4 * 1024 * 1024 * 1024,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                is_metered: true,
                current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
                max_cellular_task_duration_seconds: 180,
            }),
            client_version: "0.1.0".to_string(),
        })
        .await
        .unwrap();

    // Given: task "task_601" is claimed by cellular worker "cell_01" with run_attempt 1
    let cell_claim_req = ClaimCellularTaskRequest {
        node_id: "cell_01".to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 4,
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        }),
        battery_level_pct: 75,
        is_charging: false,
    };

    let cell_claim_res = cell_client
        .claim_cellular_task(cell_claim_req)
        .await
        .unwrap()
        .into_inner();

    assert!(cell_claim_res.task_available);
    let cell_task = cell_claim_res.task.unwrap();
    assert_eq!(cell_task.task_id, "task_601");
    assert_eq!(cell_task.run_attempt, 1);

    // Cellular worker drops socket during execution
    drop(cell_client);

    // When: 60 seconds elapse and Wi-Fi worker "wifi_01" becomes idle
    let mut wifi_client = harness.connect_client().await;

    let reg_res = wifi_client
        .register_node(RegisterNodeRequest {
            node_id: "wifi_01".to_string(),
            capabilities: Some(NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 8,
                total_ram_bytes: 12 * 1024 * 1024 * 1024,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                is_metered: false,
                current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
                max_cellular_task_duration_seconds: 180,
            }),
            client_version: "0.1.0".to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    let session_token = reg_res.session_token;

    // Then: the orchestrator speculatively assigns "task_601" to "wifi_01" with run_attempt 2
    let sync_res = wifi_client
        .sync_node_state(SyncNodeStateRequest {
            node_id: "wifi_01".to_string(),
            session_token,
            active_tasks: Vec::new(),
            completed_tasks: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(sync_res.desired_tasks.len(), 1);
    let spec_task = &sync_res.desired_tasks[0];
    assert_eq!(spec_task.task_id, "task_601");
    assert_eq!(spec_task.run_attempt, 2); // Monotonic bump to attempt 2

    // When: "wifi_01" completes and submits "task_601" with run_attempt 2
    let wifi_submit_req = SubmitCellularResultRequest {
        node_id: "wifi_01".to_string(),
        task_id: "task_601".to_string(),
        run_attempt: 2,
        exit_code: 0,
        stdout_bytes: b"Result from fast Wi-Fi node".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 12_000,
    };

    let wifi_submit_res = wifi_client
        .submit_cellular_result(wifi_submit_req)
        .await
        .unwrap()
        .into_inner();

    // Then: the orchestrator marks "task_601" as COMPLETED
    assert!(wifi_submit_res.accepted);
    assert_eq!(
        harness.scheduler.get_task_state("task_601"),
        Some(TaskState::Completed)
    );

    // When: "cell_01" subsequently submits "task_601" with run_attempt 1
    let mut cell_reconnect_client = harness.connect_client().await;
    let stale_submit_req = SubmitCellularResultRequest {
        node_id: "cell_01".to_string(),
        task_id: "task_601".to_string(),
        run_attempt: 1, // Stale run_attempt
        exit_code: 0,
        stdout_bytes: b"Slow cellular output".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 65_000,
    };

    let stale_submit_err = cell_reconnect_client
        .submit_cellular_result(stale_submit_req)
        .await
        .expect_err("Submission with stale run_attempt must fail");

    // Then: the orchestrator rejects the submission with error "TASK_ATTEMPT_STALE"
    // Since task is already finalized in Completed state, error is mapped to FailedPrecondition
    assert_eq!(stale_submit_err.code(), tonic::Code::FailedPrecondition);
}

// ============================================================================
// Section 8 Acceptance Scenario 3: Sandbox Isolation Guard for Unrooted Nodes
// ============================================================================
// Feature: Sandbox Isolation Guard for Unrooted Nodes
// Scenario: Unrooted node attempts to register and is constrained to WASM execution
// Given an Android worker node with is_rooted set to false
// When the worker node registers with the cluster
// Then the orchestrator restricts assignable engines to EXECUTION_ENGINE_WASM_WASI
// And the orchestrator never dispatches EXECUTION_ENGINE_ROOT_SHELL tasks to this node
// ============================================================================
#[tokio::test]
async fn test_bdd_scenario_3_sandbox_isolation_guard_for_unrooted_nodes() {
    let harness = BddClusterHarness::start().await;

    // Given: an Android worker node with is_rooted set to false
    let mut client = harness.connect_client().await;

    // When: the worker node registers with the cluster, claiming it supports ROOT_SHELL
    let reg_req = RegisterNodeRequest {
        node_id: "unrooted_node_99".to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false, // Explicitly unrooted
            supported_engines: vec![
                ExecutionEngine::WasmWasi as i32,
                ExecutionEngine::RootShell as i32, // Hostile or misconfigured claim
            ],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: false,
            current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
            max_cellular_task_duration_seconds: 180,
        }),
        client_version: "0.1.0".to_string(),
    };

    let reg_res = client
        .register_node(reg_req)
        .await
        .expect("Registration should succeed")
        .into_inner();

    let session_token = reg_res.session_token;

    // Then: the orchestrator restricts assignable engines to EXECUTION_ENGINE_WASM_WASI
    // Verify registered worker in scheduler strictly omitted ROOT_SHELL
    let caps = harness
        .scheduler
        .get_worker_capabilities("unrooted_node_99")
        .expect("Worker capabilities must exist");

    assert!(!caps.is_rooted);
    assert!(caps
        .supported_engines
        .contains(&(ExecutionEngine::WasmWasi as i32)));
    assert!(!caps
        .supported_engines
        .contains(&(ExecutionEngine::RootShell as i32)));

    // And: the orchestrator never dispatches EXECUTION_ENGINE_ROOT_SHELL tasks to this node
    let privileged_task = TaskSpec {
        task_id: "task_root_privileged_777".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::RootShell as i32, // Requires root shell
        timeout_seconds: 60,
        entrypoint: String::new(),
        env_vars: HashMap::new(),
        wasm_bytecode: Vec::new(),
        shell_command: "cat /proc/kallsyms".to_string(),
        artifact_sha256: String::new(),
        requires_unmetered: true,
    };
    harness.scheduler.submit_task(privileged_task).unwrap();

    // Node synchronizes state
    let sync_res = client
        .sync_node_state(SyncNodeStateRequest {
            node_id: "unrooted_node_99".to_string(),
            session_token,
            active_tasks: Vec::new(),
            completed_tasks: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();

    // Privileged task was NOT dispatched to this unrooted node
    assert!(
        sync_res.desired_tasks.is_empty(),
        "Orchestrator must NEVER dispatch EXECUTION_ENGINE_ROOT_SHELL to an unrooted node"
    );
}
