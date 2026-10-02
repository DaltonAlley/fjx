# Contributing to fjx

Bug reports, documentation fixes, and focused patches are welcome. Read
[AGENTS.md](AGENTS.md) and the [code of conduct](CODE_OF_CONDUCT.md) first.
For a new command, dependency, or behavior change, open an issue to discuss the
use case before investing in a large implementation.

## Build and check

Use Rust 1.97.1 from `rust-toolchain.toml` and Nushell 0.112.2. Run commands from
the repository root. Linux tests need Git, Bash, and `script` from util-linux.
Release tests also use GNU coreutils, `tar`, `gzip`, `zip`, and `unzip`.
Real release-asset verification additionally needs `file`.
The normal checks do not require a live Forgejo account or release credentials.

```sh
cargo build --locked
nu --no-config-file scripts/check.nu
```

The shared check script matches standalone GitHub CI on Ubuntu 24.04:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps
nu --no-config-file tests/repository.nu
nu --no-config-file tests/release.nu
nu --no-config-file scripts/release.nu validate
```

The `RUSTDOCFLAGS` line uses POSIX shell syntax. Native Windows and macOS CI,
and automated binary publication, are not currently provided.

## Keep changes small and testable

- Keep this a synchronous, single-binary crate without a public library API.
- Runtime dependencies are limited to `lexopt`, `ureq` with Rustls, `serde`,
  `serde_json`, and `rpassword`. Do not add one without discussing the constraint.
- Keep host, repository, raw-path, config, HTTP, and output rules in their owning
  modules. See [architecture](docs/architecture.md).
- Never expose tokens, follow HTTP redirects, retry writes, or emit stdout before
  a command has a final result.
- Add regression tests for bug fixes and behavior changes. Cover failure paths,
  empty stdout on errors, token redaction, and write safety where relevant.
- Update help text and the README when command or output contracts change.
  Record user-visible changes under Unreleased in [CHANGELOG.md](CHANGELOG.md).
- Change versions only through the [release procedure](docs/releasing.md).

## Submit a pull request

Explain the problem, the proposed behavior, and the checks you ran. Include a
small reproduction for bug fixes. Keep unrelated cleanup separate and avoid
committing generated build or release artifacts. Tests use local fixtures and
mock servers, so never put real tokens or private server data in a test.

For non-security reports, include the information in [SUPPORT.md](SUPPORT.md).
Report suspected vulnerabilities through [SECURITY.md](SECURITY.md), not a
public issue.
