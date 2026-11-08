# System Overview and Master Architecture

The **AndroGrid Control Plane** is a distributed coordination platform developed in Rust, engineered to harness heterogeneous Android mobile devices as edge-compute workers while operating under stringent physical constraints: power preservation, thermal boundaries, and network variability.

Traceability: `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-OP-004]`, `[INV-001]`, `[INV-002]`, `[INV-003]`, `[INV-004]`, `[INV-005]`, `[INV-006]`.

---

## Dual-Route Architecture: Wi-Fi LAN vs Cellular WAN

The cluster divides network operations into two distinct interaction planes:

```mermaid
flowchart TD
    subgraph ControlPlane["Host Control Plane (Rust Orchestrator)"]
        Scheduler["Thermal-Aware Scheduler & CAS State Engine"]
        ArtifactStore["Content-Addressed Artifact Store (SHA-256)"]
        TonicServer["Tonic gRPC Server (TLS 1.3 mTLS)"]
    end

    subgraph LocalLAN["Local Wi-Fi LAN (Unmetered)"]
        WiFiWorker["Android Worker Node (Wi-Fi)<br/>Persistent HTTP/2 State Sync"]
    end

    subgraph CellularWAN["Cellular WAN (Metered)"]
        Cloudflare["Cloudflare Anycast Edge Tunnel (cloudflared)"]
        CellWorker["Android Worker Node (Cellular)<br/>Zero-Keepalive Unary Spot Claim"]
    end

    WiFiWorker <-->|"Direct mTLS 1.3 (Port 50051)<br/>SyncNodeState (Every 2.0s)"| TonicServer
    CellWorker -->|"Outbound mTLS over HTTPS<br/>ClaimCellularTask"| Cloudflare
    Cloudflare -->|"HTTP/2 Origin Multiplexing"| TonicServer
    Scheduler <--> ArtifactStore
    TonicServer <--> Scheduler
```

### 1. Wi-Fi Persistent Declarative Surface (Unmetered LAN)
- **High-Bandwidth, Low-Latency:** When worker nodes are connected via local Wi-Fi, they communicate directly with the orchestrator without proxy overhead.
- **Node Registration (`[REQ-OP-001]`):** Nodes submit hardware metrics (CPU cores, RAM, ABI, root state, thermal status). The orchestrator validates capabilities and grants a lease session.
- **Declarative Synchronization (`[REQ-OP-002]`):** Workers execute a periodic 2.0-second synchronization loop (`SyncNodeState`), reporting execution states and receiving desired task specs.

### 2. Cellular Disconnected Unary Surface (Metered WAN)
- **Outbound-Only Ingress (`[INV-001]`):** Cellular workers never listen on server ports. All requests originate outbound.
- **Zero-Keepalive Unary Leases (`[INV-003]`, `[NFR-POW-001]`):** Cellular devices do not maintain persistent streams or heartbeats. A worker invokes `ClaimCellularTask` as a single unary RPC and immediately terminates the TCP socket within 200 ms to return the cellular modem to low-power Radio Resource Control (RRC) sleep.
- **Spot Lease Cap (`[INV-005]`):** Tasks claimed over cellular WAN are granted with a strict execution deadline capped at 180 seconds.

---

## Monotonic Consensus and Compare-And-Swap (CAS) Engine

In distributed mobile edge computing, workers experience latency spikes, radio drops, and spot preemption. To prevent double-commit races and stale execution commits, AndroGrid implements **Monotonic Attempt Fencing** (`[INV-004]`).

```mermaid
sequenceDiagram
    autonumber
    participant Orch as Host Orchestrator (CAS Engine)
    participant Cell as Cellular Worker (Attempt 1)
    participant WiFi as Wi-Fi Worker (Attempt 2)

    Note over Cell,Orch: Cellular Worker claims task (Attempt 1)
    Cell->>Orch: ClaimCellularTaskRequest (Node A)
    Orch-->>Cell: ClaimCellularTaskResponse (Task X, run_attempt=1)
    Note over Cell: Application closes socket < 200ms
    Note over Cell: Executing WASI payload locally...

    Note over Orch,WiFi: Wi-Fi worker connects; Orchestrator activates Attempt 2
    WiFi->>Orch: SyncNodeStateRequest (Node B)
    Orch->>Orch: Bump active_attempt: 1 -> 2
    Orch-->>WiFi: SyncNodeStateResponse (Task X, run_attempt=2)
    Note over WiFi: Executing WASI payload...

    Note over Cell,Orch: Cellular worker finishes Attempt 1
    Cell->>Orch: SubmitCellularResultRequest (Task X, run_attempt=1)
    Orch->>Orch: Fencing Check: submitted (1) < active (2)
    Orch-->>Cell: SchedulingError::AttemptStale (TASK_ATTEMPT_STALE)

    Note over WiFi,Orch: Wi-Fi worker completes Attempt 2
    WiFi->>Orch: SyncNodeStateRequest (Task X completed, run_attempt=2)
    Orch->>Orch: Fencing Check: submitted (2) == active (2)
    Orch->>Orch: CAS update state: RUNNING -> COMPLETED
    Orch-->>WiFi: SyncNodeStateResponse (acknowledged=Task X, accepted=true)
```

### Atomic Monotonic Fencing Rules (`[INV-004]`)
1. Every task holds a monotonic `active_attempt` counter starting at 1.
2. If a cellular worker claims Task X (`run_attempt = 1`), `active_attempt` is initialized to 1.
3. When an unmetered Wi-Fi worker connects, the orchestrator may speculatively dual-assign or preempt Task X to the Wi-Fi node, monotonically incrementing `active_attempt` to 2 and dispatching `run_attempt = 2`.
4. **Monotonic Fencing Rule (`src/scheduler/engine.rs`):** Any incoming submission bearing `run_attempt < active_attempt` is immediately rejected with `TASK_ATTEMPT_STALE`. Because activating attempt 2 sets `active_attempt = 2`, the cellular worker's attempt 1 cannot commit.
5. Once a valid submission matching `active_attempt` arrives, the orchestrator commits an atomic Compare-And-Swap (CAS) transition from `RUNNING` (or `PREEMPTED`) to `COMPLETED` or `FAILED`.
6. Any submission arriving after a task is finalized in `Completed` or `Failed` state is rejected with `AlreadyFinalized`, guaranteeing exactly-once finalization and zero duplicate executions (`[NFR-REL-001]`).

---

## WebAssembly and WASI Execution Boundary

To ensure safe computation across unrooted consumer devices:
- **Sandbox Isolation Boundary (`[INV-006]`):** Unrooted devices are strictly restricted to WebAssembly execution (`EXECUTION_ENGINE_WASM_WASI`). Shell execution (`EXECUTION_ENGINE_ROOT_SHELL`) is never assigned to unrooted workers.
- **In-Memory Pure-JVM Execution (`[REQ-VER-003]`):** Android 10+ devices enforce W^X (Write XOR Execute) memory policies on application directories. The Android worker embeds the Chicory pure-JVM WebAssembly and WASI Preview 1 interpreter, executing bytecode entirely in-memory without compiling or emitting native `.so` binaries on writable app storage.
- **Decoupled Artifact Distribution:** WebAssembly bytecode is distributed via content-addressed SHA-256 URLs, reducing unary gRPC metadata to under 4 KB and preserving cellular radio sleep deadlines (`[NFR-POW-001]`).
