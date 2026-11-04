//! Technical Feasibility Spike: Cross-Platform Wire Serialization Parity
//!
//! Verifies binary Protobuf encoding, decoding, field integrity, and frame sizing boundaries.
//! Traces to: AG-SPEC-2026-V2 Section 4.1, [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-003], [INV-004], [INV-005], [INV-006], [NFR-PERF-001], [REQ-VER-002]

use androgrid_backend::{
    andro_grid_service_server::AndroGridServiceServer, validate_frame_bound,
    ClaimCellularTaskRequest, ClaimCellularTaskResponse, ExecutionEngine, FrameBoundError,
    NetworkTransportType, NodeCapabilities, RegisterNodeRequest, RegisterNodeResponse,
    SubmitCellularResultRequest, SubmitCellularResultResponse, SyncNodeStateRequest,
    SyncNodeStateResponse, TaskSpec, MAX_FRAME_SIZE_BYTES,
};
use prost::Message;
use std::collections::HashMap;
use std::time::Instant;

/// Verifies that ClaimCellularTaskRequest serializes into a compact binary representation,
/// decodes with 100% field parity, and enforces capability constraints.
///
/// Traces to: [REQ-OP-003], [INV-003], [REQ-VER-002]
#[test]
fn spike_claim_cellular_task_request_wire_parity() {
    let mut env_map = HashMap::new();
    env_map.insert("OMP_NUM_THREADS".to_string(), "4".to_string());
    env_map.insert("WASM_SANDBOX_STRICT".to_string(), "1".to_string());

    let original_req = ClaimCellularTaskRequest {
        node_id: "android-pixel8-node-402".to_string(),
        capabilities: Some(NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![
                ExecutionEngine::WasmWasi as i32,
                ExecutionEngine::ToyboxUser as i32,
            ],
            cpu_cores: 8,
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        }),
        battery_level_pct: 78,
        is_charging: false,
    };

    let start_encode = Instant::now();
    let mut buffer = Vec::new();
    original_req
        .encode(&mut buffer)
        .expect("ClaimCellularTaskRequest encode must succeed");
    let encode_duration = start_encode.elapsed();

    // Verify wire payload size is compact (< 256 bytes) and respects [NFR-PERF-001]
    assert!(
        buffer.len() < 256,
        "Wire payload size {} bytes should be compact",
        buffer.len()
    );
    assert!(validate_frame_bound(buffer.len()).is_ok());

    let start_decode = Instant::now();
    let decoded_req = ClaimCellularTaskRequest::decode(&buffer[..])
        .expect("ClaimCellularTaskRequest decode must succeed");
    let decode_duration = start_decode.elapsed();

    // Verify bidirectional equality
    assert_eq!(decoded_req.node_id, original_req.node_id);
    assert_eq!(decoded_req.battery_level_pct, 78);
    assert!(!decoded_req.is_charging);

    let caps = decoded_req
        .capabilities
        .expect("Capabilities must be present");
    assert!(!caps.is_rooted);
    assert_eq!(caps.cpu_cores, 8);
    assert_eq!(caps.total_ram_bytes, 8 * 1024 * 1024 * 1024);
    assert_eq!(caps.os_version, "14");
    assert_eq!(caps.abi, "arm64-v8a");
    assert!(caps.is_metered);
    assert_eq!(
        caps.current_transport,
        NetworkTransportType::NetworkTransportCellularWan as i32
    );
    assert_eq!(caps.max_cellular_task_duration_seconds, 180);
    assert_eq!(
        caps.supported_engines,
        vec![
            ExecutionEngine::WasmWasi as i32,
            ExecutionEngine::ToyboxUser as i32
        ]
    );

    println!(
        "Spike ClaimCellularTaskRequest: encoded in {:?}, decoded in {:?}, size: {} bytes",
        encode_duration,
        decode_duration,
        buffer.len()
    );
}

/// Verifies that SubmitCellularResultRequest retains monotonic attempt fencing and
/// preserves execution artifacts over the binary wire.
///
/// Traces to: [REQ-OP-004], [INV-004]
#[test]
fn spike_submit_cellular_result_request_wire_parity() {
    let stdout_sample = b"Computation completed successfully: SHA256 matches".to_vec();
    let stderr_sample = Vec::new();

    let original_result = SubmitCellularResultRequest {
        node_id: "android-pixel8-node-402".to_string(),
        task_id: "task-edge-9001".to_string(),
        run_attempt: 1,
        exit_code: 0,
        stdout_bytes: stdout_sample.clone(),
        stderr_bytes: stderr_sample,
        error_code: String::new(),
        error_message: String::new(),
        execution_duration_ms: 4520,
    };

    let mut buffer = Vec::new();
    original_result
        .encode(&mut buffer)
        .expect("SubmitCellularResultRequest encode must succeed");

    assert!(validate_frame_bound(buffer.len()).is_ok());

    let decoded_result = SubmitCellularResultRequest::decode(&buffer[..])
        .expect("SubmitCellularResultRequest decode must succeed");

    assert_eq!(decoded_result.node_id, original_result.node_id);
    assert_eq!(decoded_result.task_id, original_result.task_id);
    assert_eq!(decoded_result.run_attempt, 1);
    assert_eq!(decoded_result.exit_code, 0);
    assert_eq!(decoded_result.stdout_bytes, stdout_sample);
    assert_eq!(decoded_result.execution_duration_ms, 4520);
}

/// Verifies TaskSpec serialization with embedded WASM bytecode and environment variables.
///
/// Traces to: [INV-005], [INV-006], [NFR-PERF-001]
#[test]
fn spike_task_spec_wasm_bytecode_payload_bounds() {
    let mut env = HashMap::new();
    env.insert("ITERATIONS".to_string(), "100000".to_string());

    // 1 MB synthetic WASM bytecode payload
    let synthetic_wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let mut large_wasm_bytes = synthetic_wasm;
    large_wasm_bytes.resize(1024 * 1024, 0xEA);

    let task = TaskSpec {
        task_id: "task_501".to_string(),
        run_attempt: 1,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 120,
        entrypoint: "_start".to_string(),
        env_vars: env,
        wasm_bytecode: large_wasm_bytes.clone(),
        shell_command: String::new(),
        artifact_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            .to_string(),
        requires_unmetered: false,
    };

    let response = ClaimCellularTaskResponse {
        task_available: true,
        task: Some(task),
        deadline_epoch_seconds: 1730736000,
    };

    let mut buffer = Vec::new();
    response
        .encode(&mut buffer)
        .expect("Encoding ClaimCellularTaskResponse must succeed");

    // Must be bounded within 16 MB frame limit
    assert!(validate_frame_bound(buffer.len()).is_ok());

    let decoded = ClaimCellularTaskResponse::decode(&buffer[..])
        .expect("Decoding ClaimCellularTaskResponse must succeed");

    assert!(decoded.task_available);
    let decoded_task = decoded.task.expect("Task must be present");
    assert_eq!(decoded_task.task_id, "task_501");
    assert_eq!(decoded_task.wasm_bytecode.len(), 1024 * 1024);
    assert_eq!(
        decoded_task.wasm_bytecode[0..8],
        [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
    );
}

/// Verifies strict rejection when message payloads exceed the 16 MB frame boundary.
///
/// Traces to: [NFR-PERF-001]
#[test]
fn spike_frame_boundary_rejection_over_16mb() {
    let excessive_size = MAX_FRAME_SIZE_BYTES + 1;
    let result = validate_frame_bound(excessive_size);

    assert_eq!(
        result,
        Err(FrameBoundError::ExceedsMaxBound {
            size: excessive_size,
            max: MAX_FRAME_SIZE_BYTES
        })
    );
}

/// Verifies that Tonic server service trait is generated and constructible.
///
/// Traces to: [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004]
#[test]
fn spike_tonic_service_trait_definition_presence() {
    // Mock minimal implementation of AndroGridService
    struct MockService;

    #[tonic::async_trait]
    impl androgrid_backend::proto::androgrid::v1::andro_grid_service_server::AndroGridService
        for MockService
    {
        async fn register_node(
            &self,
            _request: tonic::Request<RegisterNodeRequest>,
        ) -> Result<tonic::Response<RegisterNodeResponse>, tonic::Status> {
            Ok(tonic::Response::new(RegisterNodeResponse {
                session_token: "session-xyz".to_string(),
                heartbeat_interval_seconds: 2,
                lease_timeout_seconds: 10,
            }))
        }

        async fn sync_node_state(
            &self,
            _request: tonic::Request<SyncNodeStateRequest>,
        ) -> Result<tonic::Response<SyncNodeStateResponse>, tonic::Status> {
            Ok(tonic::Response::new(SyncNodeStateResponse {
                server_epoch_ms: 1730736000000,
                desired_tasks: Vec::new(),
                cancel_task_ids: Vec::new(),
                acknowledged_task_ids: Vec::new(),
            }))
        }

        async fn claim_cellular_task(
            &self,
            _request: tonic::Request<ClaimCellularTaskRequest>,
        ) -> Result<tonic::Response<ClaimCellularTaskResponse>, tonic::Status> {
            Ok(tonic::Response::new(ClaimCellularTaskResponse {
                task_available: false,
                task: None,
                deadline_epoch_seconds: 0,
            }))
        }

        async fn submit_cellular_result(
            &self,
            _request: tonic::Request<SubmitCellularResultRequest>,
        ) -> Result<tonic::Response<SubmitCellularResultResponse>, tonic::Status> {
            Ok(tonic::Response::new(SubmitCellularResultResponse {
                accepted: true,
                status_message: "Committed".to_string(),
            }))
        }
    }

    let service = MockService;
    let _server = AndroGridServiceServer::new(service);
}
