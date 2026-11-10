#!/usr/bin/env python3
"""
ISO/IEC/IEEE 29148 Traceability & Architecture Decision Record Verification Linter

Validates:
1. 100% coverage of master specification requirements in docs/traceability_matrix.md.
2. Bidirectional file link integrity between the matrix and filesystem.
3. Conformance of Control Plane ADRs to Michael Nygard format.
4. Typographic standards: Zero Unicode em dashes (U+2014) or en dashes (U+2013).

Traces to: AG-SPEC-2026-V2, [ISO-29148]
"""

import sys
import re
from pathlib import Path

MANDATORY_REQUIREMENTS = [
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
]

EXPECTED_ADRS = [
    ("ADR-001-mutual-tls-1.3-ecdsa-p256.md", "[INV-002]"),
    ("ADR-002-dual-route-protocol-demarcation.md", "[REQ-OP-001]"),
    ("ADR-003-monotonic-attempt-fencing-cas.md", "[INV-004]"),
    ("ADR-004-speculative-preemption-dual-assignment.md", "[INV-005]"),
    ("ADR-005-interface-segregation-admin-vs-worker.md", "[REQ-OP-001]"),
]


def find_repo_root() -> Path:
    current = Path.cwd()
    if (current / "docs" / "traceability_matrix.md").exists():
        return current
    if (current / "androgrid-backend" / "docs" / "traceability_matrix.md").exists():
        return current / "androgrid-backend"
    for parent in current.parents:
        if (parent / "docs" / "traceability_matrix.md").exists():
            return parent
        if (parent / "androgrid-backend" / "docs" / "traceability_matrix.md").exists():
            return parent / "androgrid-backend"
    return current


def verify_traceability_matrix(repo_root: Path) -> list:
    errors = []
    matrix_path = repo_root / "docs" / "traceability_matrix.md"
    if not matrix_path.exists():
        errors.append(f"Traceability matrix not found at {matrix_path}")
        return errors

    content = matrix_path.read_text(encoding="utf-8")
    for req in MANDATORY_REQUIREMENTS:
        if req not in content:
            errors.append(f"Missing mandatory requirement {req} in traceability matrix")

    # Verify file paths inside backticks
    tokens = content.split("`")
    checked_files = 0
    for token in tokens:
        token = token.strip()
        if any(token.endswith(ext) for ext in [".rs", ".proto", ".py", ".md"]) and (
            "/" in token or "\\" in token
        ):
            if (
                token.startswith("http")
                or "vendor" in token
                or "std/" in token
            ):
                continue
            target_file = repo_root / token
            if not target_file.exists():
                errors.append(f"Matrix references non-existent file on disk: {token}")
            else:
                checked_files += 1

    if checked_files < 10:
        errors.append(
            f"Expected at least 10 valid file references in matrix, found {checked_files}"
        )

    return errors


def verify_control_plane_adrs(repo_root: Path) -> list:
    errors = []
    adr_dir = repo_root / "docs" / "adr"
    if not adr_dir.exists():
        errors.append(f"ADR directory not found at {adr_dir}")
        return errors

    for adr_name, expected_trace in EXPECTED_ADRS:
        adr_file = adr_dir / adr_name
        if not adr_file.exists():
            errors.append(f"Missing ADR file: {adr_file}")
            continue

        text = adr_file.read_text(encoding="utf-8")
        if not text.startswith("# ADR-00"):
            errors.append(f"{adr_name}: must start with '# ADR-00'")
        if "## Status" not in text or "Accepted" not in text:
            errors.append(f"{adr_name}: Status must be 'Accepted'")
        if "## Context" not in text:
            errors.append(f"{adr_name}: Missing '## Context' section")
        if "## Decision" not in text:
            errors.append(f"{adr_name}: Missing '## Decision' section")
        if "## Consequences" not in text:
            errors.append(f"{adr_name}: Missing '## Consequences' section")
        if "## Alternatives Considered" not in text:
            errors.append(f"{adr_name}: Missing '## Alternatives Considered' section")
        if expected_trace not in text:
            errors.append(f"{adr_name}: Missing traceability tag {expected_trace}")

    return errors


def verify_typographic_standards(repo_root: Path) -> list:
    errors = []
    paths_to_audit = [
        repo_root / "docs",
        repo_root / "src",
        repo_root / "tests",
    ]

    for root_dir in paths_to_audit:
        if not root_dir.exists():
            continue
        for file_path in root_dir.rglob("*"):
            if not file_path.is_file():
                continue
            if file_path.suffix not in [".rs", ".md", ".proto", ".py", ".toml"]:
                continue
            # Skip binary or image files
            try:
                content = file_path.read_text(encoding="utf-8")
            except (UnicodeDecodeError, PermissionError):
                continue

            if "\u2014" in content:
                errors.append(
                    f"{file_path.relative_to(repo_root)}: contains forbidden em dash (U+2014)"
                )
            if "\u2013" in content:
                errors.append(
                    f"{file_path.relative_to(repo_root)}: contains forbidden en dash (U+2013)"
                )

    return errors


def main():
    repo_root = find_repo_root()
    print(f"=== ISO 29148 Traceability & ADR Linter ===")
    print(f"Repository Root: {repo_root}")

    all_errors = []

    print("[1/3] Verifying ISO 29148 Traceability Matrix...")
    matrix_errors = verify_traceability_matrix(repo_root)
    if matrix_errors:
        all_errors.extend(matrix_errors)
        for err in matrix_errors:
            print(f"  FAIL: {err}")
    else:
        print(f"  PASS: 100% requirements covered, file paths verified.")

    print("[2/3] Verifying Host Control Plane ADRs (Michael Nygard format)...")
    adr_errors = verify_control_plane_adrs(repo_root)
    if adr_errors:
        all_errors.extend(adr_errors)
        for err in adr_errors:
            print(f"  FAIL: {err}")
    else:
        print(f"  PASS: All 5 Control Plane ADRs valid and accepted.")

    print("[3/3] Auditing Typographic Standards (Zero Em/En Dashes)...")
    typo_errors = verify_typographic_standards(repo_root)
    if typo_errors:
        all_errors.extend(typo_errors)
        for err in typo_errors:
            print(f"  FAIL: {err}")
    else:
        print(f"  PASS: 0 Unicode em/en dashes found.")

    print("------------------------------------------")
    if all_errors:
        print(f"TOTAL FAILURES: {len(all_errors)}")
        sys.exit(1)
    else:
        print("TRACEABILITY & ADR VERIFICATION: SUCCESS (Exit code 0)")
        sys.exit(0)


if __name__ == "__main__":
    main()
