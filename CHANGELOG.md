# Changelog

User-visible changes are recorded here. Version declarations are managed through
[scripts/release.nu](scripts/release.nu). No publication date is asserted for the
current source version.

## Unreleased

### Added

- Read-only GitHub CI and a shared `scripts/check.nu` command for Cargo,
  repository, and release checks.
- Weekly dependency-update proposals, issue forms, and a pull-request checklist.
- MIT license text matching the existing package license, security and conduct
  policies, and consistent editor and line-ending settings.

### Fixed

- Release scripts now locate the standalone checkout without depending on its
  directory name or an absent parent workspace. Fixture overrides remain supported.

### Documentation

- Added source installation and a hidden-token quickstart.
- Added contribution, support, architecture, and release-maintenance guidance.
- Removed inherited parent-workflow claims and clarified standalone CI coverage
  and the absence of automated binary publication.

## 0.2.0

Current source baseline, targeting Forgejo 15.0.7:

- Typed commands for repositories, issues, pull requests, action runs, releases,
  labels, milestones, branches, and workflow dispatch, plus raw API requests.
- Stable plain output and compact JSON, buffered until a final result.
- Unix saved-token authentication and Git credential setup, with environment
  token support for automation and Windows.
- Write previews, explicit destructive-operation confirmation, and redirect refusal.

This baseline summarizes the checked-in functionality, not a reconstructed
release history. See [README.md](README.md) for detailed command contracts.
