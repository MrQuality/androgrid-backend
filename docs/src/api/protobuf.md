# Protobuf gRPC API Specification

This document provides the authoritative Protocol Buffers (v3) API specification for the AndroGrid platform, defined in `proto/androgrid/v1/orchestrator.proto`.

Traceability: `[REQ-PUB-001]`, `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-OP-004]`, `[INV-002]`, `[NFR-SEC-001]`, `[NFR-PERF-001]`.

---

## Service: `AndroGridService`

`AndroGridService` provides the primary communication interface between distributed Android worker nodes and the host control plane.

### Methods

| RPC Method | Input Type | Output Type | Transport Plane | Traceability |
| :--- | :--- | :--- | :--- | :--- |
| `RegisterNode` | `RegisterNodeRequest` | `RegisterNodeResponse` | Wi-Fi LAN | `[REQ-OP-001]` |
| `SyncNodeState` | `SyncNodeStateRequest` | `SyncNodeStateResponse` | Wi-Fi LAN | `[REQ-OP-002]` |
| `ClaimCellularTask` | `ClaimCellularTaskRequest` | `ClaimCellularTaskResponse` | Cellular WAN | `[REQ-OP-003]`, `[INV-003]`, `[NFR-POW-001]` |
| `SubmitCellularResult` | `SubmitCellularResultRequest` | `SubmitCellularResultResponse` | Cellular WAN | `[REQ-OP-004]`, `[INV-004]`, `[NFR-REL-001]` |

#### Method Descriptions

##### `RegisterNode`
Registers a newly discovered mobile node over local Wi-Fi. The node submits hardware capabilities, OS version, ABI, and thermal status. The orchestrator validates the credentials and returns a session lease.

##### `SyncNodeState`
Executes periodic declarative synchronization (every 2.0 seconds) over local Wi-Fi. Reports active and completed task states, and receives newly assigned task specifications or cancellation notices.

##### `ClaimCellularTask`
Requests a spot compute task lease over metered cellular WAN. Operates as a disconnected unary RPC. If no eligible tasks exist, returns `task_available = false`. The client terminates the connection within 200 ms to preserve baseband radio power.

##### `SubmitCellularResult`
Atomically submits task execution results (exit code, stdout, stderr, execution duration) with monotonic attempt fencing. Stale submissions are rejected.

---

## Service: `AdminService`

`AdminService` provides administrative control, task management, and cluster observability for operators and CI automation.

### Methods

| RPC Method | Input Type | Output Type | Description |
| :--- | :--- | :--- | :--- |
| `SubmitTask` | `SubmitTaskRequest` | `SubmitTaskResponse` | Enqueue a compute task into the cluster scheduling queue |
| `GetTaskStatus` | `GetTaskStatusRequest` | `GetTaskStatusResponse` | Retrieve current lifecycle state, exit code, and stdout/stderr |
| `ListWorkerNodes` | `ListWorkerNodesRequest` | `ListWorkerNodesResponse` | List all registered worker nodes and hardware metrics |
| `CancelTask` | `CancelTaskRequest` | `CancelTaskResponse` | Cancel a scheduled or actively running task |
| `UploadArtifact` | `UploadArtifactRequest` | `UploadArtifactResponse` | Upload WASM bytecode to content-addressed storage |

---

## Section 6.3 Canonical gRPC Error Matrix

The following table defines standard error mappings across all RPC interactions:

| RPC Method | gRPC Status Code | Canonical Error Identifier | Trigger Condition | Mandatory Client Action |
| :--- | :--- | :--- | :--- | :--- |
| `ClaimCellularTask` | `16 UNAUTHENTICATED` | `CERTIFICATE_INVALID` | Client certificate expired, revoked, or untrusted. | Terminate session immediately; surface notification to user. |
| `ClaimCellularTask` | `9 FAILED_PRECONDITION` | `UNMETERED_REQUIRED` | All queued tasks require unmetered Wi-Fi. | Backoff for 5 minutes or sleep until Wi-Fi association. |
| `SubmitCellularResult` | `9 FAILED_PRECONDITION` | `TASK_ATTEMPT_STALE` | Task was committed by another worker or lease expired. | Discard local task output; return worker to idle state. |
| `SubmitCellularResult` | `3 INVALID_ARGUMENT` | `PAYLOAD_CORRUPT` | Bytecode SHA-256 or stdout exceeds boundaries. | Log error diagnostic; do not retry. |

---

## Enumerations

### `NetworkTransportType`
- `NETWORK_TRANSPORT_UNSPECIFIED = 0`: Default unspecified network interface.
- `NETWORK_TRANSPORT_WIFI_LAN = 1`: Unmetered Wi-Fi local area network.
- `NETWORK_TRANSPORT_CELLULAR_WAN = 2`: Metered cellular wide area network.

### `ExecutionEngine`
- `EXECUTION_ENGINE_UNSPECIFIED = 0`: Unspecified compute engine.
- `EXECUTION_ENGINE_WASM_WASI = 1`: In-process WebAssembly/WASI Preview 1 sandbox.
- `EXECUTION_ENGINE_ROOT_SHELL = 2`: Root shell execution (deprecated, restricted to rooted hardware).
- `EXECUTION_ENGINE_TOYBOX_USER = 3`: Unprivileged toybox user shell.

### `TaskState`
- `TASK_STATE_UNSPECIFIED = 0`: Unknown state.
- `TASK_STATE_PENDING = 1`: Task enqueued, awaiting worker assignment.
- `TASK_STATE_LEASED = 2`: Task leased to a worker node.
- `TASK_STATE_RUNNING = 3`: Worker actively executing task.
- `TASK_STATE_COMPLETED = 4`: Task executed successfully and finalized.
- `TASK_STATE_FAILED = 5`: Task failed during execution.
- `TASK_STATE_PREEMPTED = 6`: Task preempted by a higher-priority worker.
- `TASK_STATE_TIMED_OUT = 7`: Execution exceeded deadline.

---

## Data Messages

### `NodeCapabilities`
```protobuf
message NodeCapabilities {
  bool is_rooted = 1;
  repeated ExecutionEngine supported_engines = 2;
  uint32 cpu_cores = 3;
  uint64 total_ram_bytes = 4;
  string os_version = 5;
  string abi = 6;
  bool is_metered = 7;
  NetworkTransportType current_transport = 8;
  uint32 max_cellular_task_duration_seconds = 9;
  int32 battery_temperature_celsius = 10;
  uint32 thermal_status = 11;
}
```

### `TaskSpec`
```protobuf
message TaskSpec {
  string task_id = 1;
  uint32 run_attempt = 2;
  ExecutionEngine engine = 3;
  uint32 timeout_seconds = 4;
  string entrypoint = 5;
  map<string, string> env_vars = 6;
  bytes wasm_bytecode = 7;
  string shell_command = 8;
  string artifact_sha256 = 9;
  bool requires_unmetered = 10;
  string artifact_url = 11;
}
```

### `ClaimCellularTaskRequest`
```protobuf
message ClaimCellularTaskRequest {
  string node_id = 1;
  NodeCapabilities capabilities = 2;
  uint32 battery_level_pct = 3;
  bool is_charging = 4;
  int32 battery_temperature_celsius = 5;
  uint32 thermal_status = 6;
}
```

### `ClaimCellularTaskResponse`
```protobuf
message ClaimCellularTaskResponse {
  bool task_available = 1;
  TaskSpec task = 2;
  uint32 deadline_epoch_seconds = 3;
}
```

### `SubmitCellularResultRequest`
```protobuf
message SubmitCellularResultRequest {
  string node_id = 1;
  string task_id = 2;
  uint32 run_attempt = 3;
  int32 exit_code = 4;
  bytes stdout_bytes = 5;
  bytes stderr_bytes = 6;
  string error_code = 7;
  string error_message = 8;
  uint64 execution_duration_ms = 9;
}
```
