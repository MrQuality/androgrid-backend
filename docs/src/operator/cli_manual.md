# Operator Manual and CLI Reference

The `androgrid-cli` tool provides cluster administrators and automation pipelines with direct control over the AndroGrid control plane: managing compute task submissions, inspecting worker node inventories, and tracking execution states.

Traceability: `[REQ-PUB-001]`, `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[NFR-SEC-001]`, `[NFR-PERF-001]`.

---

## Global Options

The following flags apply globally across all subcommands and can be provided as CLI arguments or environment variables:

| Flag | Environment Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| `--endpoint <URL>` | `ANDROGRID_ENDPOINT` | `https://127.0.0.1:50051` | gRPC control plane URL |
| `--ca-cert <PATH>` | `ANDROGRID_CA_CERT` | None | Path to root CA PEM certificate for TLS 1.3 mTLS |
| `--client-cert <PATH>` | `ANDROGRID_CLIENT_CERT` | None | Path to operator client PEM certificate |
| `--client-key <PATH>` | `ANDROGRID_CLIENT_KEY` | None | Path to operator client private key PEM |
| `--format <FORMAT>` | N/A | `text` | Output format: `text` or `json` |

---

## Subcommands

### `androgrid-cli pack`

Validates a WebAssembly module for WASI Preview 1 compliance and generates a deployment manifest.

Traceability: `[INV-006]`, `[REQ-OP-003]`.

#### Options
- `--wasm <PATH>`: (Required) Path to the WebAssembly module bytecode (`.wasm`).
- `--entrypoint <SYMBOL>`: (Optional, default: `_start`) Exported WASM entrypoint function name.
- `--output <PATH>`: (Optional) Output path for saving the generated manifest JSON.

#### Example: Human-Readable Text Output (`--format text`)
```bash
androgrid-cli pack \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --output manifest.json \
  --format text
```

Output:
```text
[+] WebAssembly module validated successfully: wasi_snapshot_preview1
    Artifact SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    Size:             1258291 bytes (1.20 MB)
    Entrypoint:       _start
[+] Manifest written to manifest.json
```

#### Example: Structured JSON Output (`--format json`)
```bash
androgrid-cli pack \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --format json
```

Output:
```json
{
  "name": "wasi_starter.wasm",
  "entrypoint": "_start",
  "artifact_sha256": "3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a",
  "size_bytes": 1258291,
  "engine": "EXECUTION_ENGINE_WASM_WASI",
  "wasi_version": "wasi_snapshot_preview1",
  "manifest_path": null
}
```

---

### `androgrid-cli submit`

Enqueues a compute task into the cluster scheduling queue. Supports automatic content-addressed bytecode uploading and decoupled URLs.

Traceability: `[REQ-OP-003]`, `[INV-004]`, `[NFR-POW-001]`.

#### Options
- `--task-id <ID>`: (Optional) Explicit task identifier. If omitted, a UUID is automatically generated.
- `--wasm <PATH>`: Path to WebAssembly module bytecode (`.wasm`). Conflicts with `--shell`.
- `--entrypoint <SYMBOL>`: (Default: `_start`) Entrypoint symbol for WASM execution.
- `--inline`: Retains raw bytecode within the gRPC message payload instead of decoupled content-addressed storage.
- `--shell <COMMAND>`: Shell command string to execute. Conflicts with `--wasm`.
- `--root`: Executes shell command with root privileges. Requires rooted worker node (`is_rooted == true`). Requires `--shell`.
- `--timeout <SECONDS>`: (Default: 30) Execution timeout deadline in seconds.
- `--requires-unmetered`: Restricts task scheduling to unmetered Wi-Fi connections only (`[ANTI-003]`).
- `--artifact-url <URL>`: Explicit download URL for pre-cached artifact distribution.
- `--env <KEY=VALUE>`: Environment variables passed to compute runtime. Can be repeated.

#### Example: Submit WASM Task (`--format text`)
```bash
androgrid-cli submit \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 60 \
  --env MATRIX_DIM=512 \
  --format text
```

Output:
```text
[+] Bytecode artifact uploaded to content-addressed storage
    SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    URL:     https://127.0.0.1:50051/artifacts/3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a.wasm
[+] Task successfully enqueued
    Task ID: task_c9f28a7e_41b3_4e89_a248_18a28f731e50
```

#### Example: Submit WASM Task (`--format json`)
```bash
androgrid-cli submit \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 60 \
  --format json
```

Output:
```json
{
  "accepted": true,
  "task_id": "task_c9f28a7e_41b3_4e89_a248_18a28f731e50",
  "message": "Task queued for scheduling"
}
```

---

### `androgrid-cli status`

Queries the execution status, exit code, stdout, and stderr for a task.

#### Arguments
- `<TASK_ID>`: (Positional, required) Task identifier.

#### Example: Query Task Status (`--format text`)
```bash
androgrid-cli status task_c9f28a7e_41b3_4e89_a248_18a28f731e50 --format text
```

Output:
```text
Task ID:           task_c9f28a7e_41b3_4e89_a248_18a28f731e50
State:             TASK_STATE_COMPLETED
Active Attempt:    1
Exit Code:         0
Execution Time:    142 ms
Stdout:
[WASI Starter] Matrix computation verified successfully.
```

#### Example: Query Task Status (`--format json`)
```bash
androgrid-cli status task_c9f28a7e_41b3_4e89_a248_18a28f731e50 --format json
```

Output:
```json
{
  "task_id": "task_c9f28a7e_41b3_4e89_a248_18a28f731e50",
  "state": "TASK_STATE_COMPLETED",
  "active_attempt": 1,
  "exit_code": 0,
  "execution_duration_ms": 142,
  "stdout": "[WASI Starter] Matrix computation verified successfully.\n",
  "stderr": "",
  "error_code": "",
  "error_message": ""
}
```

---

### `androgrid-cli nodes`

Lists all registered mobile worker nodes, their battery levels, thermal metrics, and network transport types.

#### Example: Inventory Nodes (`--format text`)
```bash
androgrid-cli nodes --format text
```

Output:
```text
Registered Worker Nodes (1):
--------------------------------------------------------------------------------
Node ID:        node_pixel_7_01
Version:        0.1.0 (Android 14 / arm64-v8a)
Transport:      NETWORK_TRANSPORT_WIFI_LAN (Unmetered)
Rooted:         false
CPU Cores:      8
RAM:            7.42 GB
Thermal Status: 0 (THERMAL_STATUS_NONE)
Last Seen:      1s ago
```

#### Example: Inventory Nodes (`--format json`)
```bash
androgrid-cli nodes --format json
```

Output:
```json
[
  {
    "node_id": "node_pixel_7_01",
    "client_version": "0.1.0",
    "os_version": "14",
    "abi": "arm64-v8a",
    "is_rooted": false,
    "is_metered": false,
    "transport": "NETWORK_TRANSPORT_WIFI_LAN",
    "cpu_cores": 8,
    "ram_bytes": 7969177600,
    "thermal_status": 0,
    "last_seen_secs_ago": 1
  }
]
```

---

### `androgrid-cli cancel`

Cancels an active or pending task in the cluster state engine.

#### Arguments
- `<TASK_ID>`: (Positional, required) Task identifier to cancel.

#### Example: Cancel Task (`--format text`)
```bash
androgrid-cli cancel task_c9f28a7e_41b3_4e89_a248_18a28f731e50 --format text
```

Output:
```text
[+] Task task_c9f28a7e_41b3_4e89_a248_18a28f731e50 successfully cancelled.
```

#### Example: Cancel Task (`--format json`)
```bash
androgrid-cli cancel task_c9f28a7e_41b3_4e89_a248_18a28f731e50 --format json
```

Output:
```json
{
  "task_id": "task_c9f28a7e_41b3_4e89_a248_18a28f731e50",
  "cancelled": true,
  "message": "Task transitioned to CANCELLED state"
}
```
