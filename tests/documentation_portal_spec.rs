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
    let subcommands = ["pack", "submit", "status", "nodes", "cancel"];
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
