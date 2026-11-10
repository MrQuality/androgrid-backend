# ADR-001: Mutual TLS 1.3 with ECDSA P-256 for Mobile Hardware Crypto Accelerators

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-15
- **Traceability:** `[INV-002]`, `[NFR-SEC-001]`, `[REQ-OP-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
AndroGrid coordinates untrusted and semi-trusted Android mobile hardware as distributed edge compute workers across heterogeneous networks (local Wi-Fi LAN and public Cellular WAN via Cloudflare Anycast ingress).

The communication security architecture must satisfy strict engineering constraints:
1. **Zero-Trust Network Topology:** Traffic traverses untrusted public cellular networks, corporate LANs, and edge proxies. Network-level perimeter security cannot be assumed.
2. **Hardware Crypto Constraints:** Mobile processors have constrained battery budgets and thermal envelopes. RSA cryptographic operations (2048-bit or 4096-bit) demand significant CPU cycle overhead, increasing handshake latency and battery consumption.
3. **Hardware Keystore Alignment:** Modern Android devices (API 28+) feature dedicated Secure Elements (SE) and StrongBox Keymaster hardware accelerators designed specifically for elliptic curve cryptography on NIST P-256 (secp256r1).
4. **Transport Security Standard:** Outdated TLS 1.2 implementations permit weak cipher suites, renegotiation attacks, and multi-round-trip handshake delays.

---

## Decision
We enforce **Mutual TLS 1.3 (mTLS)** as the exclusive cryptographic transport for all control plane interactions across both Wi-Fi LAN and Cellular WAN:
1. **Protocol Version:** TLS 1.3 is strictly mandated via Rustls 0.23 on the host control plane and Conscrypt / OkHttp on Android workers. Older TLS versions (TLS 1.0, 1.1, 1.2) are disabled.
2. **Cipher Suites:** Restrict negotiation strictly to forward-secret AEAD cipher suites: `TLS_AES_128_GCM_SHA256` and `TLS_AES_256_GCM_SHA384`.
3. **Elliptic Curve Standard:** Standardize all internal X.509 v3 root CA, server certificates, and mobile client certificates on ECDSA NIST P-256 (`prime256v1` / `secp256r1`) with SHA-256 signatures.
4. **Bidirectional Authentication:** Both server and client identities are cryptographically proven during the TLS handshake before any HTTP/2 gRPC frame is deserialized.

### Architectural Rationale
- **Hardware Acceleration:** NIST P-256 operations execute directly within hardware-backed crypto blocks on ARM SoC chips (Snapdragon, Tensor, Exynos), reducing handshake verification to sub-millisecond durations with minimal energy consumption.
- **Single-RTT Handshake:** TLS 1.3 completes full mutual authentication in a single round-trip, minimizing cellular radio wake time.
- **Cryptographic Channel Binding:** Mutual certificate authentication binds the worker identity directly to the transport layer, eliminating vulnerabilities associated with bearer token leakage.

---

## Consequences

### Positive
- Sub-millisecond mutual authentication latency leveraging mobile hardware acceleration.
- End-to-end forward secrecy for all task specifications, bytecode, and execution results.
- Complete mitigation of man-in-the-middle (MITM) attacks and unauthorized node injection.
- Zero plaintext cluster communications across LAN or WAN boundaries.

### Trade-offs
- Certificate lifecycle management requires automated enrollment, provisioning, and revocation pipelines.
- Mobile clients must complete ephemeral pairing enrollment to receive signed client certificates prior to cluster participation.

---

## Alternatives Considered
- **RSA 2048/4096 mTLS:** Rejected due to excessive compute cycles during signature verification, larger certificate transmission payloads, and high battery drain on mobile devices.
- **TLS 1.2 with Mutual Authentication:** Rejected due to multi-RTT handshake latency, legacy cipher negotiation vulnerabilities, and higher connection establishment delay.
- **Application-Layer Token Authentication (JWT/API Keys over Plaintext HTTP/2):** Rejected because bearer tokens are vulnerable to interception, replay attacks, and token theft, violating zero-trust invariants.
