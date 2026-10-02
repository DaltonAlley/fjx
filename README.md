# fjx

`fjx` is a small synchronous Forgejo client for people and scripts. Version 0.2.0 targets Forgejo 15.0.7.
Manage issues, pull requests, action runs, and releases from your terminal, with
stable plain output and compact JSON for automation.

## Install from source

Install Git and Rust 1.97.1, pinned in [rust-toolchain.toml](rust-toolchain.toml).

```sh
git clone https://github.com/DaltonAlley/fjx.git
cd fjx
cargo install --path . --locked
fjx --version
```

Make sure Cargo's binary directory, normally `$HOME/.cargo/bin`, is on `PATH`.
The crate has `publish = false`: crates.io installation and published binary
assets are not currently provided. To build without installing, run
`cargo build --locked --release` and use `target/release/fjx` (`fjx.exe` on Windows).

## Quickstart

On Unix, create a token on your Forgejo server with permissions for your commands,
then enter it at the hidden prompt:

```sh
fjx auth login --host https://forgejo.example
fjx auth status --json
fjx repo view -R owner/repo
fjx issue list -R owner/repo --all --json
# Create report.md with the issue description before previewing the request.
fjx issue create -R owner/repo --title "Bug" --body-file report.md --dry-run
```

The last command previews a request. Remove `--dry-run` only when you intend to
create the issue. Optionally run `fjx auth setup-git --host https://forgejo.example`
to share the saved login with Git and `jj git push`.

For automation, supply `FJX_TOKEN` or `FORGEJO_TOKEN` through your environment's
secret mechanism. `auth login --with-token` reads one trimmed line from stdin,
but avoid typing tokens into `echo` commands, shell history, or command arguments.
Windows uses environment tokens rather than saved logins.

## Documentation

- [Command examples](#command-examples), [configuration and safety](#configuration-and-safety), and [output contracts](#output-contracts)
- [Architecture](docs/architecture.md) and [release maintenance](docs/releasing.md)
- [MIT license](LICENSE)
- [Repository maintenance and settings](docs/repository-maintenance.md)

## Command examples

The first three commands below show the Unix saved-login workflow. On Windows, set `FJX_TOKEN` or `FORGEJO_TOKEN` instead.

```text
fjx auth login --host https://forgejo.example
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

## Configuration and safety

Common flags may go before or after command words: `--host URL`, `-R OWNER/REPO`, `--json`, `--dry-run`, and `--yes`. `FJX_HOST`, `FJX_REPO`, `FJX_TOKEN`, and `FORGEJO_TOKEN` provide non-interactive context. On Unix, `FJX_CONFIG` selects the token store.

On Unix, `auth login` takes a token from a hidden prompt. Pass `--with-token` to read one trimmed line from stdin. `auth setup-git` installs a host-scoped Git credential helper so Git and `jj git push` can use the same saved login. Unix token files use mode 0600.

On Windows, use `FJX_TOKEN` or `FORGEJO_TOKEN`; commands can run without `HOME`, `APPDATA`, or `USERPROFILE` when `--host` or `FJX_HOST` supplies the host. Persisted config and `auth login`, `auth logout`, and `auth setup-git` are unavailable because fjx cannot verify private file access with its current dependency set.

Tokens never appear in command arguments or user-facing output. The client accepts HTTPS hosts and loopback HTTP hosts and rejects redirects.

Raw `api` paths sit below the selected host's `/api/v1/` path. They must stay relative. Raw DELETE requires `--yes`. Write requests accept `--dry-run`, which prints the planned request without sending it.

Typed commands cover issue list, view, create, comment, close, and reopen; pull-request list, view, create, diff, checks, comment, review, merge, close, and reopen; action-run list, view, and watch; release list, view, create, and upload; label list and create; milestone list and create; branch list and delete; and workflow dispatch. Lists default to 30 open items where state applies. `--all` follows Forgejo pages up to 1,000 items and fails instead of returning a cut-off list. Forgejo 15.0.7 has no draft field in its create-pull request body, so `pr create --draft` uses its default `WIP:` title prefix in the one create request.

`release view` identifies a release by tag. `release upload` instead requires the positive numeric Forgejo release ID returned by release list, view, or create; a tag is not accepted in that position. Uploads stream a regular file as `application/octet-stream`, default the asset name to the file name, and let `--name` override it. Workflow dispatch requires a workflow file and ref, accepts repeated unique `--field KEY=VALUE` inputs, and returns the created run ID, run number, and jobs.

Every typed write accepts `--dry-run`, which validates local inputs and prints the planned request without sending it. Pull-request merge, branch delete, and raw DELETE require `--yes`, including for a dry run; other typed writes reject `--yes`. Read commands reject both write-safety flags.

`pr checks` exits 1 unless all normalized checks succeed. `run watch` buffers output until the run ends: it emits one final record, exits 0 only for success, and exits 1 for every other conclusion. A timeout or poll error leaves stdout empty.

## Output contracts

Plain output is stable and short. Each tab separates fields and each line feed ends a record. Within server-provided fields, backslash, tab, carriage return, and line feed encode as `\\`, `\t`, `\r`, and `\n`; every other Unicode control scalar encodes as `\u{HEX}` with uppercase hex and no leading zeroes. This encoding is reversible, so text that looks like an escape starts with `\\`. `--json` emits one compact JSON value with the original field values and no plain-output encoding. Errors go to stderr and leave stdout empty. See `fjx --help` for exit classes.

## Development and releases

Use Rust 1.97.1 from `rust-toolchain.toml` and Nushell 0.112.2. Linux tests
also require Git, Bash, `script` (util-linux), GNU coreutils, `tar`, `gzip`,
`zip`, and `unzip`. Run the same checks as CI from the repository root:

```sh
nu --no-config-file scripts/check.nu
```

This runs Cargo format, Clippy, tests, and docs, plus the repository and release
Nushell tests and version validation. The standalone GitHub CI runs on Ubuntu
24.04; it does not provide native Windows or macOS testing or publish binary
releases. The packaging scripts support six target formats, but that is not a
claim of published assets or platform runtime coverage.

See [release maintenance](docs/releasing.md) for version preparation, packaging,
and the distinction between local tooling and automated publication.
