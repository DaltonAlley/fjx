# Support

Start with [README.md](README.md) and `fjx --help`. For a bug or usage question,
open an issue in this repository. For a suspected vulnerability, follow
[SECURITY.md](SECURITY.md) instead. Support is best effort, with no guaranteed
response time.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| `fjx` is not found | Add Cargo's binary directory to `PATH`, or run the binary in `target/release`. |
| Host or repository context is missing | Pass `--host https://your-forgejo.example` and `-R owner/repo` explicitly. Remote inference requires a usable Git or jj repository. |
| No token is available | On Unix, use the hidden `auth login` prompt. In automation or Windows, supply an environment token securely. `FJX_TOKEN` takes precedence over `FORGEJO_TOKEN` and saved config. |
| Token file is rejected | On Unix, use a regular file with mode 0600. Do not bypass checks with symlinks or broad permissions. |
| HTTP or TLS failure | Check the host URL, server certificate, connectivity, and token permissions. HTTPS is required except for loopback HTTP. Redirects are intentionally rejected. |
| A delete or merge is refused | These commands require `--yes`, even with `--dry-run`. Review the target before confirming. |
| A list fails with `--all` | Pagination is capped at 1,000 items and fails rather than silently truncating. Use explicit pages for larger collections. |
| `pr checks` or `run watch` exits 1 | This can be the result of unsuccessful Forgejo work, not a client crash. Inspect the final record. A watch timeout or polling error leaves stdout empty. |
| Release upload rejects a tag | Use the positive numeric release ID, not the release tag. |

Do not share a token store or dump your whole environment. Tokens are stored as
secrets in the config file, not encrypted by fjx.

## Useful report details

Include:

- `fjx --version`, operating system, and Forgejo version.
- The command with secrets and private repository names removed.
- Expected behavior, actual behavior, exit code, and sanitized stderr.
- Whether explicit `--host` and `-R` change the result.
- A small reproduction or fixture when possible.

For build or release-script problems, include Rust and Nushell versions and the
failing check from [CONTRIBUTING.md](CONTRIBUTING.md). Redact private URLs,
usernames, tokens, and sensitive response data before posting.
