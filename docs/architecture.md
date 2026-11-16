# Architecture

`fjx` is a synchronous Rust executable, not a library or background service.
`src/main.rs` parses arguments, dispatches one command, and writes a completed
`Outcome` to stdout. Errors go to stderr with a categorized exit code. This
boundary prevents partial success output when a request or later page fails.

## Module ownership

| Module | Responsibility |
| --- | --- |
| `args` | CLI syntax, command shapes, local argument and safety-flag validation. |
| `help` | Shared command discovery metadata, scoped help, output field catalogs, and versioned schemas. |
| `host` | Host URL validation and API URL construction, including loopback HTTP rules. |
| `repo` | Owner/repository validation and Git/jj remote inference. |
| `context` | Resolve explicit flags, environment, remotes, saved host, and token source. |
| `config` | Token validation, private-file checks, persistence, and credential lookup. Windows system calls are isolated in `config/windows.rs`; descriptor rules in `config/acl.rs` are tested on every platform. |
| `http` | Synchronous authenticated requests, redirect refusal, bounded response reads, and HTTP errors. |
| `oauth` | Public-client browser authorization, PKCE S256, bounded loopback callbacks, token refresh, and platform browser/RNG integration. |
| `commands` | Typed Forgejo operations, raw API path rules, pagination, and command outcomes. |
| `output` | Stable plain-field encoding, safe multiline human text, compact JSON, and field projection. |
| `error` | Error categories and exit codes. |

`commands/typed.rs` shares typed-command helpers. `commands/triage.rs` owns
metadata resolution and planned multi-step issue/PR edits. It resolves the full
plan before sending writes, preserves unrelated labels, and reports completed
requests if a later step fails. Assignee replacement is a documented read-modify-
write race, not an atomic operation. Raw API path validation belongs to
`commands/api.rs`; do not duplicate it in the CLI parser or transport.

`main` validates requested output field names against the command's discovery
metadata before command execution. Projection runs only after a complete result
exists and preserves its exit status. It does not stream pages or turn an error
into partial success. Schemas and focused help are local-only and do not resolve
credentials or repository context.

## Context and authentication

Explicit `--host` and `-R` override their `FJX_HOST` and `FJX_REPO` environment
counterparts. Missing context can be inferred from repository remotes; the saved
default host is a final host fallback. Tokens resolve in order: `FJX_TOKEN`,
`FORGEJO_TOKEN`, then the saved token for the selected host. Invalid environment
values fail rather than silently choosing another token.

On Unix, saved credentials are protected by private-file permission checks.
Windows creates files with a protected owner-only DACL and verifies the owner and
DACL on the same handle used to read tokens. SYSTEM and built-in Administrators
are also accepted when reading existing files. This uses file access controls,
including in SSH network logons, without Credential Manager. Authentication
output must never disclose tokens.

Interactive `auth login` uses OAuth authorization code flow with PKCE S256 and
a random state value. Client IDs resolve from `--client-id`, `FJX_CLIENT_ID`,
the host's saved OAuth registration, then the public `git-credential-oauth`
client (`a4792ccc-144e-407e-86c9-5e7d8d9c3269`) pre-registered by both forges.
No instance-specific defaults are embedded. The consent page identifies the
shared application as `git-credential-oauth`; its server-side grants are shared
with other clients using that registration. Administrators may disable it;
custom public registrations with redirect URI `http://127.0.0.1/` remain
supported. No client secret is used. An
ephemeral listener binds only IPv4 loopback, checks the callback Host, path,
state, and duplicate parameters, bounds headers and read time, and closes after
authorization or a five-minute timeout. Progress and the manual authorization
link go to stderr, leaving stdout for the final result. The selected host's
base path is retained for both authorization and token exchange.

OAuth credentials extend the existing host record with client ID, refresh token,
and absolute access-token expiry. Existing PAT records remain compatible. OAuth
requests use Bearer authorization; PAT requests retain token authorization.
Expiring saved OAuth tokens refresh before API execution (and Git credential
delivery), persisting rotated credentials before use. Neither a failed refresh
nor a failed API write is replayed. Environment tokens retain precedence and
bypass saved-token refresh. A stable sidecar file serializes credential updates
and refresh across CLI/Git processes; refresh reloads credentials after locking
so a rotated refresh token is not reused. OAuth endpoint errors omit server bodies to prevent
code/token disclosure. Local logout does not revoke server authorization.

The dependency allowlist remains unchanged. Randomness comes from `/dev/urandom`
on Unix and Windows BCryptGenRandom. The small safe SHA-256/base64url implementation
is checked against FIPS digest vectors and the RFC 7636 S256 example.

## Safety and output invariants

- Hosts accept HTTPS or loopback HTTP. The HTTP client follows no redirects.
- Writes are not retried. Dry runs validate local inputs without sending writes.
- Pull-request merges, branch deletion, and raw DELETE need explicit confirmation.
- Pagination and run watching accumulate a final result before returning output.
- Plain fields have reversible control-character escaping. JSON retains original
  values. See [README output contracts](../README.md#output-contracts).
- Unsafe Rust is denied except in `config/windows.rs` and `oauth/windows.rs`.
  Their documented Windows API calls use owned handles, aligned buffers, or a
  fixed-size writable RNG buffer. There is no public library API.
  Runtime dependencies stay limited to those declared in [Cargo.toml](../Cargo.toml).

## Verification

Unit tests live with their modules. Integration tests in `tests/*.rs` exercise
the CLI with local servers, temporary repositories, and private config fixtures,
including redaction, host boundaries, pagination failures, and terminal input.
`tests/release.nu` exercises version transactions and packaging/publication
helpers using fixtures and simulated tools. Passing those tests does not mean a
binary has been published or validated on native Windows or macOS hardware.
