# ADR-004: Community Showcase, Interactive Demo Assets & Hardware Feasibility FAQ

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-21
- **Traceability:** `[REQ-PUB-001]`, `[NFR-POW-001]`, `[ANTI-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
A technically sophisticated distributed edge-compute platform faces community skepticism upon initial public presentation. Developers browsing forums such as Hacker News, Reddit, and Lobste.rs typically apply a 30-second attention filter. Without immediate visual proof and clear, empirical answers to common objections ("Doesn't this burn phone batteries?", "Why not use a Raspberry Pi?", "Is this crypto-mining?"), readers dismiss the project.

Key engineering requirements:
1. **Visual Demonstration (`[REQ-PUB-001]`):** Provide a compact, end-to-end animated demonstration showing orchestrator startup, device enrollment, workload execution, and progress bars.
2. **Asset Size Bounds:** Media assets must not inflate repository clone sizes. The demo asset must stay strictly under 5 MB.
3. **Empirical Rationale:** FAQ claims must cite verified architectural invariants: 80% charging limits, 38 C / 35 C thermal hysteresis (`ThermalSafetyGuard`), 200 ms cellular socket teardown (`[NFR-POW-001]`), and zero keepalive (`[ANTI-001]`).
4. **Ecosystem Navigation:** Present a unified architectural map linking `androgrid-backend`, `androgrid-android`, and `AndroGrid_SDD.md`.
5. **Typographic Standard:** Zero occurrences of Unicode em dashes (U+2014) or en dashes (U+2013).

---

## Decision
We implement a four-part Community Showcase subsystem within the public documentation portal and root repository assets:

1. **High-Resolution Animated Demonstration Asset (`docs/assets/demo.gif`):**
   - Programmatically generated using a headless PIL rendering engine (`tools/docs/generate_demo_gif.py`).
   - Illustrates end-to-end flow: `podman compose up`, `androgrid-cli nodes enroll --format qr`, worker connection, and parallel image tiling benchmark execution.
   - Optimized with adaptive 128-color quantization to achieve sub-200 KB footprint (164 KB), well below the 5 MB upper bound.
2. **Empirical Hardware Feasibility FAQ (`docs/src/showcase/economics_faq.md`):**
   - Directly tackles battery longevity (80% charge ceiling, SEI layer mitigation), thermal hysteresis (38 C cutoff, 35 C resume), and comparative efficiency against Raspberry Pi and x86 hardware.
   - Highlights ARM big.LITTLE power efficiency (15-50 mW idle sleep vs 3-4W RPi idle) and built-in hardware peripherals (UPS battery, Wi-Fi 6, hardware crypto).
3. **Ecosystem Architecture & Navigation Portal (`docs/src/showcase/ecosystem.md`):**
   - Unifies the three core pillars: Control Plane (`androgrid-backend`), Mobile Client (`androgrid-android`), and Master Specification (`AndroGrid_SDD.md`).
   - Embeds a Mermaid flowchart detailing component interactions and cross-links to the interactive Dokka API reference.
4. **Multi-Channel Community Launch Blueprints (`docs/src/showcase/community_launch.md`):**
   - Authors tailored launch copy for Hacker News (`Show HN`), Reddit (`r/rust` and `r/selfhosted`), and `Lobste.rs`.
   - Adheres strictly to each community's technical disclosure norms.

---

## Consequences
- **Positive:**
  - Solves the 30-second attention challenge with embedded visual demonstrations in root README files.
  - Proactively transforms skeptical developers into informed contributors through empirical hardware data.
  - Keeps repository clone size minimal (< 200 KB for all media assets).
  - Maintains strict traceability to ISO 29148 requirements and AG-SPEC-2026-V2 invariants.
- **Trade-offs:**
  - Dynamic animated GIF generation must be rerun if CLI or dashboard interfaces evolve.
  - Community launch blueprints must be periodically reviewed against external forum guideline changes.
