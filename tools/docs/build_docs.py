#!/usr/bin/env python3
"""
Public Documentation Portal Builder and Link Validator
Traces to: [NFR-DOC-001], [REQ-PUB-001]

Validates that all documentation chapters exist, markdown links resolve,
and zero forbidden Unicode dashes exist. Optionally executes `mdbook build`.
"""

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path


def audit_typographic_standards(docs_root: Path) -> int:
    """Enforces zero em/en dash rule across all documentation files."""
    errors = 0
    forbidden_pattern = re.compile(r"[\u2013\u2014]")

    for path in docs_root.rglob("*"):
        if path.is_file() and path.suffix in [".md", ".toml", ".yml", ".js"]:
            with open(path, "r", encoding="utf-8") as f:
                for line_idx, line in enumerate(f, start=1):
                    match = forbidden_pattern.search(line)
                    if match:
                        print(
                            f"[ERROR] Forbidden Unicode dash {repr(match.group(0))} in "
                            f"{path.relative_to(docs_root.parent)}:{line_idx}: {line.strip()}"
                        )
                        errors += 1
    return errors


def validate_summary_links(docs_root: Path) -> int:
    """Verifies that all links in SUMMARY.md point to valid markdown files."""
    summary_path = docs_root / "src" / "SUMMARY.md"
    if not summary_path.exists():
        print(f"[ERROR] SUMMARY.md not found at {summary_path}")
        return 1

    errors = 0
    link_pattern = re.compile(r"\[.*?\]\((.*?\.md)\)")

    with open(summary_path, "r", encoding="utf-8") as f:
        content = f.read()

    links = link_pattern.findall(content)
    if not links:
        print("[ERROR] No markdown links found in SUMMARY.md")
        return 1

    print(f"[INFO] Found {len(links)} links in SUMMARY.md")
    for link in links:
        target_path = docs_root / "src" / link
        if not target_path.exists():
            print(f"[ERROR] Missing chapter file referenced in SUMMARY.md: {target_path}")
            errors += 1
        elif target_path.stat().st_size == 0:
            print(f"[ERROR] Chapter file is empty: {target_path}")
            errors += 1

    return errors


def build_mdbook(docs_root: Path) -> int:
    """Executes mdbook build if binary is available."""
    mdbook_bin = shutil.which("mdbook")
    if not mdbook_bin:
        print("[WARN] mdbook executable not detected in PATH; skipping static rendering.")
        return 0

    print(f"[INFO] Executing mdbook build using {mdbook_bin}...")
    res = subprocess.run([mdbook_bin, "build", str(docs_root)], capture_output=True, text=True)
    if res.returncode != 0:
        print(f"[ERROR] mdbook build failed:\n{res.stderr}")
        return res.returncode

    print("[SUCCESS] mdbook build completed successfully.")
    return 0


def main() -> int:
    backend_root = Path(__file__).resolve().parent.parent.parent
    docs_root = backend_root / "docs"

    if not docs_root.exists():
        print(f"[ERROR] docs root not found at {docs_root}")
        return 1

    print("==========================================================")
    print("AndroGrid Documentation Portal Validator & Builder")
    print("==========================================================")

    typo_errors = audit_typographic_standards(docs_root)
    link_errors = validate_summary_links(docs_root)
    build_code = build_mdbook(docs_root)

    total_errors = typo_errors + link_errors + build_code
    if total_errors == 0:
        print("[SUCCESS] All documentation audits passed with 0 errors.")
        return 0
    else:
        print(f"[FAILED] Documentation audit failed with {total_errors} errors.")
        return 1


if __name__ == "__main__":
    sys.exit(main())
