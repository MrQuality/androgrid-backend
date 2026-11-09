# ADR-002: Multi-Platform Binary & Container Release Automation

## Status
Accepted

## Architectural Scope & Requirements
- **Goal Reference:** G-16
- **GitHub Issue:** https://github.com/MrQuality/androgrid-backend/issues/12
- **Traceability:** `[REQ-OP-001]`, `[REQ-OP-004]`, `[NFR-SEC-001]`, `[NFR-PERF-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Deploying the AndroGrid control plane orchestrator (`androgrid-backend`) and operator CLI (`androgrid-cli`) previously required operators to install the Rust toolchain and build from source. To accelerate production deployments, enable edge device orchestration, and support rootless container environments, pre-compiled release artifacts and multi-architecture container images must be automatically generated, cryptographically verified, and published upon git tag releases.

Key engineering constraints:
1. **Multi-Platform Support (`[REQ-OP-001]`):** Must cross-compile for 5 major host architectures:
   - Linux x86_64 (`x86_64-unknown-linux-gnu`)
   - Linux ARM64 (`aarch64-unknown-linux-gnu`)
   - Windows x86_64 (`x86_64-pc-windows-msvc`)
   - macOS Apple Silicon (`aarch64-apple-darwin`)
   - macOS Intel (`x86_64-apple-darwin`)
2. **Binary Hardening & Size Optimization (`[NFR-PERF-001]`):** Release profiles must enforce Link-Time Optimization (`lto = true`), symbol stripping (`strip = true`), and single codegen units (`codegen-units = 1`) for reproducible minimal footprints.
3. **Cryptographic Integrity (`[NFR-SEC-001]`):** Every published release archive must have a verifiable SHA-256 checksum consolidated into `SHA256SUMS.txt`.
4. **OCI Multi-Architecture Container Publishing (`[REQ-OP-004]`):** Automated publishing to GitHub Container Registry (`ghcr.io`) supporting both `linux/amd64` and `linux/arm64` under non-root unprivileged execution.
5. **Typographic Standard:** Zero occurrences of Unicode em dashes (U+2014) or en dashes (U+2013).

---

## Decision
We implement automated cross-platform release pipelines in `.github/workflows/release.yml` triggered on version tag pushes (`v*`).

### Architectural Rationale
1. **Matrix Cross-Compilation:**
   - GitHub Actions matrix runners compile native binaries on Ubuntu, Windows, and macOS.
   - Linux ARM64 targets utilize `gcc-aarch64-linux-gnu` toolchain cross-compilation without heavy emulation overhead, leveraging pure-Rust cryptography (`rustls`, `ring`) and host-executed `protoc-bin-vendored`.
2. **Hardened Cargo Release Profile:**
   - Configured in `Cargo.toml` under `[profile.release]` with `opt-level = 3`, `lto = true`, `strip = true`, and `codegen-units = 1`.
3. **Dual Binary Packaging:**
   - Both `androgrid-backend` and `androgrid-cli` are packaged together with `README.md` and `LICENSE` in `.tar.gz` (Unix/macOS) and `.zip` (Windows) formats.
4. **Consolidated SHA-256 Digest Ledger:**
   - A dedicated aggregation job downloads all platform archives, computes SHA-256 digests, publishes `SHA256SUMS.txt`, and validates consistency before creating the GitHub Release.
5. **Multi-Arch OCI Publishing with Buildx:**
   - Uses Docker Buildx with QEMU emulation to compile and push multi-architecture (`linux/amd64`, `linux/arm64`) container images to `ghcr.io/mrquality/androgrid-backend`.
   - Embeds standard OpenContainers metadata and semver tags.

---

## Consequences
- **Positive:**
  - Zero-effort binary adoption: operators download ready-to-run binaries matching their target OS.
  - End-to-end cryptographic supply chain integrity via `SHA256SUMS.txt`.
  - Multi-arch Docker and Podman container images available immediately on `ghcr.io`.
- **Trade-offs:**
  - Cross-compilation matrix requires multi-runner minutes during release pipeline runs.
  - Container buildx for ARM64 requires QEMU emulation in GitHub Actions runners.
