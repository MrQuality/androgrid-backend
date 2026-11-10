# AndroGrid Ecosystem Architecture & Navigation

Traces to: `[REQ-PUB-001]`, `[REQ-OP-001]`, `[REQ-OP-003]`, `[INV-001]`, `[INV-002]`

The **AndroGrid Ecosystem** is a distributed, high-concurrency mobile edge-computing platform organized across three foundational pillars: the Host Control Plane, the Native Android Worker Client, and the ISO 29148 System Design Specification.

---

## 1. Architectural Topology & Ecosystem Map

The diagram below illustrates the end-to-end component topology connecting the control plane, ingress gateways, and heterogeneous Android worker nodes:

```mermaid
flowchart TD
    subgraph HostControlPlane ["Host Control Plane (androgrid-backend)"]
        Orchestrator["Tonic gRPC Orchestrator (Port 50051)"]
        Scheduler["Dual-Assignment Scheduler (INV-004 / INV-005)"]
        CASStorage["Compare-And-Swap State Consensus"]
        CASArtifact["Content-Addressed Artifact Server (/artifacts)"]
        WebDashboard["Embedded Observability Dashboard (Port 8080)"]
        CLI["androgrid-cli (Operator & Dev Tool)"]
    end

    subgraph IngressGateway ["Ingress & Boundary Layer"]
        LAN["Wi-Fi LAN (Direct HTTP/2 mTLS 1.3)"]
        CF["Cloudflare Anycast Tunnel (Cellular WAN Ingress)"]
    end

    subgraph MobileWorkers ["Mobile Worker Nodes (androgrid-android)"]
        Pixel["Google Pixel 7 Worker (Wi-Fi Unmetered)"]
        Galaxy["Samsung Galaxy S22 Worker (Cellular WAN)"]
        Chicory["In-Process Chicory / Wasmtime WASI Sandbox"]
        Thermal["ThermalSafetyGuard & Battery Monitor"]
    end

    subgraph NormativeSpec ["Normative Architecture (AndroGrid_SDD.md)"]
        Spec["AG-SPEC-2026-V2 (ISO/IEC/IEEE 29148)"]
    end

    CLI -->|gRPC Admin API| Orchestrator
    Orchestrator --- Scheduler
    Scheduler --- CASStorage
    Orchestrator --- CASArtifact
    WebDashboard --- CASStorage

    LAN -->|Declarative Sync (REQ-OP-002)| Orchestrator
    CF -->|Zero-Keepalive Unary (REQ-OP-003)| Orchestrator

    Pixel -->|mTLS 1.3 (INV-002)| LAN
    Galaxy -->|mTLS 1.3 (NFR-POW-001)| CF

    Pixel --- Chicory
    Galaxy --- Chicory
    Pixel --- Thermal
    Galaxy --- Thermal

    HostControlPlane -.->|Traceability Verification| Spec
    MobileWorkers -.->|State Matrix Adherence| Spec
```

---

## 2. Core Repository Mapping

### Pillar 1: Host Control Plane (`androgrid-backend`)
- **Primary Repository:** [github.com/MrQuality/androgrid-backend](https://github.com/MrQuality/androgrid-backend)
- **Role:** High-concurrency task orchestrator, scheduling coordinator, artifact distributor, and operator CLI.
- **Technology Stack:** Rust 1.82+, Tokio 1.40 asynchronous runtime, Tonic 0.12 gRPC framework, Rustls mutual TLS 1.3, and Ratatui terminal dashboard.
- **Key Modules:**
  - `src/scheduler/`: Implements monotonic attempt fencing (`[INV-004]`) and speculative spot preemption (`[INV-005]`).
  - `src/artifact/`: SHA-256 content-addressed HTTP storage server for WASM bytecode caching (`[NFR-POW-001]`).
  - `src/transport/`: Tonic gRPC service implementations for `AndroGridService` and `AdminService`.
  - `src/bin/androgrid-cli.rs`: Unified command-line interface for packaging, submitting, and tracking tasks.
- **Documentation:** [System Architecture Overview](../architecture/system_overview.md) and [Operator Manual](../operator/cli_manual.md).

### Pillar 2: Mobile Worker Client (`androgrid-android`)
- **Primary Repository:** [github.com/MrQuality/androgrid-android](https://github.com/MrQuality/androgrid-android)
- **Role:** Native Android worker service executing sandboxed WebAssembly compute payloads across Wi-Fi and Cellular WAN.
- **Technology Stack:** Kotlin 1.9+, Android SDK (API 28 to 34), Jetpack WorkManager, Typed Foreground Services (`dataSync`), Chicory zero-dependency WASM interpreter, and OkHttp / gRPC-Kotlin.
- **Key Subsystems:**
  - `com.androgrid.worker.service`: Typed Foreground Service and WorkManager spot worker lifecycle.
  - `com.androgrid.worker.telemetry`: ThermalSafetyGuard enforcing 38 C cutoff and 35 C resume hysteresis.
  - `com.androgrid.worker.network`: Zero-keepalive cellular socket lifecycle with 200 ms radio teardown (`[NFR-POW-001]`).
  - `com.androgrid.worker.ui`: Jetpack Compose real-time worker dashboard and QR scanner.
- **Interactive Dokka API Portal:** [mrquality.github.io/androgrid-android/dokka](https://mrquality.github.io/androgrid-android/dokka/index.html)
- **Documentation:** [Android Worker Onboarding Guide](../android/worker_guide.md) and [Android Client Architecture](../android/architecture.md).

### Pillar 3: Master Specification & Invariants (`AndroGrid_SDD.md`)
- **Document Identifier:** `AG-SPEC-2026-V2` (ISO/IEC/IEEE 29148 compliant)
- **Role:** Canonical architectural specification governing all wire contracts, state transitions, invariants, and security boundaries.
- **Core Invariants Enforced:**
  - `[INV-001]` Zero Inbound Mobile Ports
  - `[INV-002]` Mutual TLS 1.3 Bidirectional Authentication
  - `[INV-003]` Zero Cellular Keepalive & RRC Radio Sleep
  - `[INV-004]` Monotonic Attempt Fencing
  - `[INV-005]` Cellular Spot Preemption & Dual-Assignment
  - `[INV-006]` WebAssembly/WASI Sandbox Boundary

---

## 3. Quick Navigation & Ecosystem Links

| Resource | Purpose | Target URL |
| :--- | :--- | :--- |
| **Backend Source Code** | Control Plane, Orchestrator, CLI | [github.com/MrQuality/androgrid-backend](https://github.com/MrQuality/androgrid-backend) |
| **Android Worker Client** | Native Android application | [github.com/MrQuality/androgrid-android](https://github.com/MrQuality/androgrid-android) |
| **Interactive Kotlin Dokka** | Android Client API reference | [mrquality.github.io/androgrid-android/dokka/index.html](https://mrquality.github.io/androgrid-android/dokka/index.html) |
| **Protobuf API Reference** | Canonical gRPC service schema | [Protobuf API Reference](../api/protobuf.md) |
| **Container Images** | Pre-built rootless OCI container | `ghcr.io/mrquality/androgrid-orchestrator:latest` |
| **Starter Workload** | Rust wasm32-wasip1 starter project | [workloads/wasi-starter/README.md](https://github.com/MrQuality/androgrid-backend/tree/main/workloads/wasi-starter) |
| **Hardware Feasibility FAQ** | Battery longevity and thermal metrics | [Mobile Economics & Battery FAQ](economics_faq.md) |
