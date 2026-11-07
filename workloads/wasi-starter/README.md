# Canonical WebAssembly (WASI) Starter Workload

## 1. Overview
This directory contains the canonical developer starter workload for AndroGrid edge computing. It compiles to a standalone WebAssembly (`wasm32-wasip1`) binary and runs hermetically inside the unrooted Android worker execution sandbox.

Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

---

## 2. Prerequisites
Ensure the Rust WebAssembly target is installed on your host system:
```bash
rustup target add wasm32-wasip1
```

---

## 3. Compilation
Build the release WebAssembly binary:
```bash
cargo build --target wasm32-wasip1 --release
```
The compiled artifact will be located at:
`target/wasm32-wasip1/release/wasi_starter.wasm`

---

## 4. Workload Packaging
Use `androgrid-cli pack` to validate WASI Preview 1 compatibility, compute cryptographic SHA-256 integrity digests, and generate a deployment manifest:
```bash
androgrid-cli pack \
  --wasm target/wasm32-wasip1/release/wasi_starter.wasm \
  --entrypoint _start \
  --output ./manifest.json
```

Example manifest output (`manifest.json`):
```json
{
  "schema_version": "androgrid.artifact.v1",
  "name": "wasi_starter",
  "entrypoint": "_start",
  "artifact_sha256": "3a4b...",
  "size_bytes": 164210,
  "engine": "EXECUTION_ENGINE_WASM_WASI",
  "wasi_version": "preview1",
  "exports": ["_start", "memory"],
  "wasi_imports": ["wasi_snapshot_preview1::fd_write"]
}
```

---

## 5. Workload Submission & Execution
Submit the packaged WASM module directly to the cluster control plane using `androgrid-cli submit`:
```bash
androgrid-cli submit \
  --wasm target/wasm32-wasip1/release/wasi_starter.wasm \
  --timeout 30 \
  --env PRIME_SEARCH_LIMIT=5000
```
The CLI automatically uploads and indexes the artifact on the orchestrator content-addressed static server, populates `artifact_sha256` and `artifact_url`, and offloads binary payloads from in-band gRPC messages to keep control plane messages strictly sub-kilobyte (< 4 KB).

---

## 6. Observability & Result Retrieval
Track execution progress and inspect stdout outputs:
```bash
androgrid-cli status <TASK_ID>
```

---

## 7. Architectural Invariants
1. **[INV-006] Sandboxed Isolation:** Code runs inside unrooted WASM memory with strict W^X bounds.
2. **[NFR-PERF-001] Frame Boundaries:** Decoupled content-addressed artifact distribution guarantees control plane gRPC frames remain < 4 KB.
3. **[NFR-POW-001] Cellular Radio Sleep:** Offloading large bytecode binaries ensures worker connection lifetimes remain strictly under 200 ms.
