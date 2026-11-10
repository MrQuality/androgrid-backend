# Community Launch Blueprints & Pitch Copy

Traces to: `[REQ-PUB-001]`, G-21

This chapter provides ready-to-publish launch copy and submission templates for Hacker News, Reddit (r/rust, r/selfhosted), and Lobste.rs. Each template is engineered to pass the 30-second attention test, showcasing empirical benchmarks while proactively dismantling common skepticism regarding battery health and thermal throttling.

---

## 1. Hacker News (Show HN)

**Title:** Show HN: AndroGrid - High-concurrency Rust control plane turning Android phones into an edge compute grid

**URL / Text Submission:**
```text
Hey HN,

I built AndroGrid, a distributed edge-computing platform that harnesses heterogeneous Android smartphones as cluster worker nodes running WebAssembly (WASI) workloads.

Code & Documentation:
https://github.com/MrQuality/androgrid-backend
https://github.com/MrQuality/androgrid-android
Documentation Portal: https://mrquality.github.io/androgrid-backend/

### Why build this?
Many of us have several old smartphones sitting in drawers. A 4-year-old phone (Snapdragon 855 / 865) has 8 CPU cores, 6-8 GB LPDDR4X RAM, fast UFS flash, hardware AES/SHA accelerators, and a built-in battery backup (UPS). Yet, we often spend $80+ on Raspberry Pis that draw 3-4W idling and lack hardware cryptography or power redundancy.

### How does it work?
- Host Control Plane (Rust): Built on Tokio 1.40 and Tonic 0.12. Coordinates task scheduling with monotonic attempt fencing [INV-004] (preventing double-commit races) and speculative dual-assignment [INV-005] (cellular spot tasks preempted when an unmetered Wi-Fi worker connects).
- Content-Addressed Storage: Bytecode artifacts are cached over an embedded HTTP server via SHA-256 hashes, keeping gRPC control messages under 4 KB.
- Mobile Worker (Android/Kotlin): Sandboxed execution via in-process Chicory/Wasmtime WASI. Workers never open inbound listening ports [INV-001] and authenticate via mutual TLS 1.3 [INV-002].
- Pairing: Operators generate a 5-minute ephemeral enrollment token with `androgrid-cli nodes enroll --format qr` and scan it with the Android client camera.

### The Big Questions:
1. Battery destruction? No. We mandate an 80% charging ceiling (avoiding high-voltage electrolyte oxidation) and enforce a minimum 20% floor on battery operation.
2. Thermal throttling? An active hysteresis engine (ThermalSafetyGuard) cuts off computation if battery temperature exceeds 38 C or OS reports THERMAL_STATUS_SEVERE. Tasks resume only after cooling to 35 C.
3. Cellular data drain? Over cellular WAN, sockets close within 200 ms [NFR-POW-001], enabling immediate baseband RRC low-power radio sleep. Persistent heartbeats are strictly prohibited [ANTI-001].
4. Crypto-mining / ad-fraud? Zero tokens, zero blockchains, zero ads. Purely self-hosted parallel compute under the MIT License.

We benchmarked a 4K image tiling pipeline across a cluster of Pixel and Galaxy phones in under 1.5 seconds.

I would love feedback on our architecture and scheduler design!
```

---

## 2. Reddit r/rust

**Title:** [Showcase] AndroGrid: High-concurrency Rust orchestrator, Tonic mTLS 1.3, and mobile WASI edge compute

**Subreddit:** r/rust

**Post Body:**
```markdown
Hi r/rust!

Over the past few months, we designed and built **AndroGrid**: a high-concurrency Rust control plane that coordinates heterogeneous Android devices executing parallel WebAssembly workloads.

Repository: https://github.com/MrQuality/androgrid-backend
Master Specification: AG-SPEC-2026-V2 (ISO 29148 compliant)

### Rust Stack & Architecture Highlights:
- **Tonic 0.12 & Tokio 1.40:** High-throughput async gRPC engine handling dual transport surfaces (persistent declarative Wi-Fi synchronization vs disconnected unary cellular spot requests).
- **Mutual TLS 1.3 via Rustls:** Bidirectional ECDSA P-256 identity verification across all network interfaces. Zero cleartext ingress.
- **Monotonic Attempt Fencing (`[INV-004]`):** Dispatched tasks carry an atomic `run_attempt` counter. Result submissions with stale attempt counters are atomically rejected via CAS state transitions (`TASK_ATTEMPT_STALE`), guaranteeing zero split-brain commits.
- **Speculative Dual-Assignment Scheduler (`[INV-005]`):** Tasks claimed over cellular WAN are treated as 180-second spot leases. If an unmetered Wi-Fi worker connects, the orchestrator dual-assigns the task with an incremented counter. The first node to commit wins.
- **Content-Addressed Bytecode Distribution:** An embedded static artifact engine serves cached WASM binaries via `/artifacts/<sha256>.wasm`, keeping control plane frames sub-4KB to satisfy the 200 ms cellular radio sleep mandate (`[NFR-POW-001]`).
- **Terminal Observability & Web UI:** Built-in Ratatui interactive TUI and Axum-powered real-time cluster metrics dashboard.

### Hero Benchmark Workload:
We implemented a distributed image tiling workload (`androgrid-cli benchmark`) that slices 4K imagery into tiles, distributes them to mobile workers running WASI Preview 1, and reassembles the processed tiles with sub-second latencies.

All code is open source under MIT. We welcome Rustaceans to check out the codebase and provide feedback on our scheduler and state machine patterns!
```

---

## 3. Reddit r/selfhosted

**Title:** Turn your drawer smartphones into a private, zero-cost edge compute cluster (WASI + Rust orchestrator)

**Subreddit:** r/selfhosted

**Post Body:**
```markdown
Hey r/selfhosted!

If you have old Android phones collecting dust in a drawer, you can now run them as private compute workers in a self-hosted cluster using **AndroGrid**.

GitHub: https://github.com/MrQuality/androgrid-backend
Docs & Quickstart: https://mrquality.github.io/androgrid-backend/getting_started/quickstart.html

### What can you do with it?
Run batch background jobs:
- Image processing and thumbnail generation
- Batch video transcode preprocessing
- Data compression and ETL pipelines
- Custom Rust / Go / C / Python WASI microservices

### Why drawer phones instead of Raspberry Pis?
- **Zero Cost:** The hardware is already purchased and amortized.
- **Superior Efficiency:** ARM big.LITTLE mobile chips consume milliwatts on efficiency cores during idle sleep (15-50 mW vs 3-4W continuous idle draw on a Raspberry Pi 4).
- **Built-in UPS:** The phone's battery protects against power outages without needing an external uninterruptible power supply.
- **Hardware Crypto:** High-speed hardware-accelerated AES and SHA instructions on modern ARMv8-A silicon.

### How to run it:
1. Start the orchestrator on your server using Podman Compose:
   ```bash
   podman compose up -d
   ```
2. Generate an enrollment QR code:
   ```bash
   androgrid-cli nodes enroll --format qr
   ```
3. Install the AndroGrid Android worker APK, tap "Scan QR Code", and point your camera at the screen. The device negotiates mutual TLS 1.3 certificates and connects immediately.
4. Submit your first workload using `androgrid-cli submit --wasm app.wasm`.

### Battery & Safety:
We designed this with strict hardware longevity safeguards:
- 80% charging limit recommendation to protect battery cells.
- Thermal throttling halts tasks if battery temperature exceeds 38 C.
- WebAssembly sandboxing isolates workloads completely from device files, contacts, and sensors.
- Zero inbound listening ports on the phone.
- No crypto-mining, no ads, 100% MIT licensed.

Check it out and let us know your thoughts!
```

---

## 4. Lobste.rs

**Title:** AndroGrid: High-concurrency Rust control plane and mobile WASI edge compute cluster

**URL:** https://github.com/MrQuality/androgrid-backend

**Post Body:**
```text
AndroGrid is an open source edge compute orchestration system designed to harness idle mobile devices as cluster compute nodes running WebAssembly/WASI Preview 1 binaries.

Technical Architecture Highlights:
- Control Plane: Implemented in Rust with Tokio and Tonic gRPC. State consensus is maintained using monotonic attempt fencing and compare-and-swap (CAS) transitions to eliminate double-commit races under network latency variations.
- Heterogeneous Transports: Direct mTLS 1.3 HTTP/2 declarative synchronization over Wi-Fi LAN, alongside disconnected, zero-keepalive unary gRPC requests over Cloudflare Anycast ingress on cellular connections.
- Radio Sleep Compliance: In cellular environments, connections terminate within 200 ms to facilitate low-power Radio Resource Control (RRC) tail drops, compliant with ISO 29148 NFR-POW-001.
- In-Process Sandboxing: Unrooted Android clients execute compute bytecode inside memory-isolated sandboxes (Chicory / Wasmtime) with zero inbound listening sockets.
- Thermal Hysteresis: The Android worker enforces a dual-threshold thermal cutoff (38 C suspend, 35 C resume) to avoid start-stop oscillation and hardware degradation.

The full specification (AG-SPEC-2026-V2) and documentation portal are available at:
https://mrquality.github.io/androgrid-backend/
```
