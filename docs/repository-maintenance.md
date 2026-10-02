# Repository maintenance

This guide records why the repository is organized this way and what still needs
owner configuration. The goal is a useful, verifiable small project, not a large
collection of badges or bots.

## Research and decisions

Reviewed on October 2, 2026. These are examples to learn from, not policies copied
wholesale or endorsements by the upstream projects.

| Source | Useful pattern | Application in fjx |
| --- | --- | --- |
| [GitHub community profiles](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/about-community-profiles-for-public-repositories) | Make project purpose, licensing, contribution expectations, and support discoverable. | Root README, MIT license text matching the existing Cargo declaration, contribution, support, conduct, and security files. |
| [ripgrep README](https://github.com/BurntSushi/ripgrep/blob/master/README.md) | Explain what the CLI does, how to install it, and how to build it. | A short introduction and reproducible source-install instructions, without claiming a crates.io package or existing binary releases. |
| [bat contributor guide](https://github.com/sharkdp/bat/blob/master/CONTRIBUTING.md) | Discuss substantial features first, add regression tests, and record user-visible changes. | Contributor guide, focused PR checklist, and an honest changelog without invented release history. |
| [fd repository](https://github.com/sharkdp/fd) | Offer README navigation and examples, with separate contributor, changelog, and security documents. | User-facing entry points separated from architecture and release details. |
| [GitHub Actions security guidance](https://docs.github.com/en/actions/reference/security/secure-use) | Minimize token permissions, pin action revisions, and keep untrusted PR code out of privileged workflows. | Read-only CI, SHA-pinned checkout without persisted credentials, checksum-verified Nu, and no publishing secrets. |
| [Dependabot configuration](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference) | Keep dependency updates visible and reviewable. | Weekly Cargo and Actions update proposals with a small open-PR limit, not automatic merges. |

The existing release scripts and tests were imported from a larger workspace.
The standalone repository now resolves its own root instead of requiring a
parent `fjx/` directory. Tests for missing parent Forgejo workflows were replaced
with local checks. Provider-specific Forgejo publication helpers remain separate
from GitHub CI. See [releasing](releasing.md) before using them.

## Checks and maintenance

- Run `nu --no-config-file scripts/check.nu` before merging. CI runs the same
  entry point on Linux for pull requests, pushes to `main`, and manual requests.
- Keep `rust-toolchain.toml`, the release container's Rust toolchain, and the
  contributor prerequisites aligned when upgrading Rust.
- When updating Nushell, review its release notes, verify the archive checksum
  against the upstream release, and update CI, the check script, and docs together.
- Review Dependabot PRs normally. Preserve the runtime dependency allowlist and
  exact version pins. Review both manifest and lockfile changes, then run checks.
- Container-image digests and the inline Nu archive checksum are reviewed
  manually. The current Dependabot config does not update those pins.
- Keep a changelog entry for user-visible behavior changes. Do not change the
  release version just to update repository documentation.

Repository checks parse the workflow and issue forms, check local Markdown file
links, and guard important CI permissions and package invariants. They are not
a replacement for an actual GitHub Actions run or a complete Markdown renderer.

## Owner setup checklist

These settings are not enabled by committing files. Confirm them in GitHub after
pushing and observing the first successful workflow run:

- [ ] Enable Actions for this repository and review the default token permissions.
  Keep the workflow token read-only and do not allow it to approve PRs.
- [ ] Add a rule for `main` requiring the `Rust and Nushell checks` status check
  and resolved conversations. Block force pushes and deletion. Review the bypass
  policy explicitly. Requiring approval from another person is useful once there
  is another active reviewer, but can lock a sole maintainer out of their own PRs.
- [ ] [Enable private vulnerability reporting](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/configure-for-a-repository).
  Verify the report button is available, then keep `SECURITY.md` accurate.
- [ ] Review dependency-graph, Dependabot alert, and secret-scanning settings
  available to the repository. Notifications must reach someone who can act on them.
- [ ] Set a short repository description and relevant topics such as `forgejo`,
  `rust`, `cli`, and `automation`.
- [ ] Choose and publish a private moderation contact if one becomes available.
- [ ] Before offering downloadable releases, decide where they are published and
  validate every promised platform. The current CI checks source and release
  logic, not a six-platform binary publication pipeline.

## Deliberately not added

- No automatic release, tag push, crates.io publication, or dependency auto-merge.
- No invented security mailbox, response SLA, coverage percentage, or support matrix.
- No native Windows/macOS claim based on Linux tests or static artifact checks.
- No extra runtime dependency, public Rust library, asynchronous runtime, or
  project-wide build system just to support repository administration.
- No stale-issue bot, funding links, or mandatory approval by a fictitious team.
