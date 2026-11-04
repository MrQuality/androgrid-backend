# Contributing to AndroGrid Control Plane

Thank you for your interest in contributing to AndroGrid! We follow strict engineering standards to ensure high reliability, determinism, and code quality.

---

## Code of Conduct

All contributors are expected to uphold our [Code of Conduct](CODE_OF_CONDUCT.md). Please report any unacceptable behavior following the guidelines in that document.

---

## Development Workflow: GitHub Flow

1. **Fork and Branch:**
   - Always branch off the `main` branch.
   - Use semantic branch names: `feat/feature-name`, `fix/bug-description`, or `chore/task-name`.
   - Never commit directly to `main`.
2. **Test-First Development (TDD):**
   - Write failing unit or integration tests before implementing new features or bug fixes.
   - Verify all tests pass locally using `cargo test` or `cargo nextest run`.
3. **Coding Standards:**
   - Format code using `cargo fmt --check`.
   - Ensure zero warnings with `cargo clippy --all-targets -- -D warnings`.
   - Provide complete docstrings for public functions and structures explaining parameters, returns, errors, and requirement IDs.
   - Punctuation Rule: Never use em dash characters in documentation or commit messages. Use standard ASCII hyphens (-) or colons (:).
4. **Submitting Pull Requests:**
   - Open a pull request against `main`.
   - Describe the architectural reasoning and changes in the PR description.
   - Rebase or squash your commits into clean, semantic commit messages before merging.

---

## Reporting Issues

- Use the GitHub Issue Tracker to report bugs or request features.
- Provide minimal reproducible examples, Rust version (`rustc --version`), and host environment details.
- For security issues, do NOT open a public issue. Refer to [SECURITY.md](SECURITY.md).
