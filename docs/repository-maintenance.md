# Repository maintenance

FJX keeps repository setup small: a useful README, license, repeatable checks,
and release instructions. The files below record the research behind those
choices and the settings that cannot be enabled by a commit.

## Research and decisions

Reviewed October 2, 2026.

| Source | Practice applied to fjx |
| --- | --- |
| [GitHub community profiles](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/about-community-profiles-for-public-repositories) | Make the purpose, license, and build path easy to find in the root README and LICENSE. Not every optional community file is useful for this repository. |
| [ripgrep](https://github.com/BurntSushi/ripgrep/blob/master/README.md) and [fd](https://github.com/sharkdp/fd) | Show installation and working examples before implementation details; link to focused architecture and release notes. |
| [GitHub Actions guidance](https://docs.github.com/en/actions/reference/security/secure-use) | Run checks with read-only permissions, commit-pinned actions, no persisted credentials, and pinned tool versions. |
| [Dependabot configuration](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference) | Propose weekly Cargo and Actions updates for review, without automatic merging. |

The release scripts came from a larger workspace. They now resolve the
standalone repository root, and tests no longer assert the presence of missing
parent Forgejo workflows. Forgejo now has its own workflow, prepared runners on
Dyad, and automatic version-triggered publication; see [releasing](releasing.md).

## Routine checks

- Run `just check` before merging. GitHub CI runs the same
  entry point on Linux for pull requests, pushes to `main`, and manual runs.
- Keep `rust-toolchain.toml`, the release container's Rust toolchain, and the
  README's build prerequisites aligned when upgrading Rust.
- When updating Nushell, review its release notes, verify the archive checksum
  against upstream, and update `ci/Containerfile`, `shell.nix`, the check
  script, and README together.
- Review Dependabot proposals and keep the runtime dependency allowlist and
  exact pins. Review manifest and lockfile changes together, then run checks.
- Review pinned container images, the Nu and just archive checksums, and the
  `just` version in GitHub CI manually; Dependabot does not update those pins.

Repository checks parse the workflow, check local Markdown links, and guard
CI permissions and package invariants. They do not replace a real GitHub
Actions run or a complete Markdown renderer.
