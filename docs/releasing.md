# Release maintenance

Forgejo publishes binary releases automatically when the version increases on
`main` and checks pass on that exact commit. Published releases are copied to GitHub.
The crate is not published to crates.io (`publish = false`). GitHub CI includes
native Windows Rust tests and Clippy. Neither workflow provides native macOS testing.

## Automatic Forgejo releases

[The Forgejo workflow](../.forgejo/workflows/ci.yaml) runs the build and local CLI
test gate on PRs, pushes to `main`, and manual checks. PR jobs have a five-minute
timeout and use a prepared image on Dyad. The image includes the pinned Rust,
just, Nushell and Python tools and a precompiled dependency cache. Each job
writes to its own container layer; there is no shared writable Cargo cache or
container socket.
New dependencies may require rebuilding the image to retain the timing target.

Use the version preparation commands below, commit all three version files,
and merge or push to Forgejo `main`. The workflow compares `release.toml` before
and after the push. An unchanged version skips publication; decreases and
inconsistent version files fail. Initial repository imports do not publish.

The release job has a separate runner slot and a 45-minute timeout. It first runs
`just check-full` for maintenance and release-tool
verification, then builds all six targets from `release-targets.toml` with the
pinned cargo-zigbuild image,
using the source commit timestamp for archive metadata. It also sets
`SOURCE_DATE_EPOCH` to that timestamp, so the Windows linker writes it into
the `fjx.exe` PE header instead of the link time, and every build of a commit
gives the same bytes. It verifies binary formats, the `fjx.exe` PE timestamp,
archive inventory, executable modes, and checksums, and runs the two
x86-64 Linux binaries with `--version`. ARM64 Linux, Windows, and macOS receive
static format validation in this workflow; the optional full local smoke
pipeline below is separate.

Only after local verification does the job create a **draft** Forgejo release
and upload the twelve archive/checksum assets. It then checks the draft through
the API: the exact asset names, each asset ID, a same-origin HTTPS download URL,
and each size against the verified local file. Only then does it push the
annotated `fjx/vVERSION` tag and make the release public. The tag comes last
because Forgejo publishes a draft as soon as it syncs a pushed tag with the
same name. The job then downloads every public attachment, without redirects,
and compares its bytes to the verified local file.

The draft check cannot compare bytes. Forgejo serves draft attachments only to
users with write access to releases, and the job token has that access only on
the API, which returns metadata, not file contents. So an upload with the right
size but wrong bytes is caught only after publication: the job fails, but the
release is already public and needs a manual repair.
The built-in repository job token supplies authentication; no PAT secret is
needed for publication. Build/test subprocesses do not inherit that token. Main runs are
serialized and are not canceled by later pushes.

If publication fails, inspect the tag, release, and asset state before rerunning
the original workflow. The helpers preserve matching tags and reject mismatched
tags or unexpected metadata/assets. Every run rebuilds all assets. While the
release is a draft and its tag is not pushed, a rerun deletes all draft
attachments and uploads the new set, because the job cannot read draft bytes
to check old ones. Once the tag is pushed, attachments are never deleted: the
tag push publishes the draft, and a rerun builds the same bytes.
Failed builds create no tag or release; failed uploads or draft checks leave the
release as a draft. A publication request can succeed even if its response is
lost: rerun the original workflow to read the state and verify all assets again,
without republishing or overwriting them. Writes are never retried automatically.
An already public release with missing attachments is rejected for explicit
repair; the job will not add files to a partial public release. A manual workflow
dispatch runs checks only. The publisher independently rechecks the main-push
context, exact revision, and version increase before contacting Forgejo.

### Maintain the Dyad runners

The maintained Dyad deployments live in
[dalton/nixos](https://dyad.tail77425.ts.net/forgejo/dalton/nixos).
`project-ci-runners` owns the `fjx-ci` slot; `fjx-release-runner` owns the
independent `fjx-release` slot. Follow that repository's preparation and scoped
installation instructions. The release image retains the digest-pinned
cargo-zigbuild base from [ci/Containerfile](../ci/Containerfile), with pinned
Rust 1.97.1, all six targets, just 1.51.0, Nushell 0.112.2, and Python 3.
Refresh it when this toolchain or target contract changes.

The release installer prepares the image in the actual rootless
`forgejo-pr-podman.service` store before changing the release unit. It retains
the deployment and image archive as a Nix GC root, keeps the previous deployment
for recovery, and restarts only `forgejo-fjx-release-runner.service`. Every start
checks storage and restores an absent image from the archive. A labelled Podman
retainer protects the prepared image against unused-image cleanup. Preserve both
the GC root and retainer while the runner uses them.

Verify the active unit selects that image, the image exists in that exact store,
and the private registration is preserved. Image presence alone does not prove
the runner is using it. The release slot remains one job, two CPUs, 4 GiB, and
45 minutes. Job containers are unprivileged with no host mounts or socket.
Runtime credentials stay in private `~/.local/state/fjx-runner/{ci,release}/.runner`
files. No whole-system activation is needed. This repository has no runner
installer; use the dalton/nixos deployments above.

## Prepare a version

These commands need just and Python 3.11 or newer. From the repository root:

```sh
just release-validate
just release-prepare --dry-run patch
just release-prepare patch
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
just release-validate-tag fjx/v0.3.1
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
just package ./release-assets x86_64-unknown-linux-gnu
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

Container builds pass the same value as `SOURCE_DATE_EPOCH`. Verification
checks inventory, checksums, archive entries, executable modes, binary formats,
and that the `fjx.exe` PE timestamp equals the given epoch:

```sh
just package-verify ./release-assets 0.3.1 --source-date-epoch 1700000000
```

These operations require GNU coreutils, `tar`, `gzip`, `zip`, `unzip`, and `file`.
Optional full smoke checks also require Docker/Buildx or the supported container
runtime and Wine. The helpers exercise x86-64 Linux in containers, ARM64 Linux
with explicit static QEMU in a container root filesystem, and Windows under
Wine. macOS gets static Mach-O validation only. None of these is native Windows
or macOS runtime CI, and the normal GitHub check workflow does not run this
full release-build/smoke pipeline.

## GitHub mirror

Forgejo is authoritative. Its native push mirror sends all branches and tags to
`DaltonAlley/fjx` after pushes, with hourly reconciliation. It uses a dedicated
GitHub write deploy key. GitHub-only refs can be overwritten or deleted; make
changes on Forgejo. Before activation, the old GitHub branches and annotated
release tags were preserved on Forgejo, including the former GitHub main under
`archive/github/main-20261115`. A verified Git bundle also preserves the old refs.

The release run copies a new release to GitHub in its **Copy releases to
GitHub** job, right after the Forgejo release is final. Forgejo starts no
workflow for events made by the job token, so the release event alone does
not start a copy. [Release mirroring](../.forgejo/workflows/github-mirror.yaml)
is the backup: it listens for published/edited releases and main pushes, and
reconciles every six hours at 00:23, 06:23, 12:23 and 18:23 UTC. It always runs
the script from trusted main, uses the existing `fjx-release` runner, and
serializes runs without cancellation. If a new tag has not reached GitHub yet,
the copier waits up to five minutes for it.
This consumes the existing version-bump publisher's public releases, without
rebuilding binaries. The separate `GH_RELEASE_MIRROR_TOKEN` Actions secret is
only supplied to the copy step. A repository-scoped GitHub token needs Contents
write permission; unlike code mirroring, release copying needs no Workflows scope.

Every run scans all published Forgejo releases, including initial backfill.
[The copier](../scripts/github-mirror.nu) checks exact source/destination tag
hashes, all twelve asset names, archive checksums and downloaded byte parity.
New GitHub releases remain drafts until verification finishes. Source drafts
are skipped, prerelease flags are retained, and prereleases are never marked
latest. Stable releases use GitHub's version-based legacy latest selection.
Earlier GitHub-only releases are retained. Issues, PR discussions, CI history,
wiki content, repository settings, secrets and package registries are not copied.

Rerun **Mirror GitHub releases** or dispatch `github-mirror.yaml` on `main` to
recover a missed event or interrupted upload. The scheduled scan also recovers
these cases. Reruns read server state. If the GitHub release is still a draft,
they delete its assets and upload all twelve again, then verify the bytes.
Exact public releases require no writes. Conflicting assets, unexpected files,
mismatched tags and incomplete public GitHub releases stop with an Actions
failure for explicit repair. The copier deletes assets only from GitHub drafts;
it never deletes or overwrites a public asset, a tag or a release. Release
withdrawals/deletions are not propagated; handle those explicitly on both
services. This is a mirror, not an archival backup of all Forgejo platform state.

## Forgejo-specific publication helpers

The `publish-plan`, `push-tag`, `delete-draft-assets`, `upload-missing-assets`,
`publish-release`, and `verify-final` commands use Forgejo API and
authentication semantics. They require
`FORGEJO_API_URL`, `FORGEJO_REPOSITORY`, and `FORGEJO_TOKEN` in the
publication environment. They are not a GitHub Releases integration and must
not be pointed at GitHub as though the APIs were interchangeable.

The workflow uses these helpers through `scripts/forgejo-ci.nu`. They support
Forgejo instances hosted below a URL path such as `/forgejo`. Keep credentials
out of logs and command examples, and never retry a write blindly.
