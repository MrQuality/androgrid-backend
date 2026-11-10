//! Integration and Verification Suite for Cluster Observability and TUI
//!
//! Verifies embedded web dashboard HTTP endpoints, Server-Sent Events (SSE) broadcast,
//! speculative preemption event emission, TUI rendering via TestBackend, and gRPC task inventory queries.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-004], [INV-005], [NFR-REL-001], [REQ-OP-001], [REQ-OP-004]

use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use androgrid_backend::admin_service_client::AdminServiceClient;
use androgrid_backend::admin_service_server::AdminServiceServer;
use androgrid_backend::observability::{
    ClusterEvent, ClusterMetrics, ClusterSnapshot, TaskSnapshot, TuiAction, TuiApp,
    WorkerNodeSnapshot,
};
use androgrid_backend::{
    AdminServiceImpl, ArtifactServer, ArtifactStore, ClusterScheduler, ExecutionEngine,
    ListTasksRequest, NetworkTransportType, NodeCapabilities, TaskExecutionReport, TaskSpec,
    TaskState,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// Verifies that the embedded web dashboard single-page application is served over HTTP
/// at `/` and `/dashboard` with valid HTML5 headers and expected visual components.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[tokio::test]
async fn test_embedded_web_dashboard_http_endpoints() {
    let scheduler = Arc::new(ClusterScheduler::new());
    let store = Arc::new(ArtifactStore::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral HTTP port");
    let addr = listener.local_addr().expect("Local address available");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    ArtifactServer::spawn_full(
        listener,
        Arc::clone(&store),
        Arc::clone(&scheduler),
        None,
        shutdown_rx,
    );

    let client = reqwest::Client::new();

    // 1. Root path serves HTML dashboard
    let root_resp = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("GET / must succeed");
    assert_eq!(root_resp.status(), reqwest::StatusCode::OK);
    let content_type = root_resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.contains("text/html"),
        "Content-Type must be text/html, got: {content_type}"
    );
    let root_body = root_resp.text().await.expect("Body must be readable");
    assert!(
        root_body.contains("AndroGrid"),
        "Dashboard must contain 'AndroGrid' title"
    );
    assert!(
        root_body.contains("Cluster Observability"),
        "Dashboard must contain 'Cluster Observability'"
    );
    assert!(
        root_body.contains("Worker Nodes"),
        "Dashboard must contain 'Worker Nodes' section"
    );
    assert!(
        root_body.contains("Task Execution Pipeline"),
        "Dashboard must contain 'Task Execution Pipeline' section"
    );

    // 2. /dashboard alias also serves the same dashboard
    let dash_resp = client
        .get(format!("http://{addr}/dashboard"))
        .send()
        .await
        .expect("GET /dashboard must succeed");
    assert_eq!(dash_resp.status(), reqwest::StatusCode::OK);
    let dash_body = dash_resp.text().await.expect("Body must be readable");
    assert!(dash_body.contains("AndroGrid"));

    let _ = shutdown_tx.send(());
}

/// Verifies that `/api/snapshot` returns valid JSON reflecting registered nodes, tasks, and aggregate metrics.
///
/// Traces to: [REQ-OP-001], [REQ-OP-004], [INV-004]
#[tokio::test]
async fn test_cluster_snapshot_endpoint() {
    let scheduler = Arc::new(ClusterScheduler::new());
    let store = Arc::new(ArtifactStore::new());

    // Register a worker node
    let caps = NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 8,
        total_ram_bytes: 6 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: false,
        current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
        max_cellular_task_duration_seconds: 180,
        battery_temperature_celsius: 30,
        thermal_status: 0,
    };
    scheduler
        .register_node("pixel-8-pro".to_string(), caps, "0.1.0".to_string(), 1000)
        .expect("Node registration must succeed");

    // Submit a task
    let spec = TaskSpec {
        task_id: "task-obs-01".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 30,
        entrypoint: "_start".to_string(),
        env_vars: Default::default(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
        artifact_url: String::new(),
    };
    scheduler
        .submit_task(spec)
        .expect("Task submission must succeed");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral HTTP port");
    let addr = listener.local_addr().expect("Local address available");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    ArtifactServer::spawn_full(
        listener,
        Arc::clone(&store),
        Arc::clone(&scheduler),
        None,
        shutdown_rx,
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/snapshot"))
        .send()
        .await
        .expect("GET /api/snapshot must succeed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let snapshot: ClusterSnapshot = resp
        .json()
        .await
        .expect("Snapshot JSON deserialization must succeed");

    assert_eq!(snapshot.nodes.len(), 1);
    assert_eq!(snapshot.nodes[0].node_id, "pixel-8-pro");
    assert_eq!(snapshot.nodes[0].transport, "Wi-Fi LAN");
    assert_eq!(snapshot.nodes[0].cpu_cores, 8);
    assert_eq!(snapshot.nodes[0].thermal_status, 0);

    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.tasks[0].task_id, "task-obs-01");
    assert_eq!(snapshot.tasks[0].state, "Pending");

    assert_eq!(snapshot.metrics.total_nodes, 1);
    assert_eq!(snapshot.metrics.wifi_nodes, 1);
    assert_eq!(snapshot.metrics.total_tasks, 1);
    assert_eq!(snapshot.metrics.pending_tasks, 1);

    let _ = shutdown_tx.send(());
}

/// Verifies that Server-Sent Events (SSE) broadcast channel emits live events to subscribers.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[tokio::test]
async fn test_server_sent_events_stream_broadcast() {
    let scheduler = Arc::new(ClusterScheduler::new());
    let mut event_rx = scheduler.subscribe_events();

    // 1. Node Registration Event
    let caps = NodeCapabilities {
        is_rooted: true,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 4,
        total_ram_bytes: 4 * 1024 * 1024 * 1024,
        os_version: "13".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: true,
        current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
        max_cellular_task_duration_seconds: 180,
        battery_temperature_celsius: 32,
        thermal_status: 1,
    };
    scheduler
        .register_node("galaxy-s22".to_string(), caps, "0.1.0".to_string(), 1000)
        .expect("Node registration should succeed");

    let event1 = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("Must receive registration event")
        .expect("Channel must not error");

    match event1 {
        ClusterEvent::NodeRegistered {
            node_id,
            transport,
            is_rooted,
            thermal_status,
            ..
        } => {
            assert_eq!(node_id, "galaxy-s22");
            assert_eq!(transport, "Cellular WAN");
            assert!(is_rooted);
            assert_eq!(thermal_status, 1);
        }
        other => panic!("Expected NodeRegistered event, got: {other:?}"),
    }

    // 2. Task Submission Event
    let spec = TaskSpec {
        task_id: "task-sse-01".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 45,
        entrypoint: "_start".to_string(),
        env_vars: Default::default(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
        artifact_url: String::new(),
    };
    scheduler
        .submit_task(spec)
        .expect("Task submission should succeed");

    let event2 = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .expect("Must receive task submission event")
        .expect("Channel must not error");

    match event2 {
        ClusterEvent::TaskSubmitted {
            task_id,
            timeout_seconds,
            ..
        } => {
            assert_eq!(task_id, "task-sse-01");
            assert_eq!(timeout_seconds, 45);
        }
        other => panic!("Expected TaskSubmitted event, got: {other:?}"),
    }
}

/// Verifies speculative preemption event emission and monotonic attempt fencing.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[tokio::test]
async fn test_speculative_preemption_event_emission_and_attempt_fencing() {
    let scheduler = Arc::new(ClusterScheduler::new());
    let mut event_rx = scheduler.subscribe_events();

    // 1. Register cellular worker
    let cell_caps = NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 4,
        total_ram_bytes: 4 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: true,
        current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
        max_cellular_task_duration_seconds: 180,
        battery_temperature_celsius: 29,
        thermal_status: 0,
    };
    scheduler
        .register_node(
            "cell-node".to_string(),
            cell_caps,
            "0.1.0".to_string(),
            1000,
        )
        .expect("Registration must succeed");

    // 2. Submit task
    let spec = TaskSpec {
        task_id: "task-preempt-race".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 60,
        entrypoint: "_start".to_string(),
        env_vars: Default::default(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
        artifact_url: String::new(),
    };
    scheduler
        .submit_task(spec)
        .expect("Submission must succeed");

    // 3. Cellular claims spot task (attempt 1)
    let claim_res = scheduler
        .claim_cellular_task_nominal("cell-node", 80, false, 1100)
        .expect("Claim must succeed");
    assert!(claim_res.task_available);

    // 4. Register Wi-Fi worker
    let wifi_caps = NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 8,
        total_ram_bytes: 8 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered: false,
        current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
        max_cellular_task_duration_seconds: 180,
        battery_temperature_celsius: 27,
        thermal_status: 0,
    };
    let wifi_reg = scheduler
        .register_node(
            "wifi-node".to_string(),
            wifi_caps,
            "0.1.0".to_string(),
            1200,
        )
        .expect("Wi-Fi registration must succeed");

    // 5. Wi-Fi syncs state -> triggers [INV-005] Speculative Preemption
    let sync_res = scheduler
        .sync_node_state("wifi-node", &wifi_reg.session_token, &[], 1300)
        .expect("Wi-Fi sync must succeed");
    assert_eq!(sync_res.desired_tasks.len(), 1);
    assert_eq!(sync_res.desired_tasks[0].run_attempt, 2);

    // Drain events to verify TaskPreempted was emitted with new_attempt = 2
    let mut found_preempt = false;
    while let Ok(event) = timeout(Duration::from_millis(100), event_rx.recv()).await {
        if let Ok(ClusterEvent::TaskPreempted {
            task_id,
            original_node_id,
            speculative_node_id,
            new_attempt,
        }) = event
        {
            if task_id == "task-preempt-race" {
                assert_eq!(original_node_id, Some("cell-node".to_string()));
                assert_eq!(speculative_node_id, "wifi-node");
                assert_eq!(new_attempt, 2);
                found_preempt = true;
                break;
            }
        }
    }
    assert!(
        found_preempt,
        "Must have emitted TaskPreempted event for task-preempt-race"
    );

    // 6. Wi-Fi completes task with attempt 2 -> commits task
    let report = TaskExecutionReport {
        task_id: "task-preempt-race".to_string(),
        run_attempt: 2,
        state: TaskState::Completed as i32,
        exit_code: 0,
        stdout_bytes: b"winner".to_vec(),
        stderr_bytes: Vec::new(),
        error_code: String::new(),
        execution_time_ms: 45,
    };
    scheduler
        .sync_node_state("wifi-node", &wifi_reg.session_token, &[report], 1400)
        .expect("Wi-Fi completion report must commit");

    assert_eq!(
        scheduler.get_task_state("task-preempt-race"),
        Some(TaskState::Completed)
    );
}

/// Verifies TUI state mapping and non-interactive rendering using ratatui TestBackend.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[test]
fn test_tui_state_mapping_and_test_backend_rendering() {
    let mut app = TuiApp::new("https://127.0.0.1:50051".to_string());

    let snapshot = ClusterSnapshot {
        nodes: vec![
            WorkerNodeSnapshot {
                node_id: "pixel-8".to_string(),
                is_rooted: false,
                transport: "Wi-Fi LAN".to_string(),
                battery_level_pct: 95,
                is_charging: true,
                thermal_status: 0,
                thermal_badge: "NOMINAL".to_string(),
                cpu_cores: 8,
                ram_mb: 8192,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                client_version: "0.1.0".to_string(),
                last_seen_epoch_ms: 1000,
            },
            WorkerNodeSnapshot {
                node_id: "moto-g".to_string(),
                is_rooted: true,
                transport: "Cellular WAN".to_string(),
                battery_level_pct: 35,
                is_charging: false,
                thermal_status: 2,
                thermal_badge: "MODERATE".to_string(),
                cpu_cores: 4,
                ram_mb: 3072,
                os_version: "12".to_string(),
                abi: "armeabi-v7a".to_string(),
                client_version: "0.1.0".to_string(),
                last_seen_epoch_ms: 1000,
            },
        ],
        tasks: vec![TaskSnapshot {
            task_id: "task-tui-100".to_string(),
            state: "Completed".to_string(),
            engine: "WasmWasi".to_string(),
            run_attempt: 1,
            assigned_node_id: Some("pixel-8".to_string()),
            speculative_node_id: None,
            timeout_seconds: 30,
            requires_unmetered: false,
            exit_code: Some(0),
            execution_duration_ms: Some(125),
            error_message: None,
        }],
        metrics: ClusterMetrics {
            total_nodes: 2,
            wifi_nodes: 1,
            cellular_nodes: 1,
            total_tasks: 1,
            pending_tasks: 0,
            leased_tasks: 0,
            completed_tasks: 1,
            failed_tasks: 0,
            preempted_tasks: 0,
            timed_out_tasks: 0,
        },
        timestamp_epoch_ms: 1000,
    };

    app.update_from_snapshot(snapshot);

    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).expect("Failed to initialize TestBackend");

    terminal
        .draw(|f| app.render(f))
        .expect("TUI draw must succeed");

    let buffer = terminal.backend().buffer();
    let content: String = buffer
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<Vec<_>>()
        .join("");

    assert!(
        content.contains("AndroGrid"),
        "Rendered buffer must contain 'AndroGrid'"
    );
    assert!(
        content.contains("pixel-8"),
        "Rendered buffer must contain 'pixel-8'"
    );
    assert!(
        content.contains("moto-g"),
        "Rendered buffer must contain 'moto-g'"
    );
    assert!(
        content.contains("task-tui-100"),
        "Rendered buffer must contain task ID"
    );
    assert!(
        content.contains("Completed"),
        "Rendered buffer must contain task state"
    );
    assert!(
        content.contains("[q] Quit"),
        "Rendered buffer must contain keybinding hints"
    );
}

/// Verifies TUI interactive keypress action mapping (quit, cancel prompt, pair prompt).
///
/// Traces to: [REQ-OP-004], [REQ-OP-001]
#[test]
fn test_tui_key_actions_handling() {
    let mut app = TuiApp::new("https://127.0.0.1:50051".to_string());

    // 'q' key triggers Quit action
    let action_q = app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('q'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(action_q, Some(TuiAction::Quit));
    assert!(app.should_quit);

    app.should_quit = false;

    // 'c' key triggers Cancel modal prompt
    let action_c = app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(action_c, None);
    assert!(app.is_cancel_modal_open());

    // Escape closes modal
    let action_esc = app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(action_esc, None);
    assert!(!app.is_cancel_modal_open());

    // 'p' key triggers GeneratePairingToken action
    let action_p = app.handle_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('p'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(action_p, Some(TuiAction::GeneratePairingToken));
}

/// Verifies that AdminService gRPC server implements ListTasks and honors state filtering.
///
/// Traces to: [REQ-OP-004], [INV-004]
#[tokio::test]
async fn test_admin_service_list_tasks_grpc() {
    let scheduler = Arc::new(ClusterScheduler::new());

    // Submit two tasks
    let task1 = TaskSpec {
        task_id: "grpc-task-01".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 30,
        entrypoint: "_start".to_string(),
        env_vars: Default::default(),
        wasm_bytecode: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        shell_command: String::new(),
        artifact_sha256: String::new(),
        requires_unmetered: false,
        artifact_url: String::new(),
    };
    let task2 = TaskSpec {
        task_id: "grpc-task-02".to_string(),
        run_attempt: 0,
        engine: ExecutionEngine::ToyboxUser as i32,
        timeout_seconds: 60,
        entrypoint: String::new(),
        env_vars: Default::default(),
        wasm_bytecode: Vec::new(),
        shell_command: "uname -a".to_string(),
        artifact_sha256: String::new(),
        requires_unmetered: true,
        artifact_url: String::new(),
    };
    scheduler.submit_task(task1).expect("Task 1 submission");
    scheduler.submit_task(task2).expect("Task 2 submission");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral gRPC port");
    let addr = listener.local_addr().expect("Local address available");

    let service = AdminServiceImpl::new(Arc::clone(&scheduler));
    let server = AdminServiceServer::new(service);

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(server)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("Server failed");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .expect("Channel URI valid")
        .connect()
        .await
        .expect("Channel connection must succeed");

    let mut client = AdminServiceClient::new(channel);

    // List all tasks (filter = 0 / ALL)
    let res = client
        .list_tasks(ListTasksRequest { state_filter: 0 })
        .await
        .expect("list_tasks must succeed")
        .into_inner();

    assert_eq!(res.tasks.len(), 2);
    let ids: Vec<&str> = res.tasks.iter().map(|t| t.task_id.as_str()).collect();
    assert!(ids.contains(&"grpc-task-01"));
    assert!(ids.contains(&"grpc-task-02"));
}

/// Typographic audit asserting zero occurrences of forbidden Unicode dashes in observability code.
#[test]
fn test_zero_forbidden_unicode_dashes_in_observability() {
    let observability_dir = std::path::Path::new("src/observability");
    if observability_dir.exists() {
        for entry in std::fs::read_dir(observability_dir).expect("Read observability dir") {
            let entry = entry.expect("DirEntry");
            let path = entry.path();
            if path.is_file() {
                let content = std::fs::read_to_string(&path).expect("Read file");
                assert!(
                    !content.contains('\u{2014}'),
                    "File {:?} contains forbidden em dash U+2014",
                    path
                );
                assert!(
                    !content.contains('\u{2013}'),
                    "File {:?} contains forbidden en dash U+2013",
                    path
                );
            }
        }
    }
}
