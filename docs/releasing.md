# Release maintenance

Forgejo publishes binary releases automatically when the version increases on
`main` and checks pass on that exact commit. GitHub releases remain manual.
The crate is not published to crates.io (`publish = false`). GitHub CI includes
native Windows Rust tests and Clippy. Neither workflow provides native macOS testing.

## Automatic Forgejo releases

[The Forgejo workflow](../.forgejo/workflows/ci.yaml) runs the build and local CLI
test gate on PRs, pushes to `main`, and manual checks. PR jobs have a five-minute
timeout and use a prepared image on Dyad. The image includes the pinned Rust
and Nushell tools and a precompiled dependency cache. Each job writes to its own
container layer; there is no shared writable Cargo cache or container socket.
New dependencies may require rebuilding the image to retain the timing target.

Use the version preparation commands below, commit all three version files,
and merge or push to Forgejo `main`. The workflow compares `release.toml` before
and after the push. An unchanged version skips publication; decreases and
inconsistent version files fail. Initial repository imports do not publish.

The release job has a separate runner slot and a 45-minute timeout. It first runs
`nu --no-config-file scripts/check.nu --full` for maintenance and release-tool
verification, then builds all six targets from `release-targets.toml` with the
pinned cargo-zigbuild image,
using the source commit timestamp for archive metadata. It verifies binary
formats, archive inventory, executable modes, and checksums, and runs the two
x86-64 Linux binaries with `--version`. ARM64 Linux, Windows, and macOS receive
static format validation in this workflow; the optional full local smoke
pipeline below is separate.

Only after local verification does the job create the annotated `fjx/vVERSION`
tag and Forgejo release and upload the twelve archive/checksum assets. It then
downloads the attachments and compares their bytes to the verified local files.
The built-in repository job token supplies authentication; no PAT secret is
needed. Main runs are serialized and are not canceled by later pushes.

If publication fails, inspect the tag, release, and asset state before rerunning
the original workflow. The helpers preserve matching tags, reject mismatched
tags or unexpected metadata/assets, and upload only missing attachments. Writes
are never retried automatically. A completed run verifies existing attachments
again; it does not overwrite them. A manual workflow dispatch runs checks only.

### Maintain the Dyad runners

[ci/Containerfile](../ci/Containerfile) owns the prepared image;
[ci/runner.nix](../ci/runner.nix) packages the installer and runner executable.
Install under Dalton's lingering user account on Dyad. The installer declares
two persistent user services, scoped to `dalton/fjx`, using the existing
`forgejo-pr-podman.service` socket. Job containers are unprivileged, have no
host mounts or socket, and have bounded CPU and memory use. Runtime credentials
live in private `~/.local/state/fjx-runner/{ci,release}/.runner` files.

On Dyad, from a clean checkout, build the image through that rootless service:

```sh
podman --remote --url "unix://${XDG_RUNTIME_DIR}/forgejo-pr-podman/podman.sock" \
  build -f ci/Containerfile -t localhost/fjx-ci:prepared .
```

Build `ci/runner.nix` with the host's nixpkgs (Forgejo Runner 13.2 or newer),
retaining the output through a profile or persistent out-link. Supply its
`fjx-install-runner` command with the immutable local `sha256:` image ID and a
private JSON file containing the result of `fjx api
/repos/dalton/fjx/actions/runners/registration-token --json`. The installer
consumes and deletes that file and reuses existing runner identities on image
updates. Never commit credentials or put them in the image/build context.

## Prepare a version

Use Nushell 0.112.2 with `--no-config-file`. From the repository root:

```sh
nu --no-config-file scripts/release.nu validate
nu --no-config-file scripts/release.nu prepare --dry-run patch
nu --no-config-file scripts/release.nu prepare patch
```

On Linux with Nix, enter `nix-shell` from the repository root before running
these commands to use the pinned Rust toolchain and check utilities described
in the [development instructions](../README.md#development-and-releases).
Container packaging still requires the runtime and cross-build tooling below.

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

Tags use `fjx/vVERSION`, for example `fjx/v0.3.1`. Validate a proposed tag with:

```sh
nu --no-config-file scripts/release.nu validate-tag fjx/v0.3.1
```

For Forgejo, let the workflow create the release tag. For manual publication,
review the intended remote before pushing a tag.

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
nu --no-config-file scripts/package-release.nu verify ./release-assets 0.3.1
```

These operations require GNU coreutils, `tar`, `gzip`, `zip`, `unzip`, and `file`.
Optional full smoke checks also require Docker/Buildx or the supported container
runtime and Wine. The helpers exercise x86-64 Linux in containers, ARM64 Linux
with explicit static QEMU in a container root filesystem, and Windows under
Wine. macOS gets static Mach-O validation only. None of these is native Windows
or macOS runtime CI, and the normal GitHub check workflow does not run this
full release-build/smoke pipeline.

## Publish manually on GitHub

Use the authenticated GitHub CLI for `DaltonAlley/fjx`. The Forgejo publication
helpers below are not compatible with GitHub. Before publication:

1. Commit the prepared version and documentation, merge or push it to `main`,
   and wait for CI on that exact commit to pass.
2. Build all six targets from that commit, using its Unix commit timestamp as
   `--source-date-epoch`, and run the inventory and checksum verification above.
   Record actual runtime checks in the release notes without claiming native
   Windows or macOS coverage from the Linux CI job.
3. Review `git remote get-url origin`, `gh auth status`, and
   `gh repo view --json nameWithOwner,url`. Validate the tag against the prepared
   version and make sure neither the tag nor release already exists.
4. Write release notes outside the source tree and create a draft containing
   all six archives and their six `.sha256` files. For example, from the checked
   release commit:

   ```sh
   git tag -a fjx/v0.3.1 -m "fjx v0.3.1"
   git push origin refs/tags/fjx/v0.3.1
   gh release create fjx/v0.3.1 --repo DaltonAlley/fjx --verify-tag --draft \
     --title "fjx v0.3.1" --notes-file ../release-notes.md ../release-assets/*
   gh release view fjx/v0.3.1 --repo DaltonAlley/fjx
   ```

5. Check that the remote tag points to the intended commit and download the
   draft assets to a fresh directory. Run `package-release.nu verify` on that
   directory before publishing:

   ```sh
   gh release edit fjx/v0.3.1 --repo DaltonAlley/fjx --draft=false --latest
   ```

Verify the published release and its asset inventory once more. If a write
returns an ambiguous result, inspect remote state before deciding whether any
repair is needed. Never overwrite an existing release tag or retry a write
blindly. These manual steps do not add automatic publication to CI.

## Forgejo-specific publication helpers

The inherited `publish-plan`, `push-tag`, `upload-missing-assets`, and
`verify-final` commands use Forgejo API and authentication semantics. They
require `FORGEJO_API_URL`, `FORGEJO_REPOSITORY`, and `FORGEJO_TOKEN` in the
publication environment. They are not a GitHub Releases integration and must
not be pointed at GitHub as though the APIs were interchangeable.

The workflow uses these helpers through `scripts/forgejo-ci.nu`. They support
Forgejo instances hosted below a URL path such as `/forgejo`. Keep credentials
out of logs and command examples, and never retry a write blindly.
