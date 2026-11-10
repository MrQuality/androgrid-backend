# ISO/IEC/IEEE 29148 Requirement Traceability Matrix

**Specification Baseline:** AG-SPEC-2026-V2 (`AndroGrid_SDD.md`)  
**Standard Compliance:** ISO/IEC/IEEE 29148:2018 (Systems and software engineering - Life cycle processes - Requirements engineering)  
**Verification Baseline:** Automated Rust Test Suites, Property Tests (proptest), Section 8 Normative BDD Scenarios (`cucumber` / `tonic`)  
**Typographic Standard:** Zero Unicode em dashes (U+2014) or en dashes (U+2013). Strict ASCII hyphen (-) conformance.

---

## 1. Executive Summary and Architecture Governance

This Traceability Matrix establishes bidirectional verification traceability between the normative requirements defined in `AG-SPEC-2026-V2` and the production codebase of the AndroGrid Host Control Plane (`androgrid-backend`).

In accordance with ISO/IEC/IEEE 29148:2018:
1. **Forward Traceability:** Every functional requirement (`[REQ-...]`), architectural invariant (`[INV-...]`), proscribed anti-requirement (`[ANTI-...]`), and non-functional quality attribute (`[NFR-...]`) traces forward to concrete source modules, structs, and methods.
2. **Backward Traceability:** Every test suite, property test, integration test, and formal docstring links backward to its governing requirement identifier.
3. **Formal Consensus:** Architecture Decision Records (`docs/adr/`) document the architectural rationale, rejected alternatives, and operational trade-offs for all foundational design choices.

---

## 2. Requirements Traceability Matrix

| Requirement ID | Requirement Title & Normative Scope | Master Spec Ref | Host Control Plane Source Implementation | Verification Test Suites | Normative Section 8 BDD Scenario & ADR |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `[REQ-OP-001]` | **Node Registration (`RegisterNode`)**<br>Validates client TLS 1.3 cert, generates session token, records capabilities, returns lease timeouts within 500 ms. | Section 4.1<br>Section 5.1 | `src/transport/service.rs`<br>`src/scheduler/engine.rs`<br>`src/scheduler/state.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `tests/bdd_acceptance_spec.rs`<br>`tests/proptest_scheduler.rs`<br>`tests/spike_grpc_mtls_e2e.rs`<br>`tests/cluster_observability_spec.rs` | Section 8 Scenario 3: Sandbox Guard<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md`<br>`docs/adr/ADR-005-interface-segregation-admin-vs-worker.md` |
| `[REQ-OP-002]` | **Declarative Wi-Fi State Sync (`SyncNodeState`)**<br>2.0-second declarative reconciliation loop over Wi-Fi transmitting active and completed task states. | Section 4.1<br>Section 5.1<br>Section 6.1 | `src/transport/service.rs`<br>`src/scheduler/engine.rs`<br>`src/scheduler/state.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `tests/bdd_acceptance_spec.rs`<br>`tests/cluster_observability_spec.rs`<br>`tests/operator_cli_integration_spec.rs` | Section 8 Scenario 2: Spot Preemption<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[REQ-OP-003]` | **Disconnected Cellular Task Claim (`ClaimCellularTask`)**<br>Isolated unary task lease over Cloudflare Anycast ingress for spot workloads (duration <= 180s, unmetered=false). | Section 4.1<br>Section 5.1<br>Section 6.2 | `src/transport/service.rs`<br>`src/scheduler/engine.rs`<br>`src/scheduler/thermal.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `tests/bdd_acceptance_spec.rs`<br>`tests/spike_wire_parity.rs`<br>`tests/spike_grpc_mtls_e2e.rs`<br>`tests/content_addressed_artifact_spec.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md`<br>`docs/adr/ADR-004-speculative-preemption-dual-assignment.md` |
| `[REQ-OP-004]` | **Atomic Result Submission (`SubmitCellularResult`)**<br>Monotonic attempt fencing and atomic result persistence. Rejects stale submissions with FAILED_PRECONDITION. | Section 4.1<br>Section 5.1<br>Section 6.2 | `src/transport/service.rs`<br>`src/scheduler/engine.rs`<br>`src/lib.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `tests/bdd_acceptance_spec.rs`<br>`tests/spike_concurrency_cas.rs`<br>`tests/spike_wire_parity.rs`<br>`tests/proptest_scheduler.rs` | Section 8 Scenario 1 & 2<br>`docs/adr/ADR-003-monotonic-attempt-fencing-cas.md`<br>`docs/adr/ADR-005-interface-segregation-admin-vs-worker.md` |
| `[INV-001]` | **Zero Inbound Mobile Ports**<br>Mobile worker nodes must not expose listening sockets. All network traffic originates outbound from the worker toward orchestrator. | Section 3.1<br>Section 2.2 | `src/transport/service.rs`<br>`src/transport/admin_service.rs`<br>`src/main.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/spike_grpc_mtls_e2e.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[INV-002]` | **Mutual TLS 1.3 Enforcement**<br>All communications across LAN and WAN strictly authenticated via X.509 certificates with ECDSA NIST P-256 keys. | Section 3.1<br>Section 2.3<br>Section 7 | `src/transport/tls.rs`<br>`src/pairing/service.rs`<br>`src/pairing/signer.rs`<br>`src/config.rs` | `tests/spike_grpc_mtls_e2e.rs`<br>`tests/ephemeral_pairing_spec.rs`<br>`tests/bdd_acceptance_spec.rs` | Section 8 Live mTLS Test Harness<br>`docs/adr/ADR-001-mutual-tls-1.3-ecdsa-p256.md` |
| `[INV-003]` | **Zero Cellular Keepalive & Radio-Sleep Guard**<br>gRPC channel actively torn down upon task claim response, permitting mobile baseband processor to enter low-power RRC sleep. | Section 3.1<br>Section 2.3<br>Section 7 | `src/transport/service.rs`<br>`src/scheduler/engine.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/documentation_portal_spec.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[INV-004]` | **Monotonic Attempt Fencing**<br>Atomic fencing counter (run_attempt). Prevents stale duplicate commits from overwriting committed results. | Section 3.1<br>Section 5.1<br>Section 6.2 | `src/scheduler/engine.rs`<br>`src/scheduler/state.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/spike_concurrency_cas.rs`<br>`tests/proptest_scheduler.rs` | Section 8 Scenario 2: Spot Preemption<br>`docs/adr/ADR-003-monotonic-attempt-fencing-cas.md` |
| `[INV-005]` | **Cellular Spot Preemption & Dual-Assignment**<br>Opportunistic dual-assignment of active cellular task to newly associated Wi-Fi worker with run_attempt increment. | Section 3.1<br>Section 6.2 | `src/scheduler/engine.rs`<br>`src/scheduler/state.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/spike_concurrency_cas.rs` | Section 8 Scenario 2: Spot Preemption<br>`docs/adr/ADR-004-speculative-preemption-dual-assignment.md` |
| `[INV-006]` | **Sandbox Isolation Boundary**<br>Unrooted worker nodes constrained strictly to in-process WASI execution. Refuses dispatch of root shell execution payloads. | Section 3.1<br>Section 4.1 | `src/scheduler/engine.rs`<br>`src/artifact/wasi_validator.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `tests/bdd_acceptance_spec.rs`<br>`tests/proptest_scheduler.rs` | Section 8 Scenario 3: Sandbox Guard |
| `[ANTI-001]` | **Proscribed Cellular Keepalive**<br>System shall not maintain persistent socket connections or ping/pong telemetry across metered cellular networks. | Section 3.2 | `src/transport/service.rs`<br>`src/scheduler/engine.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/documentation_portal_spec.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[ANTI-002]` | **Proscribed Inbound Host Port-Forwarding**<br>Deployment architecture shall not require inbound NAT port forwarding, public static IPs, or client VPN tunnels. | Section 3.2<br>Section 2.2 | `deploy/docker-compose.yml`<br>`docs/src/ingress/cloudflare_tunnel.md` | `tests/documentation_portal_spec.rs` | `docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[ANTI-003]` | **Proscribed Unmetered/Long Payloads on Cellular**<br>Orchestrator shall not dispatch unmetered-only tasks or tasks > 180s to metered cellular connections. | Section 3.2<br>Section 5.1 | `src/scheduler/engine.rs`<br>`src/scheduler/thermal.rs` | `src/scheduler/engine.rs`<br>`tests/bdd_acceptance_spec.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-004-speculative-preemption-dual-assignment.md` |
| `[NFR-SEC-001]` | **Cryptographic Transport Security**<br>TLS 1.3 cipher suites (TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384) with mandatory mutual certificate validation. | Section 7 | `src/transport/tls.rs`<br>`src/config.rs`<br>`src/pairing/signer.rs` | `tests/spike_grpc_mtls_e2e.rs`<br>`tests/ephemeral_pairing_spec.rs`<br>`tests/bdd_acceptance_spec.rs` | `docs/adr/ADR-001-mutual-tls-1.3-ecdsa-p256.md`<br>`docs/adr/ADR-005-interface-segregation-admin-vs-worker.md` |
| `[NFR-POW-001]` | **Baseband Radio Efficiency**<br>Socket termination within 200 ms of ClaimCellularTaskResponse receipt to enable fast RRC tail drop into low-power sleep. | Section 7<br>Section 3.1 | `src/transport/service.rs`<br>`src/scheduler/engine.rs` | `tests/bdd_acceptance_spec.rs`<br>`tests/documentation_portal_spec.rs` | Section 8 Scenario 1: Cellular Disconnected<br>`docs/adr/ADR-002-dual-route-protocol-demarcation.md` |
| `[NFR-PERF-001]` | **Message Frame Bounds (16 MB)**<br>Upper bound of 16 MB strictly enforced on gRPC buffers and content-addressed artifact payloads. | Section 7<br>Section 2.3 | `src/lib.rs`<br>`src/artifact/store.rs`<br>`src/transport/service.rs` | `tests/spike_wire_parity.rs`<br>`tests/distributed_image_benchmark_spec.rs`<br>`src/artifact/store.rs` | `docs/src/api/protobuf.md` |
| `[NFR-REL-001]` | **Monotonic Consensus Reliability**<br>Zero data corruption or double-commit under concurrent submissions through atomic Compare-and-Swap state transitions. | Section 7<br>Section 3.1 | `src/scheduler/engine.rs`<br>`src/scheduler/state.rs` | `tests/spike_concurrency_cas.rs`<br>`tests/proptest_scheduler.rs`<br>`tests/bdd_acceptance_spec.rs` | Section 8 Scenario 2: Spot Preemption<br>`docs/adr/ADR-003-monotonic-attempt-fencing-cas.md` |
| `[REQ-VER-001]` | **Typed Foreground Service (API 34+)**<br>Worker client passes FOREGROUND_SERVICE_TYPE_DATA_SYNC to startForeground on Android 14+ devices. | Appendix A.2<br>Appendix A.3 | `docs/src/android/architecture.md`<br>`docs/src/android/worker_guide.md` | `tests/documentation_portal_spec.rs` | Android Architectural Spec |
| `[REQ-VER-002]` | **Dynamic Capability Reporting**<br>Worker supplies exact Build.VERSION.RELEASE and primary ABI in NodeCapabilities for bytecode compatibility. | Appendix A.3<br>Section 4.1 | `src/transport/service.rs`<br>`src/scheduler/engine.rs`<br>`proto/androgrid/v1/orchestrator.proto` | `src/lib.rs`<br>`tests/bdd_acceptance_spec.rs` | Section 8 Scenario 3: Sandbox Guard |
| `[REQ-VER-003]` | **W^X Native Sandbox Compatibility**<br>Embedded WASI runtime executes entirely in memory without writing or executing shared objects on writable storage. | Appendix A.2<br>Appendix A.3 | `src/artifact/wasi_validator.rs`<br>`docs/src/workloads/compilation_guide.md` | `tests/distributed_image_benchmark_spec.rs`<br>`tests/documentation_portal_spec.rs` | Workload Compilation Guide |

---

## 3. Section 8 Normative Gherkin Acceptance Verification

The three normative Gherkin acceptance features defined in Section 8 of `AG-SPEC-2026-V2` are implemented as automated integration tests in `tests/bdd_acceptance_spec.rs`:

### Scenario 1: Cellular Disconnected Execution and Zero-Keepalive Radio Sleep
- **Traceability:** `[REQ-OP-003]`, `[REQ-OP-004]`, `[INV-001]`, `[INV-003]`, `[ANTI-001]`, `[NFR-POW-001]`
- **Test Implementation:** `tests/bdd_acceptance_spec.rs::test_bdd_scenario_1_cellular_disconnected_lifecycle`
- **Assertion Criteria:**
  1. Worker claims task over unary endpoint.
  2. Orchestrator issues spot task with 120s timeout and deadline.
  3. Client immediately closes gRPC channel and underlying socket.
  4. Client completes local WASI execution offline.
  5. Client connects via fresh socket to submit result with `run_attempt = 1`.
  6. Orchestrator accepts result and marks task `TASK_STATE_COMPLETED`.

### Scenario 2: Cellular Spot Dual-Assignment and Monotonic Fencing
- **Traceability:** `[REQ-OP-002]`, `[REQ-OP-004]`, `[INV-004]`, `[INV-005]`, `[NFR-REL-001]`
- **Test Implementation:** `tests/bdd_acceptance_spec.rs::test_bdd_scenario_2_spot_dual_assignment_and_monotonic_fencing`
- **Assertion Criteria:**
  1. Cellular worker claims task with `run_attempt = 1`.
  2. Idle Wi-Fi worker associates; orchestrator speculatively reassigns same task with `run_attempt = 2`.
  3. Wi-Fi worker completes execution first and submits result with `run_attempt = 2`.
  4. Orchestrator commits task to `TASK_STATE_COMPLETED`.
  5. Slower cellular worker subsequently submits result with `run_attempt = 1`.
  6. Orchestrator rejects stale cellular submission with `FAILED_PRECONDITION` and error code `TASK_ATTEMPT_STALE`.

### Scenario 3: Sandbox Isolation Guard for Unrooted Nodes
- **Traceability:** `[REQ-OP-001]`, `[INV-006]`, `[REQ-VER-002]`
- **Test Implementation:** `tests/bdd_acceptance_spec.rs::test_bdd_scenario_3_sandbox_isolation_guard`
- **Assertion Criteria:**
  1. Unrooted worker registers with `is_rooted = false`.
  2. Orchestrator filters available execution engines strictly to `EXECUTION_ENGINE_WASM_WASI`.
  3. Orchestrator rejects scheduling of `EXECUTION_ENGINE_ROOT_SHELL` tasks to unrooted nodes.

---

## 4. Control Plane Architecture Decision Records (ADRs)

| ADR Identifier | Title & Architecture Decision | Status | Master Traceability |
| :--- | :--- | :--- | :--- |
| `ADR-001` | **Mutual TLS 1.3 with ECDSA P-256 for Mobile Hardware Crypto Accelerators**<br>Mandates TLS 1.3 with NIST P-256 keys to maximize mobile hardware acceleration and prevent MITM attacks. | Accepted | `[INV-002]`, `[NFR-SEC-001]`, `[REQ-OP-001]` |
| `ADR-002` | **Dual-Route Protocol Demarcation (Wi-Fi Loop vs Disconnected Cellular WAN)**<br>Bifurcates transport lifecycle: persistent Wi-Fi sync vs disconnected zero-keepalive cellular unary calls to preserve mobile battery. | Accepted | `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[INV-001]`, `[INV-003]`, `[ANTI-001]`, `[NFR-POW-001]` |
| `ADR-003` | **Monotonic Attempt Fencing and Lock-Free Compare-and-Swap (CAS) State Consensus**<br>Applies atomic attempt fencing and CAS transitions to prevent duplicate commits and race conditions without distributed locks. | Accepted | `[INV-004]`, `[NFR-REL-001]`, `[REQ-OP-004]` |
| `ADR-004` | **Speculative Preemption and Dual-Assignment for Cellular Spot Leases**<br>Speculatively reassigns running cellular tasks to faster Wi-Fi workers with incremented attempt tokens to minimize cluster tail latency. | Accepted | `[INV-005]`, `[ANTI-003]`, `[REQ-OP-003]`, `[INV-004]` |
| `ADR-005` | **Interface Segregation Principle (ISP) for Operator AdminService vs Worker AndroGridService**<br>Segregates administrative control APIs from mobile worker execution interfaces to enforce least privilege and minimize attack surface. | Accepted | `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[NFR-SEC-001]` |
