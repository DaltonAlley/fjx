# CLI workflows and reference

This page is the detailed guide and reference for fjx commands. Supply `--host` and `-R owner/repo`, or use
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
Only PR edits accept `--base`. When `pr create` omits `--base`, fjx reads the
repository's default branch and sends it as `base`; a dry run makes the same read
and shows the resolved branch. Empty bodies can clear the description. Use
`--clear-milestone` to remove a milestone. Set/clear conflicts, opposing changes to
the same resolved label/assignee, and empty edits are rejected.

Label names are matched exactly against paginated repository labels; milestone
names against all repository milestones, including closed ones. Missing or
ambiguous names fail before writes. Use explicit IDs when names are ambiguous or
when applying an organization label not returned by the repository label listing.
An ID is not guessed from a numeric-looking name. IDs must be positive.

### Track issues on a project board

```text
fjx board view
fjx board view --json --fields title,cards
fjx board move 42 "In Progress"
fjx board move 42 done --board Development --dry-run
fjx board remove 42
fjx issue create --title "Timeout during sync" --board Development --dry-run
```

Boards are Gitea repository projects, checked against Gitea 28.2.0. Forgejo 17
has no project API, and board commands exit 7 there before writing. The forge's
card routes take an issue's forge-wide ID and a column ID. fjx resolves both from the issue number and the
column title, so `board move` reads the board, its columns and the issue, then
sends one write. That write adds the card when the issue is not on the board and
moves it otherwise. `board remove` reads each column to find the card. A dry run
makes the same reads and prints the write it would send. Neither command
retries its write or accepts `--yes`.

The repository's only open board is implied. Otherwise select one with
`--board TITLE` or `--board-id ID`. Titles match exactly, then ignoring case; an
unknown or ambiguous board or column fails before the write and lists the
available titles. `issue create --board` resolves the board before creating the
issue and puts the new card in the board's default column.

`board view` reports the columns in board order and the cards in each. JSON is
an array of columns, `{kind, board, id, title, default, cards}`, whose cards are
`{number, state, title, pull_request, html_url}`. Plain output has one
`column`, ID, title, card count line per column, followed by one `card`, column
title, number, state, title line per card. Creating, renaming and reordering
columns, and organization and user boards, remain available through `fjx api`.

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

When no commit statuses exist, Forgejo can return an empty SHA and state with
`total_count: 0` and null or empty statuses. `pr checks` recognizes this empty
response and reports the requested head's SHA, `state: "unknown"`, and
`statuses: []`, with exit code 1. No checks have been reported, so this result
does not establish that checks passed. Nonempty mismatched SHAs and blank SHAs
with a state or statuses still fail as incompatible data.

## Sign in and choose a host

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

`FJX_BROWSER` can name one browser executable; it is invoked directly with the
URL, without a shell. Piped input without browser flags uses token login.
`auth setup-git` installs a host-scoped Git credential helper so Git and
`jj git push` can use the same saved login, including OAuth refresh. Unix token
files use mode 0600. `auth logout` removes local credentials; it does not revoke
the authorization on the forge.

Common flags may go before or after command words: `--host URL`, `-R OWNER/REPO`, `--json`, `--dry-run`, and `--yes`. `FJX_HOST`, `FJX_REPO`, `FJX_TOKEN`, and `FORGEJO_TOKEN` provide non-interactive context. `FJX_CONFIG` selects the token store.

Without `--host` or `FJX_HOST`, the host comes from the repository's remote. An
SSH remote selects the saved login whose DNS name is the remote's SSH host. When
that host is an alias from your SSH configuration, such as
`ssh://git@forge-ssh:2222/owner/repo.git`, record it once on the login it stands
for: `fjx auth add-ssh-alias forge-ssh --host https://forgejo.example`. An alias
needs an existing saved login, names one host, applies only to SSH remotes, and
cannot be another saved host's DNS name, so a remote cannot send a token to a
host you did not log in to. `auth status --json` lists a host's aliases,
`auth remove-ssh-alias` forgets one, and `auth logout` removes them with the
login. Versions before the alias commands ignore the field and drop it when they
rewrite the token file.

`auth list` shows every saved login with its host, user, default flag and SSH
aliases. It reads only the token file: it makes no request, checks no token and
never prints one. Use `auth status` to check that a token still works.

On Windows, saved tokens default to `%APPDATA%\fjx\hosts.json`, with a `%USERPROFILE%\AppData\Roaming` fallback. Files are created with a protected DACL granting access only to the current user. Before reading, fjx checks the open file's owner and DACL; only the current user, SYSTEM, and built-in Administrators may have access. Filesystems that cannot enforce or report these permissions fail closed. The file remains plaintext on disk, protected by Windows access controls. This works in SSH sessions without Credential Manager. Environment tokens still work without `HOME`, `APPDATA`, or `USERPROFILE` when `--host` or `FJX_HOST` supplies the host.

Tokens never appear in command arguments or user-facing output. The client accepts HTTPS hosts and loopback HTTP hosts and rejects redirects.

## Command examples

Saved logins work on Unix and native Windows.

```text
fjx auth login --host https://forgejo.example
fjx auth status --json
fjx auth list --json
fjx auth setup-git --host https://forgejo.example
fjx auth add-ssh-alias forge-ssh --host https://forgejo.example
fjx repo view -R owner/repo
fjx issue list -R owner/repo --all --json --fields number,title,state
fjx issue list -R owner/repo --label bug --assignee @me --search timeout
fjx issue view -R owner/repo 12 --human
fjx issue comments -R owner/repo 12 --all --json --fields id,author,body
fjx issue edit -R owner/repo 12 --add-label bug --add-assignee @me --dry-run
fjx help issue create
fjx schema pr review --json
fjx issue create -R owner/repo --title "Bug" --body-file report.md
fjx board view -R owner/repo
fjx board move -R owner/repo 12 "In Progress"
fjx board remove -R owner/repo 12 --dry-run
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
fjx webhook list -R owner/repo --all --json
fjx webhook create -R owner/repo --input hook.json --dry-run
fjx webhook view -R owner/repo 1 --json
fjx webhook edit -R owner/repo 1 --input hook-update.json --dry-run
fjx webhook test -R owner/repo 1 --ref main
fjx webhook delete -R owner/repo 1 --yes --dry-run
fjx api /version --json
fjx api /repos/owner/repo/issues --paginate --json
```

## Organizations and teams

Organization and team commands need a host and login, with no repository context.
`org list` lists your memberships; `org list USER` lists that user's organizations
visible to your account. Team commands use the positive ID from `team list` or
`team create`. All lists accept `--page`, `--limit`, and `--all`.

```sh
fjx org list --all --json
fjx org view my-org --json
fjx org members my-org --all
fjx org repos my-org --all
fjx team list my-org --all --json
fjx team view 7 --json
fjx team members 7 --all
fjx team repos 7 --all
```

Create and edit accept a JSON object from `--input PATH` or `--input -` for stdin.
For example, save this as `org.json`:

```json
{"username":"my-org","full_name":"My organization","visibility":"private"}
```

Run `fjx org create --input org.json --dry-run` to preview, then remove `--dry-run`
to create it. For an update, save `{"description":"Updated description"}` to
`org-edit.json` and run `fjx org edit my-org --input org-edit.json`.
Organization input supports `username` (create only), `full_name`, `description`,
`email`, `website`, `location`, `visibility` (`public`, `limited`, `private`), and
`repo_admin_change_team_access` (boolean). Empty strings clear profile fields;
unknown fields and null are rejected. The server defaults new organizations to
public visibility when omitted. Organization edits read and preserve profile
fields Forgejo would otherwise clear, including during dry runs.

For a team, save this as `team.json`:

```json
{"name":"Developers","units_map":{"repo.code":"read","repo.issues":"write"},"includes_all_repositories":false,"can_create_org_repo":false}
```

```sh
fjx team create my-org --input team.json --dry-run
fjx team edit 7 --input team-edit.json --dry-run
fjx team add-member 7 alice
fjx team add-repo 7 my-org/project
fjx team remove-repo 7 my-org/project --yes
fjx team remove-member 7 alice --yes
fjx org remove-member my-org alice --yes
fjx team delete 7 --yes
fjx org delete my-org --yes
```

For `team-edit.json`, use a partial object such as `{"description":"Maintainers"}`.
Team input supports `name`, `description`, `permission` (`read`, `write`, `admin`),
`units_map`, `includes_all_repositories`, and `can_create_org_repo`. Create defaults
permission to `read` and requires a name. Non-admin creation and explicit
non-admin permission changes require a nonempty `units_map`. Map keys are
`repo.code`, `repo.issues`, `repo.ext_issues`, `repo.wiki`, `repo.ext_wiki`,
`repo.pulls`, `repo.releases`, `repo.projects`, `repo.packages`, and `repo.actions`;
values are `none`, `read`, or `write` (external issues/wiki allow only `none` or
`read`). A supplied map replaces all unit permissions, and Forgejo derives the
team's base permission from it. `admin` grants every unit and cannot combine
with a map. Unknown fields and null are rejected. Edits read the team and preserve
omitted values; changes to Owners team permissions are refused. Preservation
reads can race concurrent edits.

Adding a team member also joins them to the organization. Removing a team member
removes only that team's membership; `org remove-member` removes organization
membership and associated team access. A team repository must belong to its
organization. Forgejo enforces account permissions, last-owner protection, and
organization deletion rules. Every delete or removal requires `--yes`, including
a dry run; other writes reject `--yes`. Every write supports `--dry-run`.

Commands provide stable plain records and JSON with `--fields` projection.
Plain columns are: organization `id/name/visibility/full_name`; team
`id/name/permission/includes_all_repositories/can_create_org_repo/description`;
member `id/login/full_name`; repository `id/full_name/private/html_url`;
mutation `action/target/ok`. Columns are tab-separated and control characters
are escaped as described below. `fjx schema org --json` and
`fjx schema team --json` list all JSON fields. `fjx help org create` and
`fjx help team create` describe the input contract. Organization renaming,
public membership visibility, org repository creation, hooks, secrets, and
runner administration remain available through `fjx api`.

## Repository webhooks

Repository webhooks let Forgejo notify automation services when code, issues,
PRs or CI change. `webhook create` and `edit` accept JSON from `--input PATH|-`.
Create requires `config.url` and a nonempty `events` array; it defaults to an
active Forgejo hook using JSON POST. For example, a non-secret configuration is
`{"config":{"url":"https://receiver.example/events"},"events":["push","issues","pull_request"]}`.
Optional fields are `active`, `branch_filter`, `authorization_header` and
`config.secret`. Edit changes supplied top-level fields; supplying `config`
requires its URL. `{"active":false}` pauses an existing hook.

Forgejo 17 resets omitted event, branch-filter and authorization fields on
edit. fjx reads and preserves those fields, including during dry runs. This
read-modify-write can race another editor. Signing-secret rotation and changes
to workflow, action-run or package subscriptions require a replacement hook
because Forgejo 17 ignores those edits; fjx rejects them before writing.
Hooks reporting every event are also refused: the API hides whether they use
**Send everything**, and editing that mode can lose CI and package subscriptions.
An explicit selection of every event has the same response and is also refused.
Create a replacement hook for either case.

Keep real receiver URLs and credentials in protected files or feed them through
stdin from a secret store. Webhook output omits URLs, credentials and provider
metadata; dry runs redact URLs, signing secrets and authorization headers.
The typed commands support Forgejo JSON POST hooks; other providers use `api`.
`webhook test` queues a push test; it does not prove delivery. Forgejo 17 has no
delivery-history API: check **Settings → Webhooks → Recent Deliveries** and
verify that the receiver handled the event. Available event names depend on the
server version. The receiver must authenticate requests, handle duplicates and
apply its own authorization rules. These commands configure subscriptions;
they do not run a receiver or grant bots permission to merge.

## Raw API and write safety

Raw `api` paths sit below the selected host's `/api/v1/` path. They must stay relative. Raw DELETE requires `--yes`. Write requests accept `--dry-run`, which prints the planned request without sending it.

Typed commands cover the organization and team workflows above; issue list, view, create, edit, comments, comment, close, and reopen; board list, view, move, and remove; pull-request list, view, create, edit, files, comments, reviews, review-comments, request-review, diff, checks, comment, review, merge, close, and reopen; action-run list, view, and watch; release list, view, create, and upload; label list and create; milestone list and create; branch list and delete; and workflow dispatch. Lists default to 30 open items where state applies. `--all` follows Forgejo pages up to 1,000 items and fails instead of returning a cut-off list. Forgejo 15.0.7 has no draft field in its create-pull request body, so `pr create --draft` uses its default `WIP:` title prefix in the one create request. `pr create` without `--base` reads the repository's default branch, including during dry-run, and sends it as `base`. API errors include Forgejo's response `message`, truncated and escaped like plain output.

`release view` identifies a release by tag. `release upload` instead requires the positive numeric Forgejo release ID returned by release list, view, or create; a tag is not accepted in that position. Uploads stream a regular file as `application/octet-stream`, default the asset name to the file name, and let `--name` override it. Workflow dispatch requires a workflow file and ref, accepts repeated unique `--field KEY=VALUE` inputs, and returns the created run ID, run number, and jobs.

Every typed write accepts `--dry-run`, which validates local inputs and prints the planned request without sending it. Pull-request merge, branch/webhook delete, organization/team deletes and access removals, and raw DELETE require `--yes`, including for a dry run; other typed writes reject `--yes`. Read commands reject both write-safety flags.

`pr checks` exits 1 unless the combined state is `success`, and rejects a combined state that its child statuses contradict or a response SHA different from the requested PR head. As in Forgejo, `success` allows `skipped` jobs but needs at least one successful check. When every check is skipped, the state is `skipped` and the exit code is 1. Forgejo's empty combined-status response (blank SHA/state, zero total, no statuses) reports the requested head with state `unknown`, an empty status list, and exit 1. This means no checks have been reported; it does not establish that checks passed. `pr merge --match-head SHA --yes` sends a server-side expected-head precondition; `--auto` requests server-side merge when checks succeed. `run watch` buffers output until the run ends: it emits one final record, exits 0 only for success, and exits 1 for every other conclusion. A timeout or poll error leaves stdout empty.

Issue/PR metadata names may need paginated read requests, including during dry-run. Edits validate and resolve the entire plan before writes, but multiple writes are not atomic. A later failure reports completed requests on stderr and leaves stdout empty. Assignee additions/removals use read-modify-write and can race concurrent changes. See [safety of multi-step edits](#safety-of-multi-step-edits) before using multi-step edits in automation.

## Output contracts

`fjx help COMMAND ACTION` and `fjx COMMAND ACTION --help` show focused help without repository or authentication context. `fjx schema COMMAND ACTION --json` exposes versioned command metadata for tooling. Use `--json --fields number,title,state` to select documented top-level fields without changing bare `--json`. Unknown or duplicate fields fail before any request or write. Projection preserves object/array shapes, null values, and exit codes. It reduces stdout, not the server response size. Issue/PR/run list filters execute on the server and are preserved across pages.

`issue view --human` and `pr view --human` show readable multiline bodies and metadata. Existing plain and JSON defaults remain unchanged except for additive JSON metadata fields. Issue and PR JSON include normalized milestone metadata; PR JSON also distinguishes merged from closed and includes label/assignee names. Comment/review write results expose IDs where supplied by the server. Raw `api --json` emits `null` for a successful empty response. After a successful `issue create` or `pr create`, plain mode also writes the new URL alone on one stderr line, as `gh` does; stdout keeps the record, and `--json` keeps stderr empty.

Plain output is stable and short. Each tab separates fields and each line feed ends a record. Within server-provided fields, backslash, tab, carriage return, and line feed encode as `\\`, `\t`, `\r`, and `\n`; every other Unicode control scalar encodes as `\u{HEX}` with uppercase hex and no leading zeroes. This encoding is reversible, so text that looks like an escape starts with `\\`. `--json` emits one compact JSON value with the original field values and no plain-output encoding. Errors go to stderr and leave stdout empty. See `fjx --help` for exit classes.

## MCP for local agents

Use the same installed binary and Forgejo login with an MCP client:

```sh
fjx auth login --host https://forgejo.example
fjx mcp config --host https://forgejo.example --json
fjx mcp serve --host https://forgejo.example
```

`mcp config` prints a `mcpServers.fjx` entry with the absolute executable path,
`args: ["mcp", "serve", "--host", "https://forgejo.example"]`, and an empty `env`.
Copy that entry into your client, or enter its command and arguments in MausBot's
MCP server settings. It contains no credentials and does not change client
settings. Saved logins resolve under the account running the command; set
`FJX_CONFIG` to a private token file if the client uses another config location.
For environment authentication, use the client's secret settings for `FJX_TOKEN`
or `FORGEJO_TOKEN`. Never put tokens in arguments. A remote client must run this
binary on a machine that can reach Forgejo and access that login.

The stdio adapter follows the [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) and
[tool result format](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
It supports MCP revisions `2025-06-18` and `2025-11-25` with the
initialize/initialized handshake. It selects `2025-11-25` for other requested
versions; a client that cannot use that revision must disconnect. It does not
implement the `2026-07-28` stateless protocol. MausBot 0.1.160's command
probe requests `2025-06-18`. Only newline-delimited JSON-RPC goes to stdout.

Tools are read-only: `repository_get`, `issue_list`, `issue_get`,
`issue_comments`, `pull_request_list`, `pull_request_get`,
`pull_request_comments`, `pull_request_reviews`, `pull_request_files`,
`pull_request_checks`, `action_run_list`, `action_run_get`, and
`repository_activity`. Every call requires explicit `repo: "OWNER/REPO"`; view
and PR collection tools also require a positive `number` (the run ID for
`action_run_get`). Lists accept `page` (default 1, at most 1,000,000) and `limit`
(default 30, at most 100); issue and PR lists accept `state` (`open`, `closed`,
`all`). Unknown fields, host overrides, CLI flags, writes, file reads, and shell
execution are unavailable through these tools.

Successful results provide `{data, exit_code}` in `structuredContent` and the
same JSON as text. Check `exit_code`: a pending, missing or unsuccessful PR
check result retains 1 and never establishes merge readiness. Tool failures use
`isError: true` with `{error: {category, exit_code, message}}`; malformed protocol
requests use JSON-RPC errors. PR JSON includes `author_id` (null if absent from
the server), preserving the numeric creator identity needed for maintenance
scope. Repository text is untrusted data, never instructions or merge approval.

One tool call runs at a time; overlapping calls receive a busy error. Input
frames are limited to 64 KiB and completed tool data to 2 MiB. Existing HTTP
response limits and redirect refusal apply. Calls have a 30-second HTTP budget,
with each request (including OAuth refresh) capped at 15 seconds. A busy saved
authentication lock fails promptly so the client can retry later. Cancellation and stdin closure stop
further requests and suppress the result; an in-flight blocking HTTP request
may take up to its timeout to return. EOF closes the adapter after that worker
finishes. No queue or background polling is started.

`repository_activity` reads one page of Forgejo's activity feed and returns
normalized event IDs, types, timestamps, actor IDs, refs and content. Deduplicate
by event ID; page positions can change during new activity. This is a partial
feed, without complete webhook history or CI event coverage. It provides no
push transport, resource subscriptions, or automatic bot dispatch. The MausBot 0.1.160
integration inspected on November 16, 2026 mounts tools during agent turns; its command probe
only initializes and lists tools. No idle-bot dispatch handler for MCP resource
updates was found in the installed integration. Tool discovery is therefore
insufficient evidence for replacing repository-event delivery.

Keep existing webhook delivery and backup maintenance until an authenticated
end-to-end replacement proves that repository events awaken and route to the
intended bot. The smallest bridge still needed is a persistent, authenticated
repository-event receiver that dispatches MausBot work; MCP supplies the reads
after dispatch. A MausBot notification-to-dispatch feature could replace that
bridge if independently verified. To roll back this adapter, remove or disable
its client entry; it makes no Forgejo hook or automation changes.

## Verification and measurement

The integration suite runs the real executable against local, schema-compatible
HTTP fixtures, including failure and pagination paths. This does not claim native
Windows/macOS or live-server coverage. See [benchmarks](benchmarks.md) for the
repeatable before/after measurements, fixture sizes, and limitations.
