# Reference Test Case: Offline PKI (`androgrid-cli cert`) & Hardware E2E Verification

This guide provides a reproducible, sanitized end-to-end verification test case for validating the pure-Rust `androgrid-cli cert` PKI lifecycle against `androgrid-backend` and an Android worker device over USB ADB reverse tunneling.

Traceability: `[INV-001]`, `[INV-002]`, `[NFR-SEC-001]`, `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-OP-004]`.

---

## 1. Prerequisites

- Release binaries built (or downloaded): `androgrid-backend` and `androgrid-cli`
- `adb` (Android Platform Tools) with USB debugging enabled on the test device
- Sample WASI fixture: `tests/fixtures/hello_wasi.wasm`
- Android Worker app installed on the test device (generic placeholder Node ID below: `node_android_worker_01`)

> [!NOTE]
> All device identifiers, host paths, and certificate outputs below use sanitized placeholder values. Never commit generated private keys (`*.key`) or device-specific diagnostic dumps to version control.

---

## 2. Step-by-Step Execution

### Step 1: Bootstrap Cluster PKI Offline

Generate a fresh split `runtime/` and `operator/` PKI hierarchy in a temporary directory (`./deploy/pki-e2e`):

```bash
./target/release/androgrid-cli --format json cert init --out ./deploy/pki-e2e
```

> [!NOTE]
> **Purpose:** Initializes a self-contained ECDSA P-256 Root CA, orchestrator server certificate, and operator admin client certificate without requiring `openssl` or a running server. Splitting `runtime/` from `operator/` ensures the orchestrator never needs `admin.key`.

| Argument | Meaning |
| :--- | :--- |
| `--format json` | Emits machine-readable JSON metadata to `stdout` (without leaking private key material). |
| `cert init` | Creates the Root CA, `runtime/server.{crt,key}`, and `operator/admin.{crt,key}`. |
| `--out ./deploy/pki-e2e` | Target directory where the `runtime/` and `operator/` subdirectories are created. |

**Expected Outcome:**
- Exit code `0`, JSON printed to `stdout` containing `"action": "cert init"`, a 64-character `ca_fingerprint`, and no private key material.
- Files created:
  - `./deploy/pki-e2e/runtime/{ca.crt,server.crt,server.key}` (`server.crt` includes `OU=Orchestrator` and fullchain `leaf + Root CA`)
  - `./deploy/pki-e2e/operator/{ca.key,admin.crt,admin.key}` (`admin.crt` includes `OU=Control Plane`)

---

### Step 2: Sign a Worker CSR Offline (Optional Manual Enrollment Check)

If testing offline CSR signing instead of online QR pairing, export or generate the worker CSR (`worker.csr`) for the target `node_id` and sign it with `cert issue csr`:

```bash
./target/release/androgrid-cli --format json cert issue csr \
  --csr ./deploy/pki-e2e/worker.csr \
  --node-id node_android_worker_01 \
  --ca-dir ./deploy/pki-e2e \
  --out ./deploy/pki-e2e/client.crt
```

> [!NOTE]
> **Purpose:** Validates offline worker certificate issuance (`[INV-002]`, `[NFR-SEC-001]`). The signer strips any untrusted extensions/SANs requested in the CSR, pins the Subject CN to `--node-id`, and enforces `OU=Edge Compute` and `ClientAuth` EKU so a worker certificate can never impersonate an operator (`OU=Control Plane`) or server (`OU=Orchestrator`).

| Argument | Meaning |
| :--- | :--- |
| `cert issue csr` | Signs a PKCS#10 Certificate Signing Request (`.csr`) using the cluster Root CA. |
| `--csr ./deploy/pki-e2e/worker.csr` | Path to the worker-generated CSR containing its public key. |
| `--node-id node_android_worker_01` | Expected worker identity; must match the CSR CN and is bound into the signed certificate. |
| `--ca-dir ./deploy/pki-e2e` | Root PKI directory containing `runtime/ca.crt` and `operator/ca.key`. |
| `--out ./deploy/pki-e2e/client.crt` | Output path for the signed worker client certificate. |

**Expected Outcome:**
- Exit code `0`, JSON containing `"action": "cert issue csr"` and `"node_id": "node_android_worker_01"`.
- `./deploy/pki-e2e/client.crt` is issued with `BasicConstraints: CA:FALSE`, `KeyUsage: Digital Signature`, `ExtendedKeyUsage: TLS Web Client Authentication`, and `OU=Edge Compute`.

---

### Step 3: Forward Ports & Start the Orchestrator

Forward the gRPC mTLS port from the Android device to the workstation:

```bash
adb reverse tcp:50051 tcp:50051
```

> [!TIP]
> **Why `adb reverse`?** When connected over USB, the Android device cannot directly route to `127.0.0.1` on your workstation. `adb reverse tcp:50051 tcp:50051` tunnels outbound connections from `127.0.0.1:50051` on the phone back to port `50051` on your host, preserving `[INV-001]` (the Android worker still opens zero listening sockets).

Start `androgrid-backend` using the `runtime/` bundle and `operator/ca.key`:

```bash
export ANDROGRID_LISTEN_ADDR="127.0.0.1:50051"
export ANDROGRID_CA_CERT_PATH="./deploy/pki-e2e/runtime/ca.crt"
export ANDROGRID_CA_KEY_PATH="./deploy/pki-e2e/operator/ca.key"
export ANDROGRID_SERVER_CERT_PATH="./deploy/pki-e2e/runtime/server.crt"
export ANDROGRID_SERVER_KEY_PATH="./deploy/pki-e2e/runtime/server.key"

./target/release/androgrid-backend
```

> [!NOTE]
> **Purpose:** Launches the control plane gRPC server with strict TLS 1.3 mutual authentication (`[INV-002]`). Providing `ANDROGRID_CA_KEY_PATH` is optional and only needed if testing online QR pairing (`pair`); omit it for pure offline-signed worker verification.

| Environment Variable | Meaning |
| :--- | :--- |
| `ANDROGRID_LISTEN_ADDR` | Socket address the gRPC server binds to (`127.0.0.1:50051` for local USB/ADB testing). |
| `ANDROGRID_CA_CERT_PATH` | Root CA certificate (`runtime/ca.crt`) used to verify incoming worker and admin client certificates. |
| `ANDROGRID_CA_KEY_PATH` | Root CA private key (`operator/ca.key`), used only when online QR enrollment is enabled. |
| `ANDROGRID_SERVER_CERT_PATH` | Fullchain orchestrator certificate (`runtime/server.crt`) presented during TLS 1.3 handshake. |
| `ANDROGRID_SERVER_KEY_PATH` | Orchestrator private key (`runtime/server.key`) matching `server.crt`. |

---

### Step 4: Connect the Worker & Verify Node Registration

Enroll the Android worker using the QR pairing flow (`androgrid-cli pair`) or provisioned `ca.crt` / `client.crt` / `client.key`, set endpoint to `127.0.0.1:50051`, and start worker participation in the app.

Query registered nodes using the operator certificate (`operator/admin.crt` and `operator/admin.key`):

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki-e2e/runtime/ca.crt \
  --client-cert ./deploy/pki-e2e/operator/admin.crt \
  --client-key ./deploy/pki-e2e/operator/admin.key \
  --format json \
  nodes
```

> [!NOTE]
> **Purpose:** Verifies both TLS 1.3 mTLS connectivity from the Android worker and `AdminService` RBAC (`OU=Control Plane`) from the operator CLI (`[REQ-OP-001]`, `[REQ-OP-002]`).

| Argument | Meaning |
| :--- | :--- |
| `--endpoint https://127.0.0.1:50051` | Orchestrator gRPC TLS endpoint to connect to. |
| `--ca-cert ./deploy/pki-e2e/runtime/ca.crt` | Root CA used by the CLI to verify the orchestrator's `server.crt`. |
| `--client-cert ./deploy/pki-e2e/operator/admin.crt` | Operator client certificate (`OU=Control Plane`) required by `AdminService`. |
| `--client-key ./deploy/pki-e2e/operator/admin.key` | Operator private key matching `admin.crt`. |
| `nodes` | Lists all worker nodes currently registered and heartbeating with the orchestrator. |

**Expected Outcome:**
JSON array containing the active worker node over TLS 1.3 mTLS:

```json
[
  {
    "node_id": "node_android_worker_01",
    "cpu_cores": 8,
    "ram_mb": 7573,
    "os_version": "17",
    "abi": "arm64-v8a",
    "is_rooted": false,
    "is_metered": false,
    "transport": "Wi-Fi LAN",
    "battery_temperature_celsius": 30,
    "thermal_status": "NONE",
    "client_version": "0.1.0"
  }
]
```

---

### Step 5: Submit & Verify a WASI Workload

Submit `hello_wasi.wasm` inline and poll task status:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki-e2e/runtime/ca.crt \
  --client-cert ./deploy/pki-e2e/operator/admin.crt \
  --client-key ./deploy/pki-e2e/operator/admin.key \
  submit --task-id e2e-cert-cli-hello --wasm tests/fixtures/hello_wasi.wasm --inline

./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki-e2e/runtime/ca.crt \
  --client-cert ./deploy/pki-e2e/operator/admin.crt \
  --client-key ./deploy/pki-e2e/operator/admin.key \
  --format json \
  status e2e-cert-cli-hello
```

> [!NOTE]
> **Purpose:** Exercises end-to-end task scheduling, gRPC payload transfer, embedded WASI sandbox execution on Android (`arm64-v8a`), and monotonic attempt result reporting (`[REQ-OP-003]`, `[REQ-OP-004]`).

| Argument | Meaning |
| :--- | :--- |
| `submit` | Enqueues a new workload onto the orchestrator scheduler. |
| `--task-id e2e-cert-cli-hello` | Explicit task identifier for deterministic status polling. |
| `--wasm tests/fixtures/hello_wasi.wasm` | Path to the compiled `wasm32-wasip1` binary fixture. |
| `--inline` | Embeds the WASM bytecode directly inside the gRPC request (no separate HTTP artifact server needed). |
| `status e2e-cert-cli-hello` | Queries the current lifecycle state, exit code, `stdout`, `stderr`, and duration for the task. |

**Expected Outcome:**

```json
{
  "task_id": "e2e-cert-cli-hello",
  "state": "COMPLETED",
  "active_attempt": 1,
  "exit_code": 0,
  "stdout": "Hello, AndroGrid WASM!\n",
  "stderr": "",
  "error_code": "",
  "error_message": "",
  "execution_duration_ms": 177
}
```

---

### Step 6: Cleanup

Stop `androgrid-backend`, remove the temporary `./deploy/pki-e2e` directory, and clear ADB port forwarding (`adb reverse --remove-all`).

> [!TIP]
> **Clean Workspace Hygiene:** Removing `./deploy/pki-e2e` and running `adb reverse --remove-all` ensures no temporary private keys remain on disk and no stale port tunnels interfere with subsequent test runs.
