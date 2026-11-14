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

**Expected Outcome:**
- Exit code `0`, JSON printed to `stdout` containing `"action": "cert init"`, a 64-character `ca_fingerprint`, and no private key material.
- Files created:
  - `./deploy/pki-e2e/runtime/{ca.crt,server.crt,server.key}` (`server.crt` includes `OU=Orchestrator` and fullchain `leaf + Root CA`)
  - `./deploy/pki-e2e/operator/{ca.key,admin.crt,admin.key}` (`admin.crt` includes `OU=Control Plane`)

### Step 2: Sign a Worker CSR Offline (Optional Manual Enrollment Check)

If testing offline CSR signing instead of online QR pairing, generate or export the worker CSR (`worker.csr`) for the target `node_id` and sign it with `cert issue csr`:

```bash
./target/release/androgrid-cli --format json cert issue csr \
  --csr ./deploy/pki-e2e/worker.csr \
  --node-id node_android_worker_01 \
  --ca-dir ./deploy/pki-e2e \
  --out ./deploy/pki-e2e/client.crt
```

**Expected Outcome:**
- Exit code `0`, JSON containing `"action": "cert issue csr"` and `"node_id": "node_android_worker_01"`.
- `./deploy/pki-e2e/client.crt` is issued with `BasicConstraints: CA:FALSE`, `KeyUsage: Digital Signature`, `ExtendedKeyUsage: TLS Web Client Authentication`, and `OU=Edge Compute`.

### Step 3: Forward Ports & Start the Orchestrator

Forward the gRPC mTLS port from the Android device to the workstation:

```bash
adb reverse tcp:50051 tcp:50051
```

Start `androgrid-backend` using the `runtime/` bundle and `operator/ca.key`:

```bash
export ANDROGRID_LISTEN_ADDR="127.0.0.1:50051"
export ANDROGRID_CA_CERT_PATH="./deploy/pki-e2e/runtime/ca.crt"
export ANDROGRID_CA_KEY_PATH="./deploy/pki-e2e/operator/ca.key"
export ANDROGRID_SERVER_CERT_PATH="./deploy/pki-e2e/runtime/server.crt"
export ANDROGRID_SERVER_KEY_PATH="./deploy/pki-e2e/runtime/server.key"

./target/release/androgrid-backend
```

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

### Step 6: Cleanup

Stop `androgrid-backend`, remove the temporary `./deploy/pki-e2e` directory, and clear ADB port forwarding (`adb reverse --remove-all`).
