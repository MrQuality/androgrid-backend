# Summary

- [Introduction](README.md)
- [System Architecture](architecture/system_overview.md)
- [ISO 29148 Traceability Matrix](traceability_matrix.md)
- [Getting Started & Quickstart](getting_started/quickstart.md)
- [Workload Developer Guide: Compiling to WASI](workloads/compilation_guide.md)
  - [Tutorial: Running Your First Parallel Cluster Benchmark](workloads/image_benchmark_tutorial.md)
- [Operator Manual & CLI Reference](operator/cli_manual.md)
- [Security & PKI Architecture](security/architecture.md)
- [Cloudflare Ingress Deployment Guide](ingress/cloudflare_tunnel.md)
- [Protobuf gRPC API Specification](api/protobuf.md)
- [Control Plane Architecture Decision Records](adr/ADR-001-mutual-tls-1.3-ecdsa-p256.md)
  - [ADR-001: Mutual TLS 1.3 with ECDSA P-256](adr/ADR-001-mutual-tls-1.3-ecdsa-p256.md)
  - [ADR-002: Dual-Route Protocol Demarcation](adr/ADR-002-dual-route-protocol-demarcation.md)
  - [ADR-003: Monotonic Attempt Fencing and Lock-Free CAS Consensus](adr/ADR-003-monotonic-attempt-fencing-cas.md)
  - [ADR-004: Speculative Preemption and Dual-Assignment](adr/ADR-004-speculative-preemption-dual-assignment.md)
  - [ADR-005: Interface Segregation Principle for AdminService vs AndroGridService](adr/ADR-005-interface-segregation-admin-vs-worker.md)
- [Feature Architecture Decision Records](adr/ADR-001-documentation-portal.md)
  - [ADR-001: Documentation Portal](adr/ADR-001-documentation-portal.md)
  - [ADR-002: Release Automation](adr/ADR-002-release-automation.md)
  - [ADR-003: Distributed Image Benchmark Hero Workload](adr/ADR-003-distributed-image-benchmark.md)
  - [ADR-004: Interactive Cluster Demo Assets and Hardware Feasibility FAQ](adr/ADR-004-community-showcase-and-demo-assets.md)

# Cluster Feasibility & Ecosystem Architecture

- [Ecosystem Architecture & Navigation](showcase/ecosystem.md)
- [Mobile Cluster Economics & Battery FAQ](showcase/economics_faq.md)

# Mobile Worker Client

- [Android Worker Onboarding Guide](android/worker_guide.md)
- [Android Client Architecture](android/architecture.md)

# Alpha Testing & Hardware Verification

- [Hardware Alpha Testing Guide - v0.0.1-alpha.1](testing/hardware_alpha_guide.md)
