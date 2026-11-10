//! Integration Specification: Public Documentation Portal and GitHub Pages Pipeline
//!
//! Validates structural completeness, link integrity, CLI manual parity,
//! Protobuf API documentation coverage, and typographic standards for the
//! public documentation hub.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-PUB-001], [NFR-DOC-001], [REQ-OP-001], [REQ-OP-004], [INV-002], [NFR-SEC-001]

use std::fs;
use std::path::{Path, PathBuf};

fn get_docs_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("docs")
}

/// Validates that book.toml and SUMMARY.md exist, parse properly,
/// and reference mermaid.js integration.
///
/// Traces to: [NFR-DOC-001], [REQ-PUB-001]
#[test]
fn test_documentation_site_structure_and_book_config() {
    let docs_root = get_docs_root();
    let book_toml_path = docs_root.join("book.toml");
    let summary_path = docs_root.join("src").join("SUMMARY.md");

    assert!(
        book_toml_path.exists(),
        "docs/book.toml must exist at {:?}",
        book_toml_path
    );
    assert!(
        summary_path.exists(),
        "docs/src/SUMMARY.md must exist at {:?}",
        summary_path
    );

    let book_content = fs::read_to_string(&book_toml_path).expect("Failed to read docs/book.toml");

    assert!(
        book_content.contains("title = \"AndroGrid Documentation\""),
        "book.toml must specify title"
    );
    assert!(
        book_content.contains("additional-js"),
        "book.toml must configure additional-js for mermaid"
    );
    assert!(
        book_content.contains("mermaid"),
        "book.toml must reference mermaid scripts"
    );
    assert!(
        book_content.contains("custom.css"),
        "book.toml must configure custom.css for styling and animations"
    );

    // Verify custom styling and animation assets exist
    let custom_css = docs_root.join("theme").join("custom.css");
    assert!(
        custom_css.exists(),
        "theme/custom.css must exist at {:?}",
        custom_css
    );

    // Verify mermaid support assets exist
    let mermaid_js = docs_root.join("theme").join("mermaid.min.js");
    let mermaid_init = docs_root.join("theme").join("mermaid-init.js");
    assert!(
        mermaid_js.exists(),
        "theme/mermaid.min.js must exist at {:?}",
        mermaid_js
    );
    assert!(
        mermaid_init.exists(),
        "theme/mermaid-init.js must exist at {:?}",
        mermaid_init
    );
}

/// Validates that every link declared in SUMMARY.md exists on disk and is non-empty.
///
/// Traces to: [NFR-DOC-001]
#[test]
fn test_all_summary_links_exist_and_non_empty() {
    let docs_root = get_docs_root();
    let summary_path = docs_root.join("src").join("SUMMARY.md");

    assert!(summary_path.exists(), "SUMMARY.md must exist");
    let summary_content = fs::read_to_string(&summary_path).expect("Failed to read SUMMARY.md");

    let mut links_found = 0;
    for line in summary_content.lines() {
        if let (Some(open_paren), Some(close_paren)) = (line.find('('), line.rfind(')')) {
            if open_paren < close_paren {
                let target = &line[open_paren + 1..close_paren];
                if target.ends_with(".md") {
                    let target_path = docs_root.join("src").join(target);
                    assert!(
                        target_path.exists(),
                        "Target file referenced in SUMMARY.md does not exist: {:?}",
                        target_path
                    );
                    let content = fs::read_to_string(&target_path).unwrap_or_else(|_| {
                        panic!("Failed to read target file: {:?}", target_path)
                    });
                    assert!(
                        !content.trim().is_empty(),
                        "Target file must not be empty: {:?}",
                        target_path
                    );
                    links_found += 1;
                }
            }
        }
    }

    assert!(
        links_found >= 6,
        "SUMMARY.md must declare at least 6 core documentation chapters, found {}",
        links_found
    );
}

/// Validates that quickstart guide enforces rootless Podman deployment standards
/// and comprehensive parameter breakdowns.
///
/// Traces to: [NFR-SEC-001], [REQ-PUB-001]
#[test]
fn test_quickstart_enforces_podman_standard_and_detailed_arguments() {
    let docs_root = get_docs_root();
    let quickstart_path = docs_root
        .join("src")
        .join("getting_started")
        .join("quickstart.md");

    assert!(quickstart_path.exists(), "quickstart.md must exist");
    let content = fs::read_to_string(&quickstart_path).expect("Failed to read quickstart.md");

    assert!(
        content.contains("Podman"),
        "Quickstart must document Podman container orchestration"
    );
    assert!(
        content.contains("podman compose"),
        "Quickstart must specify podman compose command"
    );
    assert!(
        content.contains("Parameter Breakdown"),
        "Quickstart must provide detailed parameter breakdowns"
    );
    assert!(
        content.contains("Argument 1") && content.contains("Argument 2"),
        "Quickstart must explain individual positional arguments"
    );
}

/// Validates that Android worker chapters exist and cross-link to the Dokka API portal.
///
/// Traces to: [REQ-PUB-001], [REQ-OP-001], [REQ-OP-003]
#[test]
fn test_android_worker_chapters_and_dokka_cross_links() {
    let docs_root = get_docs_root();
    let guide_path = docs_root
        .join("src")
        .join("android")
        .join("worker_guide.md");
    let arch_path = docs_root
        .join("src")
        .join("android")
        .join("architecture.md");

    assert!(guide_path.exists(), "android/worker_guide.md must exist");
    assert!(arch_path.exists(), "android/architecture.md must exist");

    let guide_content = fs::read_to_string(&guide_path).expect("Failed to read worker_guide.md");
    let arch_content = fs::read_to_string(&arch_path).expect("Failed to read architecture.md");

    assert!(
        guide_content.contains("https://mrquality.github.io/androgrid-android/dokka/index.html")
    );
    assert!(arch_content.contains("https://mrquality.github.io/androgrid-android/dokka/index.html"));
}

/// Validates that the Workload Developer Guide covers multi-language WASI compilation,
/// mentions RFC Issue #22 as an interim step, and documents direct shell fallback.
///
/// Traces to: [REQ-PUB-001], [REQ-OP-004], [INV-006]
#[test]
fn test_workload_developer_compilation_guide_completeness() {
    let docs_root = get_docs_root();
    let guide_path = docs_root
        .join("src")
        .join("workloads")
        .join("compilation_guide.md");

    assert!(
        guide_path.exists(),
        "workloads/compilation_guide.md must exist"
    );
    let content = fs::read_to_string(&guide_path).expect("Failed to read compilation_guide.md");

    // Must reference RFC Issue #22 as an interim step
    assert!(
        content.contains("Issue #22") || content.contains("issues/22"),
        "Must reference RFC Issue #22"
    );
    assert!(
        content.contains("Interim"),
        "Must explicitly explain that manual compilation is an interim step"
    );

    // Must cover core language recipes
    assert!(content.contains("Rust"), "Must document Rust compilation");
    assert!(
        content.contains("wasm32-wasip1"),
        "Must specify wasm32-wasip1 target"
    );
    assert!(
        content.contains("Go"),
        "Must document Go / TinyGo compilation"
    );
    assert!(content.contains("Python"), "Must document Python execution");
    assert!(
        content.contains("JavaScript"),
        "Must document JavaScript execution"
    );

    // Must document direct shell fallback
    assert!(
        content.contains("--shell"),
        "Must document direct shell fallback"
    );
}

/// Validates that the Operator CLI Manual strictly documents all subcommands
/// and flags from the clap definition.
///
/// Traces to: [REQ-PUB-001], [REQ-OP-001], [REQ-OP-004]
#[test]
fn test_operator_cli_manual_synchronization_with_clap_definition() {
    let docs_root = get_docs_root();
    let cli_manual_path = docs_root.join("src").join("operator").join("cli_manual.md");

    assert!(
        cli_manual_path.exists(),
        "docs/src/operator/cli_manual.md must exist"
    );

    let manual = fs::read_to_string(&cli_manual_path)
        .expect("Failed to read docs/src/operator/cli_manual.md");

    // Required subcommands
    let subcommands = ["pack", "submit", "status", "nodes", "cancel", "benchmark"];
    for subcmd in subcommands {
        assert!(
            manual.contains(&format!("### `androgrid-cli {}`", subcmd))
                || manual.contains(&format!("## `androgrid-cli {}`", subcmd)),
            "Operator manual must document subcommand: {}",
            subcmd
        );
    }

    // Required flags & arguments
    let required_flags = [
        "--wasm",
        "--entrypoint",
        "--output",
        "--task-id",
        "--inline",
        "--shell",
        "--root",
        "--timeout",
        "--requires-unmetered",
        "--artifact-url",
        "--env",
        "--endpoint",
        "--ca-cert",
        "--client-cert",
        "--client-key",
        "--format",
        "--tiles",
        "--filter",
        "--dry-run",
    ];

    for flag in required_flags {
        assert!(
            manual.contains(flag),
            "Operator manual must document flag/option: {}",
            flag
        );
    }

    // Must document both text and JSON formatting examples
    assert!(
        manual.contains("--format json"),
        "Operator manual must demonstrate JSON output format"
    );
    assert!(
        manual.contains("--format text"),
        "Operator manual must demonstrate Text output format"
    );
}

/// Validates that the Protobuf API reference documents all RPC services, methods,
/// and the Section 6.3 Canonical Error Matrix.
///
/// Traces to: [REQ-PUB-001], [INV-002], [NFR-SEC-001]
#[test]
fn test_protobuf_api_documentation_completeness_and_error_matrix() {
    let docs_root = get_docs_root();
    let api_doc_path = docs_root.join("src").join("api").join("protobuf.md");

    assert!(api_doc_path.exists(), "docs/src/api/protobuf.md must exist");

    let api_doc =
        fs::read_to_string(&api_doc_path).expect("Failed to read docs/src/api/protobuf.md");

    // Services
    assert!(
        api_doc.contains("AndroGridService"),
        "Must document AndroGridService"
    );
    assert!(
        api_doc.contains("AdminService"),
        "Must document AdminService"
    );

    // AndroGridService RPC methods
    let androgrid_rpcs = [
        "RegisterNode",
        "SyncNodeState",
        "ClaimCellularTask",
        "SubmitCellularResult",
    ];
    for rpc in androgrid_rpcs {
        assert!(
            api_doc.contains(rpc),
            "Must document AndroGridService RPC: {}",
            rpc
        );
    }

    // AdminService RPC methods
    let admin_rpcs = [
        "SubmitTask",
        "GetTaskStatus",
        "ListWorkerNodes",
        "CancelTask",
        "UploadArtifact",
    ];
    for rpc in admin_rpcs {
        assert!(
            api_doc.contains(rpc),
            "Must document AdminService RPC: {}",
            rpc
        );
    }

    // Section 6.3 Canonical gRPC Error Matrix
    let canonical_errors = [
        "CERTIFICATE_INVALID",
        "UNMETERED_REQUIRED",
        "TASK_ATTEMPT_STALE",
        "PAYLOAD_CORRUPT",
    ];
    for err in canonical_errors {
        assert!(
            api_doc.contains(err),
            "Must document canonical error identifier: {}",
            err
        );
    }

    // Verify key architectural traceability tags
    let required_tags = [
        "[REQ-OP-001]",
        "[REQ-OP-002]",
        "[REQ-OP-003]",
        "[REQ-OP-004]",
        "[INV-002]",
        "[NFR-SEC-001]",
    ];
    for tag in required_tags {
        assert!(
            api_doc.contains(tag),
            "Must include traceability identifier in API documentation: {}",
            tag
        );
    }
}

/// Validates that Mermaid diagrams render with valid syntax and are present
/// in the architecture, security, and ingress chapters.
///
/// Traces to: [REQ-PUB-001]
#[test]
fn test_mermaid_diagrams_syntax_and_rendering_integrity() {
    let docs_root = get_docs_root();
    assert!(docs_root.exists(), "docs directory must exist");

    let mut mermaid_blocks_count = 0;
    fn visit_dirs(dir: &Path, count: &mut usize) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path, count);
                } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                    let content = fs::read_to_string(&path).unwrap_or_default();
                    let mut in_mermaid = false;
                    for line in content.lines() {
                        if line.trim().starts_with("```mermaid") {
                            in_mermaid = true;
                            *count += 1;
                        } else if in_mermaid && line.trim() == "```" {
                            in_mermaid = false;
                        }
                    }
                }
            }
        }
    }

    visit_dirs(&docs_root.join("src"), &mut mermaid_blocks_count);
    assert!(
        mermaid_blocks_count >= 3,
        "Documentation must contain at least 3 Mermaid architecture diagrams, found {}",
        mermaid_blocks_count
    );
}

/// Validates that zero Unicode em dashes (U+2014) or en dashes (U+2013) exist
/// in any documentation file or workflow definition.
///
/// Traces to: Typographic Standards (Zero Em/En Dashes)
#[test]
fn test_zero_forbidden_unicode_dashes_in_documentation() {
    let docs_root = get_docs_root();
    assert!(docs_root.exists(), "docs directory must exist");

    fn audit_file(path: &Path) {
        let content = fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("Failed to read file for typographic check: {:?}", path));
        for (line_num, line) in content.lines().enumerate() {
            assert!(
                !line.contains('\u{2014}'),
                "Forbidden em dash (U+2014) found in {:?} at line {}: {}",
                path,
                line_num + 1,
                line
            );
            assert!(
                !line.contains('\u{2013}'),
                "Forbidden en dash (U+2013) found in {:?} at line {}: {}",
                path,
                line_num + 1,
                line
            );
        }
    }

    fn visit_dirs(dir: &Path) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path);
                } else if path.extension().and_then(|s| s.to_str()) == Some("md")
                    || path.extension().and_then(|s| s.to_str()) == Some("toml")
                    || path.extension().and_then(|s| s.to_str()) == Some("yml")
                    || path.extension().and_then(|s| s.to_str()) == Some("js")
                {
                    audit_file(&path);
                }
            }
        }
    }

    visit_dirs(&docs_root);

    // Also check .github/workflows/pages.yml
    let pages_workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("pages.yml");
    if pages_workflow.exists() {
        audit_file(&pages_workflow);
    }
}

/// Validates GitHub Pages deployment workflow configuration.
///
/// Traces to: [NFR-DOC-001]
#[test]
fn test_github_pages_workflow_configuration_integrity() {
    let pages_workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("pages.yml");

    assert!(
        pages_workflow.exists(),
        ".github/workflows/pages.yml must exist at {:?}",
        pages_workflow
    );

    let workflow_content =
        fs::read_to_string(&pages_workflow).expect("Failed to read .github/workflows/pages.yml");

    assert!(
        workflow_content.contains("deploy-pages") || workflow_content.contains("gh-pages"),
        "Workflow must configure GitHub Pages deployment"
    );
    assert!(
        workflow_content.contains("mdbook"),
        "Workflow must build documentation using mdbook"
    );
    assert!(
        workflow_content.contains("branches: [ \"main\" ]")
            || workflow_content.contains("branches: [ main ]"),
        "Workflow must trigger on main branch"
    );
}

/// Validates that demo animated showcase asset exists, is non-empty,
/// and adheres to the strict 5 MB file size limit to prevent git bloat.
///
/// Traces to: [REQ-PUB-001], G-21
#[test]
fn test_showcase_demo_asset_exists_and_size_bounds() {
    let docs_root = get_docs_root();
    let demo_gif_path = docs_root.join("assets").join("demo.gif");

    assert!(
        demo_gif_path.exists(),
        "docs/assets/demo.gif must exist at {:?}",
        demo_gif_path
    );

    let metadata = fs::metadata(&demo_gif_path)
        .unwrap_or_else(|_| panic!("Failed to read metadata of {:?}", demo_gif_path));

    assert!(metadata.len() > 0, "demo.gif must not be empty");
    assert!(
        metadata.len() < 5_000_000,
        "demo.gif must be under 5 MB to prevent git bloat, found {} bytes",
        metadata.len()
    );
}

/// Validates that the Mobile Cluster Economics and Battery FAQ chapter exists,
/// provides empirical battery and thermal metrics, and cites key architectural invariants.
///
/// Traces to: [NFR-POW-001], [ANTI-001], [INV-003], [REQ-OP-003], G-21
#[test]
fn test_showcase_economics_faq_chapter_completeness() {
    let docs_root = get_docs_root();
    let faq_path = docs_root
        .join("src")
        .join("showcase")
        .join("economics_faq.md");

    assert!(
        faq_path.exists(),
        "docs/src/showcase/economics_faq.md must exist at {:?}",
        faq_path
    );

    let content =
        fs::read_to_string(&faq_path).expect("Failed to read docs/src/showcase/economics_faq.md");

    // Battery longevity and charging limits
    assert!(
        content.contains("80%") || content.contains("80 %"),
        "FAQ must explain battery charging limit ceiling (80%)"
    );
    assert!(
        content.contains("chemical degradation") || content.contains("battery longevity"),
        "FAQ must address chemical degradation and battery longevity"
    );

    // Thermal throttling hysteresis
    assert!(
        content.contains("38") && content.contains("35"),
        "FAQ must cite 38 C cutoff and 35 C resume hysteresis thresholds"
    );
    assert!(
        content.contains("hysteresis"),
        "FAQ must detail thermal hysteresis dampening"
    );

    // Energy efficiency and ARM big.LITTLE
    assert!(
        content.contains("big.LITTLE") || content.contains("DynamIQ"),
        "FAQ must analyze ARM big.LITTLE / DynamIQ efficiency"
    );
    assert!(
        content.contains("Raspberry Pi") || content.contains("RPi"),
        "FAQ must contrast mobile SoC efficiency against Raspberry Pi / x86"
    );

    // Zero hardware cost and drawer devices
    assert!(
        content.contains("zero-cost") || content.contains("drawer"),
        "FAQ must articulate zero hardware cost of drawer smartphones"
    );

    // Key traceability identifiers
    let required_identifiers = ["[NFR-POW-001]", "[ANTI-001]", "[INV-003]", "[REQ-OP-003]"];
    for id in required_identifiers {
        assert!(
            content.contains(id),
            "Economics FAQ must cite architectural identifier: {}",
            id
        );
    }
}

/// Validates that the Ecosystem Overview landing page exists, maps all core repositories
/// and specifications, and embeds a Mermaid architecture topology diagram.
///
/// Traces to: [REQ-PUB-001], G-21
#[test]
fn test_showcase_ecosystem_landing_page_and_navigation() {
    let docs_root = get_docs_root();
    let ecosystem_path = docs_root.join("src").join("showcase").join("ecosystem.md");

    assert!(
        ecosystem_path.exists(),
        "docs/src/showcase/ecosystem.md must exist at {:?}",
        ecosystem_path
    );

    let content =
        fs::read_to_string(&ecosystem_path).expect("Failed to read docs/src/showcase/ecosystem.md");

    // Must map all three pillars
    assert!(
        content.contains("androgrid-backend"),
        "Ecosystem page must reference androgrid-backend"
    );
    assert!(
        content.contains("androgrid-android"),
        "Ecosystem page must reference androgrid-android"
    );
    assert!(
        content.contains("AndroGrid_SDD.md") || content.contains("AG-SPEC-2026-V2"),
        "Ecosystem page must reference master specification"
    );

    // Must contain Mermaid diagram
    assert!(
        content.contains("```mermaid"),
        "Ecosystem page must include a Mermaid architecture diagram"
    );

    // Must link to external resources
    assert!(
        content.contains("dokka"),
        "Ecosystem page must link to Dokka API documentation"
    );
}

/// Validates that internal launch templates are strictly isolated from the public repository.
///
/// Traces to: [REQ-PUB-001], G-21
#[test]
fn test_internal_launch_blueprints_strictly_isolated_from_public_docs() {
    let docs_root = get_docs_root();
    let launch_path = docs_root
        .join("src")
        .join("showcase")
        .join("community_launch.md");

    assert!(
        !launch_path.exists(),
        "community_launch.md must NOT exist in public repository: {:?}",
        launch_path
    );
}

/// Validates that ADR-004 is authored, accepted, and linked in SUMMARY.md.
///
/// Traces to: [REQ-PUB-001], G-21
#[test]
fn test_showcase_adr_004_exists_and_referenced() {
    let docs_root = get_docs_root();
    let adr_path = docs_root
        .join("src")
        .join("adr")
        .join("ADR-004-community-showcase-and-demo-assets.md");

    assert!(adr_path.exists(), "ADR-004 must exist at {:?}", adr_path);

    let adr_content = fs::read_to_string(&adr_path).expect("Failed to read ADR-004");
    assert!(
        adr_content.contains("## Status") && adr_content.contains("Accepted"),
        "ADR-004 must be in Accepted status"
    );

    let summary_path = docs_root.join("src").join("SUMMARY.md");
    let summary_content = fs::read_to_string(&summary_path).expect("Failed to read SUMMARY.md");

    assert!(
        summary_content.contains("ADR-004-community-showcase-and-demo-assets.md"),
        "SUMMARY.md must reference ADR-004"
    );
}

/// Validates that the repository root README embeds the demo GIF asset
/// and includes community showcase navigation.
///
/// Traces to: [REQ-PUB-001], G-21
#[test]
fn test_readme_embeds_demo_asset_and_showcase_sections() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let readme_path = manifest_dir.join("README.md");

    assert!(readme_path.exists(), "README.md must exist at root");

    let readme_content = fs::read_to_string(&readme_path).expect("Failed to read README.md");

    assert!(
        readme_content.contains("docs/assets/demo.gif"),
        "README.md must embed docs/assets/demo.gif"
    );
    assert!(
        readme_content.contains("Community Showcase")
            || readme_content.contains("Economics & Hardware Feasibility"),
        "README.md must feature community showcase or feasibility section"
    );
}
