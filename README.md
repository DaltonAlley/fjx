# fjx

`fjx` is a small synchronous, single-binary Forgejo client for people and scripts.
It provides stable output, private token storage, and HTTP requests that refuse
redirects. Version 0.4.0 targets Forgejo 15.0.7.

- View repositories; manage issues, pull requests, and reviews.
- Inspect action runs and dispatch workflows.
- Publish releases and assets; manage labels, milestones, and branches.
- Use readable views, stable tab-separated output, or compact JSON with selected fields.
- Preview writes with `--dry-run`, or use `api` for other Forgejo endpoints.

## Preview

After login, preview an issue without creating it. This is actual CLI output
using an example host, repository, title, and body:

```console
$ fjx issue create --host https://forgejo.example -R owner/repo --title 'Example issue' --body 'Example description.' --dry-run
POST	https://forgejo.example/api/v1/repos/owner/repo/issues	{"body":"Example description.","title":"Example issue"}
```

## Install (recommended)

Download the archive for your system and its matching `.sha256` file from
[Forgejo Releases](https://dyad.tail77425.ts.net/forgejo/dalton/fjx/releases).

| System | Archive target |
| --- | --- |
| Linux x86-64 or ARM64, glibc | `x86_64-unknown-linux-gnu` or `aarch64-unknown-linux-gnu` |
| Linux x86-64 or ARM64, musl | `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| Windows x86-64 | `x86_64-pc-windows-gnu` |

Verify the checksum before extracting. For example, on Linux x86-64 with glibc,
run these commands in the download directory:

```sh
sha256sum --check fjx-0.4.0-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf fjx-0.4.0-x86_64-unknown-linux-gnu.tar.gz
./fjx --version
```

Move `fjx` to a directory on `PATH`. On Windows, extract the `.zip` archive and
add the directory containing `fjx.exe` to `PATH`. On macOS, use `shasum -a 256 -c`
to check the checksum; on Windows, compare `Get-FileHash -Algorithm SHA256` with
the value in the `.sha256` file.

Code and published releases are mirrored from Forgejo to GitHub. Downloads are also available from
[GitHub Releases](https://github.com/DaltonAlley/fjx/releases).

## Install from source

Install Git and Rust 1.97.1, pinned in [rust-toolchain.toml](rust-toolchain.toml).

```sh
git clone https://dyad.tail77425.ts.net/forgejo/dalton/fjx.git
cd fjx
cargo install --path . --locked
fjx --version
```

Make sure Cargo's binary directory, normally `$HOME/.cargo/bin`, is on `PATH`.
The crate is not published to crates.io. To build without installing, run
`cargo build --locked --release` and use `target/release/fjx` (`fjx.exe` on Windows).

## Quickstart

Sign in through your browser. By default, fjx uses the public
**git-credential-oauth** application pre-registered by Forgejo and Gitea, so no
application registration or client ID is needed on a standard installation:

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

Login opens the default browser and also prints an authorization link. Approve
access in the browser; fjx saves your login and refreshes expiring tokens
automatically. The browser's consent page calls the shared application
**git-credential-oauth**; its authorization is also shared with Git credential
helpers using that client ID. Subsequent interactive logins reuse the saved
client ID. OAuth tokens currently have access to all actions your
account can perform; Forgejo does not yet implement OAuth permission scopes.

Client IDs resolve in this order: `--client-id`, `FJX_CLIENT_ID`, the host's saved
OAuth client ID, then `a4792ccc-144e-407e-86c9-5e7d8d9c3269`
(`git-credential-oauth`). Existing saved custom registrations continue to work.
If the server disables the built-in application, register a public OAuth2
application under **Settings → Applications** with redirect URI
`http://127.0.0.1/`, then pass its ID with `--client-id ID`. A custom application
also keeps its consent and authorization separate from the shared Git helper.

Use `--no-browser` to print the link without opening it, or `--web` to request
browser login when stdin is not a terminal. The browser must reach the CLI's
local callback: over SSH, forward the callback port printed by fjx, or use a
personal access token instead. This is a browser callback flow, not device-code
authorization. Login times out after five minutes.

For a personal access token, create one on the forge with permissions for your
commands and run `fjx auth login --with-token --host https://forgejo.example`.
Interactive token input is hidden. For automation, supply `FJX_TOKEN` or `FORGEJO_TOKEN` through your environment's
secret mechanism. `auth login --with-token` reads one trimmed line from stdin,
but avoid typing tokens into `echo` commands, shell history, or command arguments.

## Documentation

- [Command examples](#command-examples), [configuration and safety](#configuration-and-safety), and [output contracts](#output-contracts)
- [Issue and PR workflows](docs/cli-workflows.md), including selective output, triage, and inline reviews
- [Reproducible CLI benchmarks](docs/benchmarks.md)
- [Architecture](docs/architecture.md) and [release maintenance](docs/releasing.md)
- [MIT license](LICENSE)
- [Repository maintenance and settings](docs/repository-maintenance.md)

## Command examples

Saved logins work on Unix and native Windows.

```text
fjx auth login --host https://forgejo.example
fjx auth status --json
fjx auth setup-git --host https://forgejo.example
fjx repo view -R owner/repo
fjx issue list -R owner/repo --all --json --fields number,title,state
fjx issue list -R owner/repo --label bug --assignee @me --search timeout
fjx issue view -R owner/repo 12 --human
fjx issue comments -R owner/repo 12 --all --json --fields id,author,body
fjx issue edit -R owner/repo 12 --add-label bug --add-assignee @me --dry-run
fjx help issue create
fjx schema pr review --json
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

Common flags may go before or after command words: `--host URL`, `-R OWNER/REPO`, `--json`, `--dry-run`, and `--yes`. `FJX_HOST`, `FJX_REPO`, `FJX_TOKEN`, and `FORGEJO_TOKEN` provide non-interactive context. `FJX_CONFIG` selects the token store.

`auth login` uses browser OAuth in a terminal, defaulting to the pre-registered
`git-credential-oauth` application for new logins. Pass `--client-id ID` or set
`FJX_CLIENT_ID` to override it; the ID is remembered with a successful OAuth login.
`--no-browser` prints the authorization link without launching a browser, and
`--web` explicitly selects OAuth without a terminal. `FJX_BROWSER` can name one
browser executable; it is invoked directly with the URL, without a shell.
`--with-token` uses a hidden prompt in a terminal or reads one trimmed line from
stdin. Piped input without browser flags retains the existing token behavior.
`auth setup-git` installs a host-scoped Git credential helper so Git and
`jj git push` can use the same saved login, including OAuth refresh. Unix token
files use mode 0600. `auth logout` removes local credentials; it does not revoke
the authorization on the forge.

On Windows, saved tokens default to `%APPDATA%\fjx\hosts.json`, with a `%USERPROFILE%\AppData\Roaming` fallback. Files are created with a protected DACL granting access only to the current user. Before reading, fjx checks the open file's owner and DACL; only the current user, SYSTEM, and built-in Administrators may have access. Filesystems that cannot enforce or report these permissions fail closed. The file remains plaintext on disk, protected by Windows access controls. This works in SSH sessions without Credential Manager. Environment tokens still work without `HOME`, `APPDATA`, or `USERPROFILE` when `--host` or `FJX_HOST` supplies the host.

Tokens never appear in command arguments or user-facing output. The client accepts HTTPS hosts and loopback HTTP hosts and rejects redirects.

Raw `api` paths sit below the selected host's `/api/v1/` path. They must stay relative. Raw DELETE requires `--yes`. Write requests accept `--dry-run`, which prints the planned request without sending it.

Typed commands cover issue list, view, create, edit, comments, comment, close, and reopen; pull-request list, view, create, edit, files, comments, reviews, review-comments, request-review, diff, checks, comment, review, merge, close, and reopen; action-run list, view, and watch; release list, view, create, and upload; label list and create; milestone list and create; branch list and delete; and workflow dispatch. Lists default to 30 open items where state applies. `--all` follows Forgejo pages up to 1,000 items and fails instead of returning a cut-off list. Forgejo 15.0.7 has no draft field in its create-pull request body, so `pr create --draft` uses its default `WIP:` title prefix in the one create request. `pr create` without `--base` reads the repository's default branch, including during dry-run, and sends it as `base`. API errors include Forgejo's response `message`, truncated and escaped like plain output.

`release view` identifies a release by tag. `release upload` instead requires the positive numeric Forgejo release ID returned by release list, view, or create; a tag is not accepted in that position. Uploads stream a regular file as `application/octet-stream`, default the asset name to the file name, and let `--name` override it. Workflow dispatch requires a workflow file and ref, accepts repeated unique `--field KEY=VALUE` inputs, and returns the created run ID, run number, and jobs.

Every typed write accepts `--dry-run`, which validates local inputs and prints the planned request without sending it. Pull-request merge, branch delete, and raw DELETE require `--yes`, including for a dry run; other typed writes reject `--yes`. Read commands reject both write-safety flags.

`pr checks` exits 1 unless all normalized checks succeed, and rejects contradictory success statuses or a response SHA different from the requested PR head. Forgejo's empty combined-status response (blank SHA/state, zero total, no statuses) reports the requested head with state `unknown`, an empty status list, and exit 1. This means no checks have been reported; it does not establish that checks passed. `pr merge --match-head SHA --yes` sends a server-side expected-head precondition; `--auto` requests server-side merge when checks succeed. `run watch` buffers output until the run ends: it emits one final record, exits 0 only for success, and exits 1 for every other conclusion. A timeout or poll error leaves stdout empty.

Issue/PR metadata names may need paginated read requests, including during dry-run. Edits validate and resolve the entire plan before writes, but multiple writes are not atomic. A later failure reports completed requests on stderr and leaves stdout empty. Assignee additions/removals use read-modify-write and can race concurrent changes. See [workflow details](docs/cli-workflows.md) before using multi-step edits in automation.

## Output contracts

`fjx help COMMAND ACTION` and `fjx COMMAND ACTION --help` show focused help without repository or authentication context. `fjx schema COMMAND ACTION --json` exposes versioned command metadata for tooling. Use `--json --fields number,title,state` to select documented top-level fields without changing bare `--json`. Unknown or duplicate fields fail before any request or write. Projection preserves object/array shapes, null values, and exit codes. It reduces stdout, not the server response size. Issue/PR/run list filters execute on the server and are preserved across pages.

`issue view --human` and `pr view --human` show readable multiline bodies and metadata. Existing plain and JSON defaults remain unchanged except for additive JSON metadata fields. Issue and PR JSON include normalized milestone metadata; PR JSON also distinguishes merged from closed and includes label/assignee names. Comment/review write results expose IDs where supplied by the server. Raw `api --json` emits `null` for a successful empty response.

Plain output is stable and short. Each tab separates fields and each line feed ends a record. Within server-provided fields, backslash, tab, carriage return, and line feed encode as `\\`, `\t`, `\r`, and `\n`; every other Unicode control scalar encodes as `\u{HEX}` with uppercase hex and no leading zeroes. This encoding is reversible, so text that looks like an escape starts with `\\`. `--json` emits one compact JSON value with the original field values and no plain-output encoding. Errors go to stderr and leave stdout empty. See `fjx --help` for exit classes.

## Development and releases

Use Rust 1.97.1 from `rust-toolchain.toml` and Nushell 0.112.2. Linux tests
also require Git, Bash, and `script` (util-linux). Run the same build and E2E
gate as CI from the repository root:

```sh
nu --no-config-file scripts/check.nu
```

On Linux with Nix, [shell.nix](shell.nix) supplies the pinned Rust toolchain and
check utilities. It requires a `nixpkgs` channel with Nushell 0.112.2:

```sh
nix-shell --run 'nu --no-config-file scripts/check.nu'
nix-shell --run 'cargo build --locked --release'
```

The gate runs `cargo build --locked` and `cargo test --locked --all-targets`.
Tests execute the built CLI against local HTTP servers, temporary repositories,
and auth/config fixtures; they need no live Forgejo account or credentials.
The stateful journeys in `tests/e2e_journeys.rs` cover saved-login issue management,
lost write responses, pagination failure and recovery, and checks followed by a
PR head change before a guarded merge. They assert server state across separate
CLI processes, output and exit codes, and absence of duplicate writes. Each CLI
process has a ten-second deadline; fixture socket operations have two-second
deadlines. Run just these journeys with `cargo test --locked --test e2e_journeys`.
They run automatically in the existing PR/main gate and Windows test job.
Forgejo checks the exact PR head in one **Build and E2E** job on a prepared Dyad
runner with a five-minute timeout. Only PRs and pushes to `main` run automatically;
new PR updates cancel superseded runs.

For maintenance and before a release, run `nu --no-config-file scripts/check.nu --full`.
This also runs format, Clippy, docs, repository and release-tool fixtures, and
version validation. It additionally needs GNU coreutils, `tar`, `gzip`, `zip`,
`unzip`, and `file`.

A version increase merged or pushed to `main` triggers a separate six-target
binary release after checks pass. Builds, archive checks, and Linux x86-64
runtime smoke checks precede uploads to a draft. The release becomes public
only after all twelve downloaded assets match the checked local files. PRs,
manual checks, and unchanged versions never publish.

GitHub CI runs the build and E2E gate on Ubuntu 24.04 and Rust tests and Clippy on
native Windows. GitHub release mirroring copies the verified Forgejo notes,
binaries, and checksums; it does not rebuild them. Neither workflow provides native
macOS runtime coverage.

See [release maintenance](docs/releasing.md) for version preparation, packaging,
and the distinction between local tooling and automated publication.
