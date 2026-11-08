# Quickstart Guide: Local Cluster Deployment

This guide provides a comprehensive, step-by-step walkthrough to deploy and verify an AndroGrid control plane cluster locally using **Podman** for rootless container orchestration.

Traceability: `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[ANTI-002]`, `[NFR-SEC-001]`.

---

## Architectural Rationale: Why Podman?

AndroGrid standardizes on **Podman** (`podman compose`) over traditional Docker for all containerized control plane deployments.

### Key Security & Operational Benefits:
1. **Rootless by Default:** Podman runs entirely without root privileges or a background root daemon (`dockerd`). If a container process is compromised, the attacker has zero root permissions on the host system.
2. **Fork/Exec Model:** Each container runs directly as a child process of the invoking user, allowing granular audit logging, direct Linux cgroup v2 resource limits, and native `systemd` integration.
3. **SELinux Volume Relabeling (`:Z`):** Podman automatically applies private unshared SELinux security context labels to mounted PKI volumes, protecting certificates from access by unauthorized host processes.

---

## Prerequisites

Before starting, ensure the following utilities are installed on your host system:
- **Podman & Podman Compose:** Podman version 4.5+ with `podman-compose` or `podman compose` provider.
- **Rust Toolchain:** Version 1.82 or newer (`rustup default stable`) for compiling client binaries.
- **OpenSSL:** Version 1.1.1 or 3.0+ for generating cryptographic X.509 v3 certificates.
- **Bash Shell:** Linux, macOS, or WSL2 / Git Bash on Windows.

---

## Step 1: Generate Internal Cluster PKI Certificates

All cluster communications strictly enforce Mutual TLS 1.3 (`[INV-002]`, `[NFR-SEC-001]`). Use the automated PKI generator script to construct the internal certificate authority and keys.

```bash
# Navigate to the backend repository root
cd androgrid-backend

# Ensure the PKI script is executable
chmod +x tools/pki/generate_cluster_pki.sh

# Generate the cryptographic assets
./tools/pki/generate_cluster_pki.sh ./deploy/pki node_worker_01 androgrid_secure_pki localhost
```

### Parameter Breakdown & Architectural Significance

| Argument Position | Value in Example | Description & Cryptographic Purpose |
| :--- | :--- | :--- |
| **Argument 1** | `./deploy/pki` | **Target Output Directory:** The directory where generated certificates and private keys are stored. This directory is mounted into the Podman container. |
| **Argument 2** | `node_worker_01` | **Mobile Worker Node Identifier:** The unique Common Name (CN) and client identity embedded in the worker X.509 certificate. |
| **Argument 3** | `androgrid_secure_pki` | **PKCS#12 Export Password:** The encryption passphrase used to protect `client.p12` when importing into the Android KeyStore. |
| **Argument 4** | `localhost` | **Additional SAN (Subject Alternative Name):** Domain names or IP addresses added to `server.crt` to prevent TLS hostname validation errors. |

### Generated Cryptographic Assets

```text
deploy/pki/
|-- ca.crt          # Internal Root CA certificate (distributed to all nodes as sole trust anchor)
|-- ca.key          # Root CA private key (kept secret on the host control plane, chmod 600)
|-- server.crt      # Host Orchestrator server certificate with SANs (localhost, 127.0.0.1)
|-- server.key      # Host Orchestrator private key for TLS 1.3 termination
|-- client.crt      # Worker/Operator client certificate for Mutual TLS authentication
|-- client.key      # Worker/Operator client private key
`-- client.p12      # Encrypted PKCS#12 archive for Android KeyStore provisioning
```

<div class="callout callout-tip">
  <strong>Cryptographic Curve Selection:</strong> All keys utilize NIST curve <code>prime256v1</code> (P-256 ECDSA). P-256 provides 128-bit cryptographic security equivalent to 3072-bit RSA while reducing key size by 85%, accelerating mobile TLS handshakes, and saving battery on worker devices.
</div>

---

## Step 2: Launch the Control Plane via Podman Compose

Launch the orchestrator container using Podman Compose:

```bash
cd deploy
podman compose up -d --build
```

*(Note: If using standalone `podman-compose`, execute `podman-compose up -d`)*

### Container Blueprint Breakdown (`deploy/podman-compose.yml`)

Here is what happens during initialization:
- **`build.dockerfile: Containerfile`:** Compiles `androgrid-backend` inside a multi-stage Debian Bookworm container using Rust 1.82, dropping all root privileges and running as unprivileged user `androgrid` (UID 10001).
- **`ports: ["50051:50051"]`:** Binds the gRPC transport port to host port `50051`.
- **`ANDROGRID_LISTEN_ADDR=0.0.0.0:50051`:** Instructs Tonic to bind to all network interfaces inside the container namespace.
- **`ANDROGRID_CA_CERT_PATH=/etc/androgrid/pki/ca.crt`:** Configures client authentication trust store. Any incoming connection lacking a client certificate signed by this CA is rejected during TLS 1.3 handshake.
- **`ANDROGRID_HEARTBEAT_SECS=2`:** Sets the Wi-Fi declarative state sync heartbeat window to 2.0 seconds (`[REQ-OP-002]`).
- **`ANDROGRID_LEASE_TIMEOUT_SECS=10`:** Marks worker nodes as disconnected if heartbeats lapse beyond 10 seconds.
- **`volumes: ["./pki:/etc/androgrid/pki:ro,Z"]`:** Mounts the certificates as read-only (`:ro`) with private SELinux volume relabeling (`:Z`).

### Verifying Container Health

```bash
podman ps
```

Expected output:
```text
CONTAINER ID  IMAGE                              COMMAND               STATUS                PORTS                    NAMES
8b3a7df194c0  localhost/deploy_orchestrator:latest  /usr/local/bin/an...  Up 12 seconds (healthy)  0.0.0.0:50051->50051/tcp  androgrid-orchestrator
```

The healthcheck command (`nc -z 127.0.0.1 50051`) verifies that the gRPC TLS port is listening and accepting socket handshakes.

---

## Step 3: Build and Test Operator CLI (`androgrid-cli`)

Compile the high-performance operator management binary from source:

```bash
cargo build --bin androgrid-cli --release
```

The resulting binary is located at `./target/release/androgrid-cli`.

### Query Registered Worker Nodes

Execute the `nodes` subcommand to inspect the cluster inventory:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  nodes
```

#### Detailed Flag Breakdown:
- `--endpoint https://127.0.0.1:50051`: Specifies the HTTPS/gRPC URL of the control plane orchestrator.
- `--ca-cert ./deploy/pki/ca.crt`: Supplies the internal Root CA certificate to validate the server certificate.
- `--client-cert ./deploy/pki/client.crt`: Supplies the operator client certificate to authenticate against the orchestrator.
- `--client-key ./deploy/pki/client.key`: Supplies the operator private key for TLS 1.3 client proof of possession.
- `nodes`: The command to list all registered worker nodes.

#### Expected Output:
```text
Registered Worker Nodes (0):
--------------------------------------------------------------------------------
(No mobile worker nodes currently registered. Start an Android client to connect.)
```

---

## Step 4: Package and Submit a WebAssembly Compute Workload

AndroGrid decouples large bytecode modules from unary control plane messages by indexing bytecode via content-addressed storage (`[NFR-PERF-001]`, `[NFR-POW-001]`).

### Packaging the Workload (`androgrid-cli pack`)

Validate the starter WASI module for WASI Preview 1 compatibility:

```bash
./target/release/androgrid-cli pack \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --output manifest.json
```

#### Argument Explanations:
- `--wasm <PATH>`: The path to the compiled WebAssembly bytecode binary (`.wasm`).
- `--entrypoint _start`: The exported entrypoint symbol executed inside the WASI runtime.
- `--output manifest.json`: Generates an ISO 29148 compliant deployment manifest containing the SHA-256 digest and engine requirements.

### Submitting the Workload (`androgrid-cli submit`)

Submit the task to the cluster queue:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  submit \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 30 \
  --env MATRIX_DIM=256
```

#### What Happens Under the Hood:
1. **SHA-256 Digesting:** The CLI computes the cryptographic SHA-256 checksum of the WASM bytecode.
2. **Content-Addressed Registration:** The bytecode is uploaded to the orchestrator artifact store via `AdminService.UploadArtifact`.
3. **Decoupled Task Dispatch:** The CLI dispatches a lightweight `SubmitTaskRequest` (under 1 KB) referencing `artifact_sha256` and `artifact_url`. This guarantees that cellular worker leases remain sub-4KB, preserving baseband radio sleep deadlines (`[NFR-POW-001]`).

Output:
```text
[+] Bytecode artifact uploaded to content-addressed storage
    SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    URL:     https://127.0.0.1:50051/artifacts/3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a.wasm
[+] Task successfully enqueued
    Task ID: task_7e8b91c2_f024_4b89_8d11_0234a9b51e44
```

---

## Step 5: Track Task Lifecycle and Query Results (`status`)

Query the task execution state using the returned `Task ID`:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  status task_7e8b91c2_f024_4b89_8d11_0234a9b51e44
```

### Response Field Descriptions

| Output Field | Meaning & Operational Context |
| :--- | :--- |
| `Task ID` | Canonical UUID identifying the compute task. |
| `State` | Current lifecycle state (`TASK_STATE_PENDING`, `LEASED`, `RUNNING`, `COMPLETED`, `FAILED`). |
| `Active Attempt` | Monotonic run attempt counter (`run_attempt`) protecting against stale double-commits (`[INV-004]`). |
| `Exit Code` | Process exit code (`0` for success, non-zero for runtime errors). |
| `Execution Time` | Duration spent executing the WebAssembly bytecode on the worker device in milliseconds. |
| `Stdout` | Standard output stream emitted by the WASI compute workload. |
| `Stderr` | Standard error diagnostic stream. |

---

## Summary of Next Steps

- Explore the [Operator Manual](operator/cli_manual.html) for advanced CLI configurations and JSON automation pipelines.
- Read the [Cloudflare Ingress Guide](ingress/cloudflare_tunnel.html) to expose your local Podman orchestrator to remote cellular workers without port-forwarding rules.
- Review [Security Architecture](security/architecture.html) to understand mTLS 1.3 handshakes and cellular radio sleep mechanisms.
