# Release maintenance

The source version is 0.2.0. This repository does not currently publish binary
assets automatically or publish to crates.io (`publish = false`). GitHub CI
checks the project on Ubuntu 24.04, not native Windows or macOS. Packaging and
publication helpers are tooling, not evidence of a published release.

## Prepare a version

Use Nushell 0.112.2 with `--no-config-file`. From the repository root:

```sh
nu --no-config-file scripts/release.nu validate
nu --no-config-file scripts/release.nu prepare --dry-run patch
nu --no-config-file scripts/release.nu prepare patch
```

Use `minor` or `major` instead of `patch` when appropriate. The preparation
script updates `release.toml`, `Cargo.toml`, and the `fjx` package entry in
`Cargo.lock` as a transaction with rollback on failure. Do not edit just one
version declaration. Inspect the diff, update user-facing documentation when
needed, and run the [project checks](../README.md#development-and-releases)
before committing.

The release script finds the standalone repository root from its own location,
not the current directory or checkout name. `RELEASE_WORKSPACE_ROOT` is a
legacy fixture override that expects a parent directory containing `fjx/`;
leave it unset for ordinary standalone use.

Tags use `fjx/vVERSION`, for example `fjx/v0.2.0`. Validate a proposed tag with:

```sh
nu --no-config-file scripts/release.nu validate-tag fjx/v0.2.0
```

Do not push tags or publish assets until the intended remote and release process
have been reviewed.

## Package locally

`release-targets.toml` declares six packaging targets:

| Target | Archive |
| --- | --- |
| `x86_64-unknown-linux-gnu` | `tar.gz` |
| `aarch64-unknown-linux-gnu` | `tar.gz` |
| `x86_64-unknown-linux-musl` | `tar.gz` |
| `aarch64-unknown-linux-musl` | `tar.gz` |
| `aarch64-apple-darwin` | `tar.gz` |
| `x86_64-pc-windows-gnu` | `zip`, containing `fjx.exe` |

```sh
nu --no-config-file scripts/package-release.nu ./release-assets x86_64-unknown-linux-gnu
```

Omit the target or use `all` for all six. Each archive has a `.sha256` file.
The script builds with `packaging/release.Containerfile` using Podman by default,
or the runtime selected by `FJX_CONTAINER_RUNTIME`. The explicit `build`
subcommand uses Docker by default. Container builds need the configured runtime
and cross-build tooling, not just Cargo on the host.

To package existing binaries, set `FJX_PACKAGE_BIN_ROOT` to a directory with
one target-named subdirectory per target, containing `fjx` (`fjx.exe` on Windows).
Use `--source-date-epoch` with a positive Unix timestamp for reproducible archive
metadata. Keep generated assets out of source commits.

Verification checks inventory, checksums, archive entries, executable modes,
and binary formats:

```sh
nu --no-config-file scripts/package-release.nu verify ./release-assets 0.2.0
```

These operations require GNU coreutils, `tar`, `gzip`, `zip`, `unzip`, and `file`.
Optional full smoke checks also require Docker/Buildx or the supported container
runtime and Wine. The helpers exercise x86-64 Linux in containers, ARM64 Linux
with explicit static QEMU in a container root filesystem, and Windows under
Wine. macOS gets static Mach-O validation only. None of these is native Windows
or macOS runtime CI, and the normal GitHub check workflow does not run this
full release-build/smoke pipeline.

## Publication is Forgejo-specific

The inherited `publish-plan`, `push-tag`, `upload-missing-assets`, and
`verify-final` commands use Forgejo API and authentication semantics. They
require `FORGEJO_API_URL`, `FORGEJO_REPOSITORY`, and `FORGEJO_TOKEN` in the
publication environment. They are not a GitHub Releases integration and must
not be pointed at GitHub as though the APIs were interchangeable.

Before adding automated publication, review authentication, tag protection,
asset provenance, permissions, and platform coverage using the
[repository maintenance checklist](repository-maintenance.md). Keep credentials
out of logs and command examples, and never retry a write blindly.
