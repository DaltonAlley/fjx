# FJX Invariants

This document records the durable rules that project changes must preserve.
Cross-project rules remain in the user's global constitution.

## INV-binary-boundary: One synchronous executable

- **Scope:** Crate architecture and runtime dependencies.
- **Rule:** Keep fjx synchronous, a single binary with no public library API, and runtime dependencies limited to the allowlist in AGENTS.md. Keep host, repository, raw-path, config, HTTP, and output policy in their owning modules.
- **Owner:** Cargo.toml and the module boundaries documented in docs/architecture.md.
- **Evidence:** Cargo checks, tests/repository.nu, and review of module ownership.
- **Origin:** Contributor instructions and documented architecture.

## INV-safe-results: Safe requests and complete output

- **Scope:** Authentication, HTTP operations, and command output.
- **Rule:** Never disclose tokens, follow HTTP redirects, retry a write, or emit command stdout before a final result exists. Keep failures on stderr with the documented exit classes.
- **Owner:** Config, HTTP, output, and command execution boundaries; authenticated release helpers.
- **Evidence:** CLI integration tests and publication fixtures in tests/release.nu.
- **Origin:** Contributor instructions, README output contracts, and docs/architecture.md.

## INV-release-identity: Consistent source and release identity

- **Scope:** Version preparation, packaging, and publication.
- **Rule:** Update release.toml, Cargo.toml, and the fjx lockfile entry together using scripts/release.nu. Release tags must identify the checked commit and may never be moved. Keep README and release instructions aligned with scripts and actual CI coverage.
- **Owner:** scripts/release.nu, release-targets.toml, and documentation.
- **Evidence:** Version validation and the transaction, tag, metadata, and asset tests in tests/release.nu.
- **Origin:** Contributor instructions and documented release contracts.

## INV-forgejo-ci: Fast PR checks and version-triggered releases

- **Scope:** Forgejo checks and release automation.
- **Rule:** Check pull requests with the complete project check entry point and aim to finish in under five minutes. Automate a release when the version increases on main, after checks pass for that exact commit. Ordinary commits and initial imports must not publish. Keep release builds outside PR checks.
- **Owner:** .forgejo/workflows/ci.yaml, scripts/forgejo-ci.nu, and the dedicated Dyad runners.
- **Evidence:** tests/forgejo-ci.nu, workflow policy checks in tests/repository.nu, and measured Forgejo runs. Timing is a target dependent on runner load, not a guarantee.
- **Origin:** User request to set up Forgejo CI, release on version bumps, and target PR checks below five minutes; user selected Dyad for the runner.
