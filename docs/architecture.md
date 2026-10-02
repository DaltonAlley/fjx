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
| `config` | Token validation, Unix private-file checks, persistence, and credential lookup. |
| `http` | Synchronous authenticated requests, redirect refusal, bounded response reads, and HTTP errors. |
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
Windows uses environment tokens because the current dependency set cannot verify
private persisted-file access. Authentication output must never disclose tokens.

## Safety and output invariants

- Hosts accept HTTPS or loopback HTTP. The HTTP client follows no redirects.
- Writes are not retried. Dry runs validate local inputs without sending writes.
- Pull-request merges, branch deletion, and raw DELETE need explicit confirmation.
- Pagination and run watching accumulate a final result before returning output.
- Plain fields have reversible control-character escaping. JSON retains original
  values. See [README output contracts](../README.md#output-contracts).
- There is no unsafe Rust and no public library API. Runtime dependencies stay
  limited to those listed in [AGENTS.md](../AGENTS.md).

## Verification

Unit tests live with their modules. Integration tests in `tests/*.rs` exercise
the CLI with local servers, temporary repositories, and private config fixtures,
including redaction, host boundaries, pagination failures, and terminal input.
`tests/release.nu` exercises version transactions and packaging/publication
helpers using fixtures and simulated tools. Passing those tests does not mean a
binary has been published or validated on native Windows or macOS hardware.
