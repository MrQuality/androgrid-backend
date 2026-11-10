//! ISO/IEC/IEEE 29148 Traceability Matrix and Control Plane ADR Verification Suite
//!
//! Validates 100% requirements coverage, bidirectional source and test links,
//! Michael Nygard ADR conformance, and zero typographic dash violations.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004],
//! [INV-001], [INV-002], [INV-003], [INV-004], [INV-005], [INV-006],
//! [ANTI-001], [ANTI-002], [ANTI-003],
//! [NFR-SEC-001], [NFR-POW-001], [NFR-PERF-001], [NFR-REL-001],
//! [REQ-VER-001], [REQ-VER-002], [REQ-VER-003]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn get_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Normative requirements defined in AG-SPEC-2026-V2 that must be mapped
/// in the ISO 29148 Traceability Matrix.
const MANDATORY_REQUIREMENT_TAGS: &[&str] = &[
    "[REQ-OP-001]",
    "[REQ-OP-002]",
    "[REQ-OP-003]",
    "[REQ-OP-004]",
    "[INV-001]",
    "[INV-002]",
    "[INV-003]",
    "[INV-004]",
    "[INV-005]",
    "[INV-006]",
    "[ANTI-001]",
    "[ANTI-002]",
    "[ANTI-003]",
    "[NFR-SEC-001]",
    "[NFR-POW-001]",
    "[NFR-PERF-001]",
    "[NFR-REL-001]",
    "[REQ-VER-001]",
    "[REQ-VER-002]",
    "[REQ-VER-003]",
];

/// Expected 5 Host Control Plane ADRs defined in Issue #11 / Goal G-15.
const EXPECTED_CONTROL_PLANE_ADRS: &[(&str, &str)] = &[
    ("ADR-001-mutual-tls-1.3-ecdsa-p256.md", "[INV-002]"),
    ("ADR-002-dual-route-protocol-demarcation.md", "[REQ-OP-001]"),
    ("ADR-003-monotonic-attempt-fencing-cas.md", "[INV-004]"),
    (
        "ADR-004-speculative-preemption-dual-assignment.md",
        "[INV-005]",
    ),
    (
        "ADR-005-interface-segregation-admin-vs-worker.md",
        "[REQ-OP-001]",
    ),
];

/// Validates that docs/traceability_matrix.md exists, is non-empty, and maps
/// 100% of all master specification requirements to implementation and verification artifacts.
///
/// Traces to: AG-SPEC-2026-V2, [ISO-29148]
#[test]
fn test_traceability_matrix_exists_and_covers_all_normative_requirements() {
    let repo_root = get_repo_root();
    let matrix_path = repo_root.join("docs").join("traceability_matrix.md");

    assert!(
        matrix_path.exists(),
        "docs/traceability_matrix.md must exist at {:?}",
        matrix_path
    );

    let content = fs::read_to_string(&matrix_path)
        .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", matrix_path, e));

    assert!(
        content.len() > 1000,
        "docs/traceability_matrix.md must be comprehensive, found length {}",
        content.len()
    );

    for tag in MANDATORY_REQUIREMENT_TAGS {
        assert!(
            content.contains(tag),
            "Traceability matrix must cover normative requirement {}",
            tag
        );
    }

    assert!(
        content.contains("Section 8"),
        "Traceability matrix must trace to Section 8 Normative Gherkin BDD Scenarios"
    );
    assert!(
        content.contains("bdd_acceptance_spec.rs"),
        "Traceability matrix must reference bdd_acceptance_spec.rs"
    );
}

/// Validates that all file paths referenced in docs/traceability_matrix.md exist on disk.
///
/// Traces to: AG-SPEC-2026-V2, [ISO-29148]
#[test]
fn test_traceability_matrix_references_valid_files_on_disk() {
    let repo_root = get_repo_root();
    let matrix_path = repo_root.join("docs").join("traceability_matrix.md");

    assert!(matrix_path.exists(), "Matrix must exist");
    let content = fs::read_to_string(&matrix_path).expect("Failed to read matrix");

    // Scan for backticked paths ending in .rs, .proto, .py, or .md
    let mut checked_count = 0;
    let parts: Vec<&str> = content.split('`').collect();
    for token in parts {
        let token = token.trim();
        if (token.ends_with(".rs")
            || token.ends_with(".proto")
            || token.ends_with(".py")
            || token.ends_with(".md"))
            && (token.contains('/') || token.contains('\\'))
        {
            if token.starts_with("http") || token.contains("vendor") || token.contains("std/") {
                continue;
            }

            let full_path = repo_root.join(token);
            assert!(
                full_path.exists(),
                "File referenced in traceability matrix does not exist on disk: {:?} (derived from relative path: {})",
                full_path,
                token
            );
            checked_count += 1;
        }
    }

    assert!(
        checked_count >= 10,
        "Matrix must reference at least 10 concrete files, verified {}",
        checked_count
    );
}

/// Validates that all 5 Host Control Plane ADRs exist under docs/adr/ and conform
/// to the Michael Nygard architectural decision record structure.
///
/// Traces to: AG-SPEC-2026-V2, [INV-001], [INV-002], [INV-003], [INV-004], [INV-005]
#[test]
fn test_control_plane_adrs_exist_and_conform_to_nygard_format() {
    let repo_root = get_repo_root();
    let adr_dir = repo_root.join("docs").join("adr");

    assert!(
        adr_dir.exists(),
        "docs/adr directory must exist at {:?}",
        adr_dir
    );

    for (filename, expected_trace) in EXPECTED_CONTROL_PLANE_ADRS {
        let file_path = adr_dir.join(filename);
        assert!(
            file_path.exists(),
            "Control Plane ADR file {:?} must exist",
            file_path
        );

        let content = fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", file_path, e));

        // Format checks: Title, Status, Context, Decision, Consequences, Alternatives Considered
        assert!(
            content.starts_with("# ADR-00"),
            "ADR {:?} must start with '# ADR-00'",
            filename
        );
        assert!(
            content.contains("## Status") && content.contains("Accepted"),
            "ADR {:?} must have Accepted status",
            filename
        );
        assert!(
            content.contains("## Context"),
            "ADR {:?} must define Context section",
            filename
        );
        assert!(
            content.contains("## Decision"),
            "ADR {:?} must define Decision section",
            filename
        );
        assert!(
            content.contains("## Consequences"),
            "ADR {:?} must define Consequences section",
            filename
        );
        assert!(
            content.contains("## Alternatives Considered"),
            "ADR {:?} must document Alternatives Considered",
            filename
        );
        assert!(
            content.contains(expected_trace),
            "ADR {:?} must cite expected traceability tag {}",
            filename,
            expected_trace
        );
    }
}

/// Validates that the documentation portal (mdBook) links to the traceability matrix
/// and control plane ADRs in docs/src/SUMMARY.md.
///
/// Traces to: AG-SPEC-2026-V2, [REQ-PUB-001], [NFR-DOC-001]
#[test]
fn test_traceability_portal_integration_and_summary() {
    let repo_root = get_repo_root();
    let summary_path = repo_root.join("docs").join("src").join("SUMMARY.md");
    let portal_matrix_path = repo_root
        .join("docs")
        .join("src")
        .join("traceability_matrix.md");

    assert!(summary_path.exists(), "docs/src/SUMMARY.md must exist");
    assert!(
        portal_matrix_path.exists(),
        "docs/src/traceability_matrix.md must exist for mdBook compilation"
    );

    let summary_content = fs::read_to_string(&summary_path).expect("Failed to read SUMMARY.md");

    assert!(
        summary_content.contains("traceability_matrix.md"),
        "SUMMARY.md must reference traceability_matrix.md"
    );

    for (filename, _) in EXPECTED_CONTROL_PLANE_ADRS {
        assert!(
            summary_content.contains(filename),
            "SUMMARY.md must reference Control Plane ADR {}",
            filename
        );
    }
}

/// Audits all ADR and traceability files to ensure zero occurrences of forbidden
/// Unicode em dashes (U+2014) or en dashes (U+2013).
///
/// Traces to: AGENTS.md typographic standards
#[test]
fn test_zero_forbidden_unicode_dashes_in_traceability_and_adrs() {
    let repo_root = get_repo_root();
    let files_to_check = vec![
        repo_root.join("docs").join("traceability_matrix.md"),
        repo_root
            .join("docs")
            .join("src")
            .join("traceability_matrix.md"),
        repo_root
            .join("docs")
            .join("adr")
            .join("ADR-001-mutual-tls-1.3-ecdsa-p256.md"),
        repo_root
            .join("docs")
            .join("adr")
            .join("ADR-002-dual-route-protocol-demarcation.md"),
        repo_root
            .join("docs")
            .join("adr")
            .join("ADR-003-monotonic-attempt-fencing-cas.md"),
        repo_root
            .join("docs")
            .join("adr")
            .join("ADR-004-speculative-preemption-dual-assignment.md"),
        repo_root
            .join("docs")
            .join("adr")
            .join("ADR-005-interface-segregation-admin-vs-worker.md"),
    ];

    for file_path in files_to_check {
        if !file_path.exists() {
            continue; // Will be caught by file existence tests
        }
        let content = fs::read_to_string(&file_path)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", file_path, e));

        assert!(
            !content.contains('\u{2014}'),
            "File {:?} contains forbidden em dash (U+2014)",
            file_path
        );
        assert!(
            !content.contains('\u{2013}'),
            "File {:?} contains forbidden en dash (U+2013)",
            file_path
        );
    }
}

/// Executes the automated traceability verification script and asserts exit code 0.
///
/// Traces to: AG-SPEC-2026-V2, [ISO-29148]
#[test]
fn test_automated_traceability_linter_script() {
    let repo_root = get_repo_root();
    let script_path = repo_root
        .join("tools")
        .join("docs")
        .join("verify_traceability.py");

    assert!(
        script_path.exists(),
        "Verification script must exist at {:?}",
        script_path
    );

    let output = Command::new("python")
        .arg(&script_path)
        .current_dir(&repo_root)
        .output()
        .or_else(|_| {
            Command::new("python3")
                .arg(&script_path)
                .current_dir(&repo_root)
                .output()
        })
        .expect("Failed to execute verification script");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);

    assert!(
        output.status.success(),
        "Traceability verification script failed with code {:?}. Error: {}",
        output.status.code(),
        stderr
    );
}
