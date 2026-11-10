# Operator Manual and CLI Reference

The `androgrid-cli` tool provides cluster administrators, devops engineers, and automated CI pipelines with complete operational control over the AndroGrid control plane: managing compute task submissions, inspecting registered mobile worker inventories, monitoring runtime status, and cancelling tasks.

Traceability: `[REQ-PUB-001]`, `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[NFR-SEC-001]`, `[NFR-PERF-001]`.

---

## Global Command Line Options

The following flags apply globally across all subcommands. They can be supplied explicitly on the command line or via environment variables:

| Flag | Environment Variable | Type | Default | Description & Operational Impact |
| :--- | :--- | :--- | :--- | :--- |
| `--endpoint <URL>` | `ANDROGRID_ENDPOINT` | String | `https://127.0.0.1:50051` | **Control Plane URL:** Target gRPC endpoint. Must use HTTPS scheme when TLS 1.3 is enabled. |
| `--ca-cert <PATH>` | `ANDROGRID_CA_CERT` | Path | None | **Root CA Certificate:** Path to internal Root CA PEM file. Required when communicating with an mTLS-secured orchestrator. |
| `--client-cert <PATH>` | `ANDROGRID_CLIENT_CERT` | Path | None | **Client Certificate:** Path to operator X.509 client certificate for mutual TLS authentication (`[INV-002]`). |
| `--client-key <PATH>` | `ANDROGRID_CLIENT_KEY` | Path | None | **Client Key:** Path to unencrypted PKCS#8 or SEC1 private key matching `--client-cert`. |
| `--format <FORMAT>` | N/A | Enum | `text` | **Output Format:** Choose `text` for human-friendly terminal badges and tables, or `json` for machine automation. |

---

## Subcommands

### `androgrid-cli pack`

Validates a compiled WebAssembly binary for WASI Preview 1 compliance, computes its cryptographic SHA-256 digest, and optionally generates an ISO 29148 deployment manifest.

Traceability: `[INV-006]`, `[REQ-OP-003]`.

#### Parameter Specification

| Option | Type | Required | Default | Detailed Description |
| :--- | :--- | :--- | :--- | :--- |
| `--wasm <PATH>` | File Path | **Yes** | N/A | Path to the `.wasm` binary file. The tool parses WASM magic headers (`\0asm`) and validates WASI import namespaces. |
| `--entrypoint <SYMBOL>` | String | No | `_start` | Exported function symbol invoked by the runtime. WASI command binaries typically export `_start`. |
| `--output <PATH>` | File Path | No | None | Optional path to write the formatted JSON deployment manifest containing module metadata. |

#### Human-Readable Invocation (`--format text`)
```bash
androgrid-cli pack \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --output manifest.json \
  --format text
```

Expected Output:
```text
[+] WebAssembly module validated successfully: wasi_snapshot_preview1
    Artifact SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    Size:             1258291 bytes (1.20 MB)
    Entrypoint:       _start
[+] Manifest written to manifest.json
```

#### JSON Automation Invocation (`--format json`)
```bash
androgrid-cli pack \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --format json
```

Expected JSON Output:
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

Enqueues a new compute workload into the cluster scheduling queue. Operates in two modes:
1. **Decoupled Artifact Mode (Default):** Uploads the WASM bytecode to the content-addressed artifact store via `AdminService.UploadArtifact` and passes a lightweight URL reference, keeping gRPC payloads sub-4KB (`[NFR-POW-001]`).
2. **In-Band Mode (`--inline`):** Retains raw bytecode in the gRPC message payload (restricted by `[NFR-PERF-001]` 16 MB frame limit).

Traceability: `[REQ-OP-003]`, `[INV-004]`, `[INV-005]`, `[NFR-POW-001]`, `[NFR-PERF-001]`.

#### Parameter Specification

| Option | Type | Required | Default | Detailed Description & Constraints |
| :--- | :--- | :--- | :--- | :--- |
| `--wasm <PATH>` | File Path | Conditional | None | Path to `.wasm` binary. Required unless `--shell` is specified. Conflicts with `--shell`. |
| `--entrypoint <SYM>` | String | No | `_start` | Exported WASI function name. Applicable only with `--wasm`. |
| `--task-id <ID>` | String | No | UUIDv4 | Custom unique task identifier. If omitted, the orchestrator generates a random UUID. |
| `--inline` | Boolean Flag | No | `false` | When set, embeds raw bytecode inside the gRPC payload instead of uploading to content-addressed storage. |
| `--shell <CMD>` | String | Conditional | None | Shell command string. Conflicts with `--wasm`. |
| `--root` | Boolean Flag | No | `false` | Grants root execution privileges. Requires `--shell`. Only executed by rooted worker nodes (`[INV-006]`). |
| `--timeout <SECS>` | Integer | No | `30` | Maximum allowed runtime in seconds before the orchestrator triggers task timeout. |
| `--requires-unmetered` | Boolean Flag | No | `false` | When enabled, prevents scheduling to cellular workers (`[ANTI-003]`). Restricts to Wi-Fi nodes. |
| `--artifact-url <URL>` | String | No | None | Explicit HTTP/HTTPS pre-cached URL for artifact download by mobile clients. |
| `--env <KEY=VALUE>` | Key-Value Pair | No | Empty | Environment variables provided to the runtime environment. Can be specified multiple times. |

#### Example Invocation: Standard WASM Submission
```bash
androgrid-cli submit \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 60 \
  --env MATRIX_DIM=512 \
  --env LOG_LEVEL=debug \
  --format text
```

Expected Output:
```text
[+] Bytecode artifact uploaded to content-addressed storage
    SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    URL:     https://127.0.0.1:50051/artifacts/3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a.wasm
[+] Task successfully enqueued
    Task ID: task_c9f28a7e_41b3_4e89_a248_18a28f731e50
```

---

### `androgrid-cli status`

Fetches real-time execution progress, completion state, exit codes, and output buffers for an assigned task.

#### Arguments & Options

| Argument | Position | Required | Description |
| :--- | :--- | :--- | :--- |
| `<TASK_ID>` | Positional (1) | **Yes** | The canonical task identifier returned during submission. |

#### Example Invocation
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

---

### `androgrid-cli nodes`

Queries the active worker node registry, detailing device architecture, battery levels, thermal metrics, and network transport modes.

#### Example Invocation
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

---

### `androgrid-cli cancel`

Signals the orchestrator state machine to transition a pending or running task into the `TASK_STATE_PREEMPTED` or `CANCELLED` state.

#### Arguments & Options

| Argument | Position | Required | Description |
| :--- | :--- | :--- | :--- |
| `<TASK_ID>` | Positional (1) | **Yes** | The canonical task identifier to abort. |

#### Example Invocation
```bash
androgrid-cli cancel task_c9f28a7e_41b3_4e89_a248_18a28f731e50 --format text
```

Output:
```text
[+] Task task_c9f28a7e_41b3_4e89_a248_18a28f731e50 successfully cancelled.
```

---

### `androgrid-cli pair`

Generates a time-bounded ephemeral device pairing token, renders an interactive high-contrast ANSI UTF-8 QR code directly in the terminal, and provides a textual fallback payload for zero-configuration Android worker node onboarding.

Traceability: `[INV-002]`, `[NFR-SEC-001]`, `[REQ-OP-001]`.

#### Parameter Specification

| Option | Type | Required | Default | Detailed Description |
| :--- | :--- | :--- | :--- | :--- |
| `--node-id <ID>` | String | No | None | Optional pre-assigned worker node identifier to bind strictly to this pairing session. |
| `--expires-in <SECONDS>` | Integer | No | `300` | Pairing token lifetime in seconds (5 minutes by default). Enforces single-use expiry. |
| `--san <HOST>` | String | No | None | Explicit orchestrator SAN domain or IP (e.g. `192.168.1.100:50051`) embedded into QR payload. |
| `--output-png <PATH>` | File Path | No | None | Optional destination path to render and save the QR code as a standalone PNG image file. |

#### Human-Readable Terminal Invocation (`--format text`)
```bash
androgrid-cli pair \
  --node-id node_pixel_7_01 \
  --expires-in 300 \
  --output-png ./pairing_qr.png \
  --format text
```

Expected Output:
```text
================================================================================
                      AndroGrid Device Pairing Engine                           
================================================================================
Scan this QR code from the AndroGrid Android worker application:

[ ANSI UTF-8 QR CODE MATRIX ]

--------------------------------------------------------------------------------
Pairing Details (Manual Fallback):
  Endpoint:       https://127.0.0.1:50051
  CA Fingerprint: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
  Pairing Token:  8a7f92e104b281f6d3910c2834b71940
  Bound Node ID:  node_pixel_7_01
  Expires At:     epoch 1700000300 (TTL 300s)
  QR Image PNG:   ./pairing_qr.png
--------------------------------------------------------------------------------
Raw Scannable Payload:
{"cluster_endpoint":"https://127.0.0.1:50051","ca_fingerprint":"3c79...","pairing_token":"8a7f...","node_id":"node_pixel_7_01","expires_at_epoch_s":1700000300}
================================================================================
```

#### JSON Automation Invocation (`--format json`)
```bash
androgrid-cli pair \
  --node-id node_pixel_7_01 \
  --expires-in 300 \
  --format json
```

Expected JSON Output:
```json
{
  "pairing_token": "8a7f92e104b281f6d3910c2834b71940",
  "cluster_endpoint": "https://127.0.0.1:50051",
  "ca_fingerprint": "3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a",
  "node_id": "node_pixel_7_01",
  "expires_at_epoch_seconds": 1700000300,
  "pairing_payload_json": "{\"cluster_endpoint\":\"https://127.0.0.1:50051\",\"ca_fingerprint\":\"3c79...\",\"pairing_token\":\"8a7f...\",\"node_id\":\"node_pixel_7_01\",\"expires_at_epoch_s\":1700000300}",
  "output_png": null
}
```

---

### `androgrid-cli benchmark`

Executes the canonical distributed image-processing hero workload across connected Android worker nodes or local simulated runner. Partitions high-resolution input images into discrete geometric tiles, dispatches them concurrently over gRPC or local workers, aggregates processing latencies, measures parallel speedup ratios vs a single-core CPU baseline, and reassembles the composite result image.

Traceability: `[INV-005]`, `[INV-006]`, `[NFR-PERF-001]`, `[REQ-OP-004]`.

#### Parameter Specification

| Option | Type | Required | Default | Detailed Description & Constraints |
| :--- | :--- | :--- | :--- | :--- |
| `--input <PATH>` | File Path | **Yes** | N/A | Path to source image file (JPEG or PNG format). |
| `--output <PATH>` | File Path | **Yes** | N/A | Destination path to write the reassembled composite output image. |
| `--workload <NAME>` | String | No | `image-processor` | Workload identifier name. |
| `--tiles <COUNT>` | Integer | No | `16` | Number of geometric tiles to partition image into (e.g. 1, 4, 9, 16). Slices conform to the 16 MB frame limit (`[NFR-PERF-001]`). |
| `--filter <FILTER>` | String | No | `sobel` | Image filter operation: `sobel`, `blur`, `downscale`, `equalize`, `grayscale`, or `invert`. |
| `--concurrency <N>` | Integer | No | `4` | Maximum concurrent worker tasks to dispatch in parallel. |
| `--timeout <SECS>` | Integer | No | `60` | Per-tile task timeout limit in seconds. |
| `--requires-unmetered` | Boolean Flag | No | `false` | When enabled, schedules only on unmetered Wi-Fi worker nodes (`[ANTI-003]`). |
| `--wasm <PATH>` | File Path | No | None | Optional path to custom compiled `.wasm` bytecode for execution. |
| `--dry-run` | Boolean Flag | No | `false` | When enabled, runs multi-worker simulation locally without connecting to a remote orchestrator. |

#### Human-Readable Invocation (`--format text`)
```bash
androgrid-cli benchmark \
  --input ./tests/fixtures/images/benchmark_sample.png \
  --output ./result_sobel.png \
  --tiles 16 \
  --filter sobel \
  --concurrency 4 \
  --dry-run \
  --format text
```

Expected Output:
```text
========================================================
          AndroGrid Parallel Compute Benchmark          
========================================================
Workload:                 image-processor
Filter Operation:         sobel
Input Dimensions:         1024x1024 pixels
Output Dimensions:        1024x1024 pixels
Tiles:                    16/16 completed
Active Cluster Workers:   4
--------------------------------------------------------
Single-Core Baseline:     180 ms
Distributed Wall-Clock:   52 ms
Parallel Speedup Ratio:   3.46x
Payload Transferred:      1.82 MB (1910240 bytes)
--------------------------------------------------------
Worker Latency Distribution:
  - Node worker-01: tile #0 (attempt 1) finished in 12 ms
  - Node worker-02: tile #1 (attempt 1) finished in 14 ms
  - Node worker-03: tile #2 (attempt 1) finished in 11 ms
  - Node worker-04: tile #3 (attempt 1) finished in 13 ms
========================================================
```

#### JSON Automation Invocation (`--format json`)
```bash
androgrid-cli benchmark \
  --input ./tests/fixtures/images/benchmark_sample.png \
  --output ./result_blur.png \
  --tiles 4 \
  --filter blur \
  --dry-run \
  --format json
```

Expected JSON Output:
```json
{
  "workload": "image-processor",
  "filter": "blur",
  "input_width": 1024,
  "input_height": 1024,
  "output_width": 1024,
  "output_height": 1024,
  "tiles_total": 4,
  "tiles_successful": 4,
  "active_worker_count": 4,
  "wall_clock_duration_ms": 48,
  "single_thread_baseline_ms": 144,
  "parallel_speedup_ratio": 3.0,
  "total_payload_bytes_transferred": 819200,
  "node_latencies": [
    {
      "node_id": "worker-01",
      "tile_index": 0,
      "attempt": 1,
      "duration_ms": 22
    }
  ]
}
```

---

### `androgrid-cli top`

Launches an interactive, real-time terminal UI cluster monitor (powered by `ratatui` and `crossterm`) for visualizing connected worker nodes, hardware capabilities, task queues, and speculative preemption events.

#### Traceability
- Functional Requirements: `[REQ-OP-001]`, `[REQ-OP-004]`
- System Invariants: `[INV-004]`, `[INV-005]`, `[NFR-REL-001]`

#### Syntax & Options
```text
Usage: androgrid-cli top [OPTIONS]

Options:
      --refresh-rate <REFRESH_RATE>  Polling refresh interval in seconds [default: 1]
      --endpoint <ENDPOINT>          gRPC control plane endpoint [default: https://127.0.0.1:50051]
      --ca-cert <CA_CERT>            Path to root CA PEM certificate for mTLS
      --client-cert <CLIENT_CERT>    Path to operator client PEM certificate for mTLS
      --client-key <CLIENT_KEY>      Path to operator client private key PEM for mTLS
  -h, --help                         Print help
```

#### Interactive Keybindings
- `q`: Exit the terminal monitor cleanly, restoring terminal screen buffer and cursor.
- `c`: Open task cancellation dialog to cancel a pending or leased task by ID.
- `p`: Generate an ephemeral mobile node onboarding pairing token and display scannable payload.
- `r`: Force an immediate telemetry refresh from the control plane.

#### Example Invocation
```bash
androgrid-cli top --endpoint https://127.0.0.1:50051 --refresh-rate 1
```

