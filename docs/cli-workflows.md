# Issue and pull-request workflows

These commands target Forgejo 15's API. Supply `--host` and `-R owner/repo`, or use
FJX's existing environment, saved-host, and Git/jj remote context. Examples assume
that context and a token are already configured. Preview writes with `--dry-run`
before removing that flag to execute them.

## Discover just the command you need

```text
fjx help issue
fjx issue create --help
fjx issue list --state closed --help
fjx schema pr review --json
```

Help and schema never read stdin, resolve credentials, or send requests. Leaf help
shows the command's arguments, flags, output fields, and an example. Schema has a
`schema_version` and command metadata for tools. Prefer a leaf schema over loading
the entire catalog into an agent's context.

## Select records on the server and fields at the output boundary

```text
fjx issue list --label bug --assignee @me --search timeout --all --json --fields number,title,state
fjx issue list --author @me --since 2026-09-01T00:00:00Z --state all
fjx pr list --author @me --milestone v1.0 --json --fields number,title,head_sha,merged
fjx run list --status failure --ref refs/heads/main --json --fields id,name,head_sha
```

| List | Server filters |
|---|---|
| `issue list` | `--label` (repeatable), `--author`, `--assignee`, `--search`, `--milestone`, `--since`, `--before`, `--sort` |
| `pr list` | `--label` (repeatable), `--author`, `--milestone`, `--sort` |
| `run list` | `--status`, `--event`, `--ref`, `--head-sha`, `--workflow` |

Filter capabilities differ by endpoint. PR text search and assignee filtering are
not supported by the typed PR list endpoint. Issue date filters require RFC3339.
Run `--workflow` corresponds to Forgejo's workflow ID query parameter. `--sort`
values are passed to the server. Repeated issue labels retain Forgejo's OR/any
matching behavior, and Forgejo discards unknown issue filter names. Use known
labels/milestones when narrowing a query rather than assuming a misspelled name
must return no results. PR label and milestone names are resolved to IDs first.
`@me` in issue author/assignee or PR author filters needs one current-user read.

Lists default to page 1 and 30 records. `--limit` sets the **page size**, between 1
and 50, not the total count. `--page` and `--all` are mutually exclusive. `--all`
fetches all reported pages up to the 1,000-item safety cap. Each page receives the
same server filters. A late page failure emits no partial stdout.

`--fields` requires `--json` and accepts comma-separated, documented **top-level**
field names. It preserves the object/array shape, selected JSON types, nulls, and
command exit code. Unknown and duplicate fields fail before any command execution,
including before writes. Fields do not support jq expressions, nested paths, or
wildcards. Bare `--json` keeps all normalized fields. Raw `api` has no typed output
schema and does not support field projection of responses.

Projection saves stdout bytes and agent context, not server bandwidth. Server
filters can reduce both request count and response bytes. Optional selected fields
remain present as `null`. Dry-run projection uses the request fields `kind`,
`method`, `url`, and `body`, rather than the eventual result fields.

## Read bodies, discussion, and review context

```text
fjx issue view 42 --human
fjx issue comments 42 --all --json --fields id,author,body
fjx pr view 42 --human
fjx pr files 42 --json --fields filename,status,additions,deletions
fjx pr comments 42 --all --json
fjx pr reviews 42 --all --json
fjx pr review-comments 42 17 --all --json
```

`--human` is an explicit presentation option for issue/PR view and cannot combine
with `--json`. It preserves body line breaks while escaping terminal control
characters. Existing plain view remains one stable tab-separated summary record.
No TTY detection silently changes output.

Issue and PR JSON contain label and assignee names and a compact milestone object
or `null`. PR JSON also includes `merged`, `merged_at`, and `merge_commit_sha`,
separate from the open/closed state.

`pr comments` reads the general conversation. `pr reviews` reads review summaries.
`pr review-comments NUMBER REVIEW_ID` reads inline comments for that review, with
file/line and commit context. All collection commands accept `--page`, `--limit`,
and `--all`; there is no hidden fetch of every discussion or a full diff. Comment
and review IDs can be used with Forgejo's raw API for further operations. Editing
or deleting an existing comment is not a new typed command in this change.

## Create and triage issues or PRs

```text
fjx issue create --title "Timeout during sync" --body-file report.md --label bug --assignee @me --milestone v1.0 --dry-run
fjx pr create --head fix-timeout --title "Fix sync timeout" --label bug --assignee @me --dry-run
fjx issue edit 42 --title "Sync times out after reconnect" --body-file report.md --dry-run
fjx issue edit 42 --add-label bug --remove-label needs-triage --add-assignee @me --milestone v1.0 --dry-run
fjx pr edit 42 --base main --remove-assignee former-reviewer --clear-milestone --dry-run
fjx pr request-review 42 --reviewer alice --team maintainers --dry-run
```

Creation accepts repeated `--label`, `--label-id`, and `--assignee` flags, plus
`--milestone NAME` or `--milestone-id ID`. Editing accepts repeated
`--add-label`, `--remove-label`, `--add-label-id`, `--remove-label-id`,
`--add-assignee`, and `--remove-assignee`, plus title/body and milestone changes.
Only PR edits accept `--base`. Empty bodies can clear the description. Use
`--clear-milestone` to remove a milestone. Set/clear conflicts, opposing changes to
the same resolved label/assignee, and empty edits are rejected.

Label names are matched exactly against paginated repository labels; milestone
names against all repository milestones, including closed ones. Missing or
ambiguous names fail before writes. Use explicit IDs when names are ambiguous or
when applying an organization label not returned by the repository label listing.
An ID is not guessed from a numeric-looking name. IDs must be positive.

### Safety of multi-step edits

All local validation and required identifier/assignee reads finish before the
first write. `--dry-run` may therefore perform **read requests**, but never writes.
An edit dry-run returns an array of planned request records in JSON, even if it
contains one request. A create dry-run retains the existing single-request object.

Title/body/milestone/assignee changes share a PATCH when possible. Labels use
add/remove endpoints so unrelated labels are not replaced. A combined edit is not
a server transaction: if a later write fails, stderr identifies completed steps,
stdout stays empty, and FJX does not retry or pretend it rolled changes back.
A timeout can also leave the failed step's server-side result uncertain. Inspect
the current issue/PR before retrying.

Assignee changes require reading current assignments and sending a replacement
list. They can race a concurrent editor. No client-side check makes that sequence
atomic. Existing destructive `--yes` rules are unchanged; routine issue/PR edits
reject `--yes` and use `--dry-run` for preview.

## Submit a coherent inline review

Put a JSON array in `comments.json`:

```json
[
  {
    "path": "src/client.rs",
    "new_position": 42,
    "body": "Please handle the empty response before decoding JSON."
  },
  {
    "path": "src/client.rs",
    "old_position": 18,
    "body": "This removed branch covered the retry guard."
  }
]
```

Then use the full commit SHA you actually reviewed:

```text
fjx pr review 42 --event request-changes --body "Two points to address." --comments-file comments.json --commit 0123456789abcdef0123456789abcdef01234567 --dry-run
```

Each comment needs a nonempty body, a repository-relative path, and exactly one
positive old/new line position. Unknown JSON keys are rejected. The array is
bounded to 1,000 comments and the input to 16 MiB. `--comments-file -` reads stdin;
it cannot share stdin with `--body-file -`. Inline comments require a full 40- or
64-digit hexadecimal commit SHA. They are sent in one review request, not as
separate generic comments. A body is optional when valid inline comments provide
the review content. Output includes the new review ID when Forgejo supplies it.

For a merge after review, pin the expected head:

```text
fjx pr checks 42 --json --fields sha,state
fjx pr merge 42 --match-head 0123456789abcdef0123456789abcdef01234567 --yes --dry-run
fjx pr merge 42 --auto --match-head 0123456789abcdef0123456789abcdef01234567 --yes --dry-run
```

`--match-head` sends Forgejo's `head_commit_id` precondition; the server decides
whether it matches. `--auto` sends `merge_when_checks_succeed`; acceptance schedules
an automatic merge and is not a claim the PR is already merged. A successful check
response for a different PR-head SHA or with contradictory child failures is an
incompatible-data error rather than success. FJX never automatically retries a
review, edit, or merge.

## Verification and measurement

The integration suite runs the real executable against local, schema-compatible
HTTP fixtures, including failure and pagination paths. This does not claim native
Windows/macOS or live-server coverage. See [benchmarks](benchmarks.md) for the
repeatable before/after measurements, fixture sizes, and limitations.
