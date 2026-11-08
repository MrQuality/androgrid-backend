# Quickstart Guide: Local Cluster Deployment

This guide walks you through bootstrapping an AndroGrid control plane cluster on your local development machine using Docker Compose and testing communication with `androgrid-cli`.

Traceability: `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[ANTI-002]`, `[NFR-SEC-001]`.

---

## Prerequisites

Before starting, ensure the following utilities are installed:
- **Rust Toolchain:** Version 1.82 or newer (`rustup default stable`)
- **Docker & Docker Compose:** Docker Engine 24.0+ with Compose v2
- **OpenSSL:** Version 1.1.1 or 3.0+ for certificate generation
- **Bash:** Linux, macOS, or WSL2 / Git Bash on Windows

---

## Step 1: Generate Internal Cluster PKI

All communication requires Mutual TLS 1.3 (`[INV-002]`). Use the automated PKI generation script to build the Root CA, orchestrator server certificate, and operator client credentials:

```bash
# Navigate to the backend repository
cd androgrid-backend

# Run the PKI generator script
chmod +x tools/pki/generate_cluster_pki.sh
./tools/pki/generate_cluster_pki.sh ./deploy/pki node_worker_01 androgrid_secure_pki
```

This generates the following cryptographic assets in `deploy/pki/`:
- `ca.crt` & `ca.key`: Internal Root CA certificate and private key.
- `server.crt` & `server.key`: Orchestrator server certificate with SANs (`localhost`, `127.0.0.1`).
- `client.crt` & `client.key`: Operator client certificate for administrative control.
- `client.p12`: Android PKCS12 keystore bundle for mobile worker nodes.

---

## Step 2: Launch the Control Plane via Docker Compose

AndroGrid provides a complete production-grade Compose configuration in `deploy/docker-compose.yml`:

```bash
cd deploy
docker compose up -d --build
```

Verify that the orchestrator container is running and healthy:

```bash
docker compose ps
```

Expected output:
```text
NAME                     IMAGE                      COMMAND                  SERVICE        STATUS
androgrid-orchestrator   androgrid-orchestrator     "/usr/local/bin/andr…"   orchestrator   Up (healthy)
```

The orchestrator listens on port `50051` with strict mTLS 1.3 enabled (`[NFR-SEC-001]`).

---

## Step 3: Verify Cluster Status with `androgrid-cli`

Build the operator CLI binary:

```bash
cargo build --bin androgrid-cli --release
```

Query the list of registered worker nodes using the generated certificates:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  nodes
```

For JSON automation pipelines, append `--format json`:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  --format json \
  nodes
```

---

## Step 4: Submit a WebAssembly Task

Submit a compute task targeting the WebAssembly sandbox (`[INV-006]`):

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  submit \
  --wasm ./workloads/wasi-starter/target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 30
```

Check the status of the submitted task:

```bash
./target/release/androgrid-cli \
  --endpoint https://127.0.0.1:50051 \
  --ca-cert ./deploy/pki/ca.crt \
  --client-cert ./deploy/pki/client.crt \
  --client-key ./deploy/pki/client.key \
  status <TASK_ID>
```

---

## Next Steps

- Consult the [Operator Manual](operator/cli_manual.md) for full CLI parameter specifications.
- Configure remote WAN cellular ingress via the [Cloudflare Tunnel Guide](ingress/cloudflare_tunnel.md).
