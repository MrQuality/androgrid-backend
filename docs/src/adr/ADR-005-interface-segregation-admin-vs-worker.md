# ADR-005: Interface Segregation Principle (ISP) for Operator AdminService vs Worker AndroGridService

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-15
- **Traceability:** `[REQ-OP-001]`, `[REQ-OP-004]`, `[INV-002]`, `[NFR-SEC-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
A distributed compute cluster serves two diametrically opposed constituencies:
1. **Cluster Operators & Automation Pipelines:** Human administrators, CLI tools (`androgrid-cli`), and CI/CD pipelines that inject tasks, inspect global node states, cancel running jobs, and issue device pairing tokens.
2. **Mobile Worker Nodes:** Untrusted or semi-trusted Android edge clients running across diverse network environments that register capabilities, pull tasks, and submit execution results.

Combining administrative control operations and mobile worker lifecycle methods into a monolithic gRPC service contract introduces severe architectural and security risks:
- **Violates Interface Segregation Principle (ISP):** Clients are forced to depend on RPC methods they do not execute. Mobile worker stubs would compile against task injection and cancellation methods.
- **Privilege Escalation Risk:** A rogue or compromised Android worker node could attempt to invoke administrative methods (such as canceling other workers' tasks or generating unauthorized pairing tokens) unless complex runtime role-based access control (RBAC) interceptors are added.
- **Contract Drift and SDK Bloat:** Changes to administrative tooling would force version updates and re-generation of client stubs on mobile worker applications.

---

## Decision
We apply the **Interface Segregation Principle (ISP)** by formally bifurcating the gRPC control plane into two independent, decoupled services:
1. **`AndroGridService` (Worker Execution Surface):**
   - Purely dedicated to edge worker lifecycle: `RegisterNode`, `SyncNodeState`, `ClaimCellularTask`, and `SubmitCellularResult`.
   - Exposes minimal API footprint tailored specifically to outbound-only worker operations.
   - Compiled into the mobile client application (`androgrid-android`) as a lightweight client stub.
2. **`AdminService` (Operator Administrative Surface):**
   - Dedicated to administrative command and control: `SubmitTask`, `GetTaskStatus`, `ListTasks`, `CancelTask`, `ListWorkerNodes`, `UploadArtifact`, and `GeneratePairingToken`.
   - Exclusively consumed by operator tools (`androgrid-cli`, management dashboards, and integration test harnesses).
   - Enforces dedicated authentication and TLS transport boundaries isolated from public mobile worker traffic.

### Architectural Rationale
- **Principle of Least Privilege:** Untrusted edge workers have zero visibility or client-side interface access to management functions, reducing attack surface at compile-time and protocol-time.
- **Independent Lifecycle & Evolution:** Administrative RPC contracts can evolve, add observability features, or alter operator schemas without breaking or requiring re-deployment of mobile worker client binaries.
- **Clean Rust Implementation:** `AndroGridServiceImpl` and `AdminServiceImpl` independently wrap the shared atomic `ClusterScheduler` without coupling network handling logic.

---

## Consequences

### Positive
- Strict compile-time and protocol-level separation of administrative capabilities from mobile worker clients.
- Clean SOLID-compliant architecture adhering to ISO/IEC/IEEE 29148 modularity guidelines.
- Mobile client APK sizes remain minimal by avoiding unused administrative Protobuf definitions.

### Trade-offs
- The orchestrator runtime must host two distinct gRPC service endpoints (either on separate ports or co-located under distinct Tonic routing rules).

---

## Alternatives Considered
- **Monolithic Single Service (`AndroGridService` containing all RPCs):** Rejected due to violation of ISP, swollen mobile client stubs, and elevated security risk of privilege escalation.
- **Separate REST / OpenAPI Surface for Administration:** Rejected because Protobuf and gRPC provide type safety, high-performance binary serialization, streaming observability, and shared schema contracts across Rust CLI and daemon tools.
