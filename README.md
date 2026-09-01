# fjx

`fjx` is a small synchronous Forgejo client for people and scripts. Version 0.2.0 targets Forgejo 15.0.7.

The first three commands below show the Unix saved-login workflow. On Windows, set `FJX_TOKEN` or `FORGEJO_TOKEN` instead.

```text
fjx auth login --host https://forgejo.example --with-token
fjx auth status --json
fjx auth setup-git --host https://forgejo.example
fjx repo view -R owner/repo
fjx issue list -R owner/repo --all --json
fjx issue create -R owner/repo --title "Bug" --body-file report.md
fjx pr create -R owner/repo --head feature --title "Change"
fjx pr checks -R owner/repo 12 --json
fjx run watch -R owner/repo 42 --poll 5 --wait 600
fjx release list -R owner/repo --all
fjx release view -R owner/repo v1.2.3 --json
fjx release create -R owner/repo --tag v1.2.3 --title "fjx 1.2.3" --target main
fjx release upload -R owner/repo 7 ./fjx.tar.gz --name fjx-v1.2.3.tar.gz
fjx label list -R owner/repo --all
fjx label create -R owner/repo --name bug --color '#d73a4a' --description "Needs fixing"
fjx milestone list -R owner/repo --state all
fjx milestone create -R owner/repo --title v1.2 --due 2026-09-30T23:59:59Z
fjx branch list -R owner/repo --all
fjx branch delete -R owner/repo feature/old --yes
fjx workflow dispatch -R owner/repo release.yml --ref main --field channel=stable
fjx api /version --json
fjx api /repos/owner/repo/issues --paginate --json
```

Common flags may go before or after command words: `--host URL`, `-R OWNER/REPO`, `--json`, `--dry-run`, and `--yes`. `FJX_HOST`, `FJX_REPO`, `FJX_TOKEN`, and `FORGEJO_TOKEN` provide non-interactive context. On Unix, `FJX_CONFIG` selects the token store.

On Unix, `auth login` takes a token from a hidden prompt. Pass `--with-token` to read one trimmed line from stdin. `auth setup-git` installs a host-scoped Git credential helper so Git and `jj git push` can use the same saved login. Unix token files use mode 0600.

On Windows, use `FJX_TOKEN` or `FORGEJO_TOKEN`; commands can run without `HOME`, `APPDATA`, or `USERPROFILE` when `--host` or `FJX_HOST` supplies the host. Persisted config and `auth login`, `auth logout`, and `auth setup-git` are unavailable because fjx cannot verify private file access with its current dependency set.

Tokens never appear in command arguments or user-facing output. The client accepts HTTPS hosts and loopback HTTP hosts and rejects redirects.

Raw `api` paths sit below the selected host's `/api/v1/` path. They must stay relative. Raw DELETE requires `--yes`. Write requests accept `--dry-run`, which prints the planned request without sending it.

Typed commands cover issue list, view, create, comment, close, and reopen; pull-request list, view, create, diff, checks, comment, review, merge, close, and reopen; action-run list, view, and watch; release list, view, create, and upload; label list and create; milestone list and create; branch list and delete; and workflow dispatch. Lists default to 30 open items where state applies. `--all` follows Forgejo pages up to 1,000 items and fails instead of returning a cut-off list. Forgejo 15.0.7 has no draft field in its create-pull request body, so `pr create --draft` uses its default `WIP:` title prefix in the one create request.

`release view` identifies a release by tag. `release upload` instead requires the positive numeric Forgejo release ID returned by release list, view, or create; a tag is not accepted in that position. Uploads stream a regular file as `application/octet-stream`, default the asset name to the file name, and let `--name` override it. Workflow dispatch requires a workflow file and ref, accepts repeated unique `--field KEY=VALUE` inputs, and returns the created run ID, run number, and jobs.

Every typed write accepts `--dry-run`, which validates local inputs and prints the planned request without sending it. Pull-request merge, branch delete, and raw DELETE require `--yes`, including for a dry run; other typed writes reject `--yes`. Read commands reject both write-safety flags.

`pr checks` exits 1 unless all normalized checks succeed. `run watch` buffers output until the run ends: it emits one final record, exits 0 only for success, and exits 1 for every other conclusion. A timeout or poll error leaves stdout empty.

Plain output is stable and short. Each tab separates fields and each line feed ends a record. Within server-provided fields, backslash, tab, carriage return, and line feed encode as `\\`, `\t`, `\r`, and `\n`; every other Unicode control scalar encodes as `\u{HEX}` with uppercase hex and no leading zeroes. This encoding is reversible, so text that looks like an escape starts with `\\`. `--json` emits one compact JSON value with the original field values and no plain-output encoding. Errors go to stderr and leave stdout empty. See `fjx --help` for exit classes.

## Release

`nu --no-config-file scripts/release.nu validate` checks the three version declarations. `nu --no-config-file scripts/release.nu prepare [--dry-run] patch|minor|major` updates them as one transaction. `nu --no-config-file scripts/package-release.nu OUTPUT_DIR [TARGET]` builds one target or all six published targets and writes an archive plus `.sha256` for each:

- `x86_64-unknown-linux-gnu` (`tar.gz`)
- `aarch64-unknown-linux-gnu` (`tar.gz`)
- `x86_64-unknown-linux-musl` (`tar.gz`)
- `aarch64-unknown-linux-musl` (`tar.gz`)
- `aarch64-apple-darwin` (`tar.gz`)
- `x86_64-pc-windows-gnu` (`zip`, containing `fjx.exe`)

Set `FJX_PACKAGE_BIN_ROOT` to a directory containing target-named subdirectories with `fjx` (`fjx.exe` for Windows) to package prebuilt files. Otherwise the script uses `podman` (or `FJX_CONTAINER_RUNTIME`) with `packaging/release.Containerfile`.

Release verification checks the complete asset inventory, checksums, archive contents, executable bits where applicable, and binary formats. CI smoke-runs x86-64 Linux in a container and ARM64 Linux with a pinned static QEMU interpreter inside an ARM64 container root filesystem, without relying on host `binfmt_misc`; it runs the Windows build under Wine. The macOS ARM64 artifact receives static Mach-O format validation only; CI does not run it on macOS hardware. These checks do not claim native Windows or macOS runtime coverage.

Release scripts require Nushell 0.112.2 and should always run with
`--no-config-file`. The Forgejo workflow installs Nu from the official
digest-pinned container action
`docker://ghcr.io/nushell/nushell@sha256:cda9491fdc5b7a74d713340c198c9c53b23431a0e6671f7c06809edb76a43b2c`.
It copies the runtime into `.act-setup-nu-runtime`, verifies version `0.112.2`,
and sets each repository script step shell to
`./.act-setup-nu-runtime/nu --no-config-file {0}`. The trusted pull-request
final gate is intentionally different: it has exactly two inline runner-shell
steps and uses no checkout, action, repository code, or Nu runtime.
