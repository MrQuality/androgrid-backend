# ADR-001: Public Documentation Portal and Headless Publishing Pipeline

## Status
Accepted

## Architectural Scope & Requirements
- **Feature Reference:** Issue #10
- **Traceability:** `[NFR-DOC-001]`, `[REQ-PUB-001]`
- **Standard:** ISO/IEC/IEEE 29148

---

## Context
Project documentation previously existed across fragmented files (`README.md`, architecture specifications, and code docstrings). External operators, contributors, and systems engineers required a centralized, searchable, and interactive technical documentation hub.

Key engineering constraints:
1. **Public Isolation:** Public documentation must never expose internal infrastructure hostnames, credentials, or private repository paths.
2. **Automated Headless Publishing (`[NFR-DOC-001]`):** Zero manual intervention in deployment. Must build and publish automatically to GitHub Pages on merges to `main`.
3. **Diagramming Support:** Render complex architectural topologies and sequence flows cleanly using Mermaid.js.
4. **API and CLI Drift Prevention:** Zero drift between code definitions (`clap` CLI flags, Protobuf `.proto` schemas) and documentation.
5. **Typographic Standard:** Zero occurrences of Unicode em dashes (U+2014) or en dashes (U+2013).

---

## Decision
We select **mdBook** as the official static documentation site generator for AndroGrid, complemented by client-side Mermaid.js rendering and GitHub Actions Pages deployment.

### Architectural Rationale
1. **Rust Ecosystem Alignment:** `mdBook` is the canonical standard for Rust projects, providing sub-second build times, offline full-text search indexing, and native Markdown processing.
2. **Client-Side Mermaid.js Integration:** Rather than requiring heavy third-party Rust preprocessors (`mdbook-mermaid`) which complicate CI cross-compilation, Mermaid.js is loaded dynamically via `additional-js` in `docs/book.toml`.
3. **Automated Parity Verification:** An automated Rust integration test suite (`tests/documentation_portal_spec.rs`) validates that every CLI subcommand and flag defined in `clap` is documented, all Protobuf RPC methods are covered, and all internal links resolve.
4. **Headless Deployment:** GitHub Actions workflow `.github/workflows/pages.yml` builds the site headlessly and publishes the static artifacts directly to GitHub Pages.

---

## Consequences
- **Positive:**
  - High-performance, searchable documentation portal with dark and light theme switching.
  - Interactive Mermaid architecture diagrams render on all standard web browsers.
  - Zero risk of CLI flag drift due to automated CI test verification.
- **Trade-offs:**
  - Dynamic Mermaid rendering requires JavaScript enabled in the client browser.
  - Protobuf API documentation is verified via integration tests rather than manual wiki edits.
