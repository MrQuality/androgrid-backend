# ADR-002: Dual-Route Protocol Demarcation (Persistent Wi-Fi Loop vs Disconnected Zero-Keepalive Cellular WAN)

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-15
- **Traceability:** `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[INV-001]`, `[INV-003]`, `[ANTI-001]`, `[ANTI-002]`, `[NFR-POW-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
AndroGrid worker nodes operate under two fundamentally contrasting network regimes:
1. **Unmetered Local Wi-Fi:** Devices are typically connected to local AC power and unmetered broadband. Low latency and high throughput enable continuous, bidirectional synchronization.
2. **Metered Cellular WAN (LTE / 5G):** Devices are battery-powered and operate over carrier networks with strict data metering and energy constraints.

Maintaining an active TCP or HTTP/2 streaming socket over cellular networks forces the mobile device baseband processor to stay in a high-power Radio Resource Control (RRC) Connected / DCH state. In 4G and 5G basebands, the active radio state consumes 1.5W to 2.5W of continuous power. Even small periodic keepalive packets (e.g., gRPC ping frames every 30 seconds) trigger baseband state transitions and prevent the RRC inactivity timer from expiring. This leads to rapid battery depletion, device heating, and aggressive task killing by the Android operating system.

---

## Decision
We architect a **Dual-Route Protocol Demarcation** that bifurcates transport lifecycle based on active network transport:
1. **Wi-Fi LAN Route (Persistent Declarative Loop):**
   - When connected to a designated cluster Wi-Fi SSID, the worker maintains a persistent mTLS gRPC channel.
   - The worker executes a 2.0-second declarative synchronization loop (`SyncNodeState`), reporting local task progress and pulling newly scheduled jobs.
2. **Cellular WAN Route (Disconnected Zero-Keepalive Unary Lifecycle):**
   - When connected via cellular WAN, the worker interacts with the control plane strictly through two isolated unary RPC calls: `ClaimCellularTask` and `SubmitCellularResult`.
   - Immediately upon receiving the `ClaimCellularTaskResponse`, the worker unconditionally closes the gRPC channel and tears down the underlying TCP socket within 200 ms (`[NFR-POW-001]`).
   - The task execution proceeds entirely offline in the local WASI sandbox while the baseband radio enters low-power RRC idle sleep state.
   - Upon execution completion or fatal failure, the worker establishes a brand-new outbound mTLS connection to invoke `SubmitCellularResult`, tearing down the socket immediately following result acknowledgment.

### Architectural Rationale
- **Baseband Tail-Drop Optimization:** Mobile RRC baseband processors require 10 to 15 seconds of silence before transitioning to low-power idle state. Zero keepalive guarantees rapid entry into idle state during the computation phase.
- **Outbound-Only Invariant (`[INV-001]`):** Cellular mobile carriers enforce Carrier-Grade NAT (CGNAT), making inbound socket connections impossible. Dual-route unary calls strictly originate outbound from the worker node toward the Cloudflare Anycast ingress.
- **Power Envelope Preservation:** Eliminating heartbeat chatter reduces battery consumption by over 80% during long-running tasks.

---

## Consequences

### Positive
- Cellular worker devices conserve battery power and avoid baseband thermal runaway.
- Completely adheres to `[ANTI-001]` (zero keepalive over metered networks).
- Operates reliably through restrictive CGNAT and mobile carrier firewalls without port forwarding (`[ANTI-002]`).

### Trade-offs
- The orchestrator cannot proactively cancel or interrupt a cellular task once leased, requiring speculative preemption and deadline expiration mechanisms (`[INV-005]`).
- Connection re-establishment incurs a single TLS 1.3 handshake penalty per result submission.

---

## Alternatives Considered
- **Unified Persistent gRPC Streaming across all transports:** Rejected because persistent cellular streaming prevents RRC sleep, draining mobile battery within hours.
- **Periodic Cellular Heartbeat Polling:** Rejected because recurring polling restarts the baseband RRC tail timer, maintaining high radio power draw continuously.
- **Mobile Push Notifications (Firebase Cloud Messaging / FCM):** Rejected due to external cloud dependency, variable delivery latency (up to minutes), and violation of hermetic cluster operation.
