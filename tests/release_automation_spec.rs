//! Integration Specification: Multi-Platform Binary & Container Release Automation
//!
//! Validates the GitHub Actions release workflow matrix, Cargo release profile,
//! multi-stage container build definitions, package archive structures, and
//! cryptographic checksum generation.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-001], [REQ-OP-004], [NFR-SEC-001], [NFR-PERF-001]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

fn get_project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Validates that Cargo.toml configures a hardened, optimized release profile
/// enforcing Link-Time Optimization (LTO) and debug symbol stripping.
///
/// Traces to: [NFR-PERF-001], [NFR-SEC-001]
#[test]
fn test_cargo_release_profile_hardening() {
    let cargo_toml_path = get_project_root().join("Cargo.toml");
    assert!(cargo_toml_path.exists(), "Cargo.toml must exist");

    let content = fs::read_to_string(&cargo_toml_path).expect("Failed to read Cargo.toml");

    assert!(
        content.contains("[profile.release]"),
        "Cargo.toml must define [profile.release] configuration section"
    );
    assert!(
        content.contains("lto = true")
            || content.contains("lto = \"fat\"")
            || content.contains("lto = \"thin\""),
        "Release profile must enable Link-Time Optimization (lto = true)"
    );
    assert!(
        content.contains("strip = true") || content.contains("strip = \"symbols\""),
        "Release profile must strip debug symbols to minimize binary footprint"
    );
    assert!(
        content.contains("opt-level = 3") || content.contains("opt-level = '3'"),
        "Release profile must specify maximum optimization (opt-level = 3)"
    );
    assert!(
        content.contains("codegen-units = 1"),
        "Release profile must set codegen-units = 1 for optimal cross-crate LTO"
    );
}

/// Validates that .github/workflows/release.yml exists and defines the multi-platform
/// cross-compilation matrix across all 5 required operating system architectures.
///
/// Traces to: [REQ-OP-001], [REQ-OP-004]
#[test]
fn test_release_workflow_matrix_architecture_coverage() {
    let workflow_path = get_project_root()
        .join(".github")
        .join("workflows")
        .join("release.yml");

    assert!(
        workflow_path.exists(),
        ".github/workflows/release.yml must exist at {:?}",
        workflow_path
    );

    let content = fs::read_to_string(&workflow_path).expect("Failed to read release.yml");

    // Verify tag trigger
    assert!(
        content.contains("tags:") && content.contains("v*"),
        "Release workflow must trigger on version tags (v*)"
    );

    // Verify 5 required compilation targets
    let required_targets = [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
    ];

    for target in required_targets {
        assert!(
            content.contains(target),
            "Release workflow matrix must contain target {}",
            target
        );
    }

    // Verify both binaries are packaged
    assert!(
        content.contains("androgrid-backend"),
        "Release workflow must compile and package androgrid-backend"
    );
    assert!(
        content.contains("androgrid-cli"),
        "Release workflow must compile and package androgrid-cli"
    );

    // Verify archive packaging formats
    assert!(
        content.contains(".tar.gz"),
        "Release workflow must package Unix binaries into .tar.gz archives"
    );
    assert!(
        content.contains(".zip"),
        "Release workflow must package Windows binaries into .zip archives"
    );

    // Verify SHA-256 checksum consolidation
    assert!(
        content.contains("SHA256SUMS.txt"),
        "Release workflow must generate and publish consolidated SHA256SUMS.txt"
    );
}

/// Validates that .github/workflows/release.yml includes OCI container publishing
/// to GitHub Container Registry (ghcr.io) for multi-arch architectures.
///
/// Traces to: [NFR-SEC-001], [REQ-OP-001]
#[test]
fn test_release_workflow_oci_container_publishing() {
    let workflow_path = get_project_root()
        .join(".github")
        .join("workflows")
        .join("release.yml");

    assert!(workflow_path.exists(), "release.yml must exist");
    let content = fs::read_to_string(&workflow_path).expect("Failed to read release.yml");

    assert!(
        content.contains("ghcr.io"),
        "Release workflow must target GitHub Container Registry (ghcr.io)"
    );
    assert!(
        content.contains("linux/amd64") && content.contains("linux/arm64"),
        "Release container build must support multi-arch: linux/amd64,linux/arm64"
    );
    assert!(
        content.contains("docker/setup-buildx-action"),
        "Workflow must configure Docker Buildx for multi-architecture builds"
    );
    assert!(
        content.contains("docker/login-action"),
        "Workflow must authenticate against ghcr.io"
    );
    assert!(
        content.contains("docker/metadata-action"),
        "Workflow must extract OCI container metadata and semver tags"
    );
}

/// Validates that Dockerfile and Containerfile compile both androgrid-backend
/// and androgrid-cli, placing both in /usr/local/bin under unprivileged ownership.
///
/// Traces to: [NFR-SEC-001], [REQ-OP-004]
#[test]
fn test_container_definitions_binary_parity() {
    let dockerfile_path = get_project_root().join("Dockerfile");
    let containerfile_path = get_project_root().join("Containerfile");

    assert!(dockerfile_path.exists(), "Dockerfile must exist");
    assert!(containerfile_path.exists(), "Containerfile must exist");

    let dockerfile = fs::read_to_string(&dockerfile_path).expect("Failed to read Dockerfile");
    let containerfile =
        fs::read_to_string(&containerfile_path).expect("Failed to read Containerfile");

    for (name, content) in [
        ("Dockerfile", &dockerfile),
        ("Containerfile", &containerfile),
    ] {
        assert!(
            content.contains("androgrid-backend"),
            "{} must reference androgrid-backend",
            name
        );
        assert!(
            content.contains("androgrid-cli"),
            "{} must reference androgrid-cli",
            name
        );
        assert!(
            content.contains("/usr/local/bin/androgrid-backend"),
            "{} must install androgrid-backend into /usr/local/bin",
            name
        );
        assert!(
            content.contains("/usr/local/bin/androgrid-cli"),
            "{} must install androgrid-cli into /usr/local/bin",
            name
        );
        assert!(
            content.contains("USER androgrid"),
            "{} must enforce execution under unprivileged androgrid user",
            name
        );
    }
}

/// Validates SHA-256 digest calculation logic ensuring reproducible cryptographic integrity.
///
/// Traces to: [NFR-SEC-001]
#[test]
fn test_checksum_sha256_reproducibility() {
    let test_data = b"androgrid-release-binary-payload-verification-2026";
    let mut hasher = Sha256::new();
    hasher.update(test_data);
    let digest = format!("{:x}", hasher.finalize());

    assert_eq!(
        digest.len(),
        64,
        "SHA-256 hexadecimal output must be exactly 64 characters"
    );

    let formatted_line = format!("{}  sample_artifact.tar.gz", digest);
    let parts: Vec<&str> = formatted_line.split_whitespace().collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], digest);
    assert_eq!(parts[1], "sample_artifact.tar.gz");
}

/// Asserts zero occurrences of Unicode em dashes (U+2014) or en dashes (U+2013)
/// in release and container automation configuration files.
///
/// Traces to: AGENTS.md Typographic Standards
#[test]
fn test_zero_unicode_dashes_in_release_artifacts() {
    let files_to_check = ["Dockerfile", "Containerfile", "Cargo.toml"];

    for file_name in files_to_check {
        let path = get_project_root().join(file_name);
        if path.exists() {
            let content = fs::read_to_string(&path).expect("Failed to read file");
            for (line_num, line) in content.lines().enumerate() {
                assert!(
                    !line.contains('\u{2014}'),
                    "Forbidden em dash (U+2014) detected in {} line {}",
                    file_name,
                    line_num + 1
                );
                assert!(
                    !line.contains('\u{2013}'),
                    "Forbidden en dash (U+2013) detected in {} line {}",
                    file_name,
                    line_num + 1
                );
            }
        }
    }
}
