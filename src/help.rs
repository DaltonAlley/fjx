use crate::error::Error;
use crate::output::Outcome;
use serde::Serialize;
use std::fmt::Write;

const ORGANIZATION: &[&str] = &[
    "kind",
    "id",
    "name",
    "full_name",
    "description",
    "email",
    "website",
    "location",
    "visibility",
    "repo_admin_change_team_access",
];
const TEAM: &[&str] = &[
    "kind",
    "id",
    "name",
    "description",
    "permission",
    "units_map",
    "includes_all_repositories",
    "can_create_org_repo",
];
const MEMBER: &[&str] = &["kind", "id", "login", "full_name"];
const ORG_REPO: &[&str] = &["kind", "id", "full_name", "private", "html_url"];
const ORG_RESULT: &[&str] = &["kind", "action", "ok", "target"];
const JSON_INPUT: &str = "--input PATH|- (JSON object from a file or stdin)";
const RESULT: &[&str] = &["kind", "action", "ok", "number", "html_url"];
const WEBHOOK: &[&str] = &["id", "type", "active", "events", "branch_filter"];
const WEBHOOK_RESULT: &[&str] = &["kind", "action", "ok", "id"];
const ISSUE: &[&str] = &[
    "kind",
    "number",
    "title",
    "body",
    "html_url",
    "author",
    "state",
    "labels",
    "assignees",
    "milestone",
    "created_at",
    "updated_at",
];
const PULL: &[&str] = &[
    "author_id",
    "kind",
    "number",
    "title",
    "body",
    "html_url",
    "author",
    "state",
    "draft",
    "mergeable",
    "base",
    "head",
    "head_sha",
    "created_at",
    "updated_at",
    "merged",
    "merged_at",
    "merge_commit_sha",
    "labels",
    "milestone",
    "assignees",
];
const RUN: &[&str] = &[
    "kind",
    "id",
    "name",
    "event",
    "status",
    "conclusion",
    "head_branch",
    "head_sha",
    "html_url",
    "created_at",
    "updated_at",
];
const RELEASE: &[&str] = &[
    "kind",
    "id",
    "tag",
    "title",
    "body",
    "html_url",
    "draft",
    "prerelease",
    "target",
    "created_at",
    "published_at",
];
const LABEL: &[&str] = &[
    "kind",
    "id",
    "name",
    "color",
    "description",
    "exclusive",
    "archived",
];
const MILESTONE: &[&str] = &[
    "kind",
    "id",
    "title",
    "description",
    "state",
    "open_issues",
    "closed_issues",
    "due_on",
    "created_at",
    "updated_at",
    "closed_at",
];
const COMMENT: &[&str] = &[
    "kind",
    "id",
    "body",
    "author",
    "html_url",
    "created_at",
    "updated_at",
];
const RESULT_ID: &[&str] = &["kind", "action", "ok", "number", "html_url", "id"];
const PR_DISCUSSION: &[&str] = &[
    "kind",
    "number",
    "id",
    "body",
    "author",
    "html_url",
    "created_at",
    "updated_at",
];
const PR_REVIEW: &[&str] = &[
    "kind",
    "number",
    "id",
    "body",
    "author",
    "state",
    "html_url",
    "commit_id",
    "submitted_at",
    "dismissed",
    "stale",
    "comments_count",
];
const PR_COMMENT: &[&str] = &[
    "kind",
    "number",
    "id",
    "body",
    "author",
    "state",
    "html_url",
    "created_at",
    "updated_at",
    "commit_id",
    "path",
    "old_position",
    "new_position",
    "pull_request_review_id",
];
const BOARD: &[&str] = &[
    "kind",
    "id",
    "title",
    "description",
    "state",
    "open_issues",
    "closed_issues",
    "html_url",
    "created_at",
    "updated_at",
];
const BOARD_COLUMN: &[&str] = &["kind", "board", "id", "title", "default", "cards"];
const BOARD_RESULT: &[&str] = &[
    "kind",
    "action",
    "ok",
    "number",
    "board",
    "column",
    "column_id",
];
const SSH_ALIAS_RESULT: &[&str] = &["kind", "action", "ok", "host", "ssh_alias"];
const BOARD_FLAG: &str =
    "--board TITLE | --board-id ID (default: the repository's only open board)";
const CREATED_URL: &str = "plain output also writes the new URL alone on one stderr line after a successful create; --json and --dry-run keep stderr empty";
const PAGING: &str = "--page N (default 1), --all, --limit N (default 30)";
const BODY: &str = "--body TEXT | --body-file PATH|-";
const METADATA: &str = "--label NAME, --label-id ID, --assignee NAME (repeatable), --milestone NAME | --milestone-id ID";
const EDIT: &str = "--title TEXT, --body TEXT | --body-file PATH|-, --add-label NAME, --remove-label NAME, --add-label-id ID, --remove-label-id ID, --add-assignee NAME, --remove-assignee NAME, --milestone NAME | --milestone-id ID | --clear-milestone";
const COMMON: &[&str] = &[
    "--host URL",
    "-R OWNER/REPO",
    "--json",
    "--fields FIELD,... (requires --json)",
    "-h, --help",
];

#[derive(Serialize)]
struct CommandMeta {
    name: &'static str,
    usage: &'static str,
    #[serde(skip)]
    flags: &'static [&'static str],
    examples: &'static [&'static str],
    output_fields: &'static [&'static str],
    write: bool,
    destructive: bool,
}

impl CommandMeta {
    fn flags(&self) -> Vec<&'static str> {
        let mut flags = self.flags.to_vec();
        if self.write && !self.name.starts_with("auth ") && self.name != "api" {
            flags.push("--dry-run (show request without writing)");
        }
        if self.destructive && self.name != "api" {
            flags.push("--yes (required confirmation, including dry-run)");
        }
        flags
    }
}

#[derive(Serialize)]
struct SchemaCommand {
    #[serde(flatten)]
    metadata: &'static CommandMeta,
    flags: Vec<&'static str>,
    #[serde(flatten)]
    arguments: ArgumentMetadata,
}

#[derive(Serialize)]
struct Positional {
    name: &'static str,
    required: bool,
}

#[derive(Default, Serialize)]
struct ArgumentMetadata {
    required_flags: &'static [&'static str],
    positionals: Vec<Positional>,
    enum_values: std::collections::BTreeMap<&'static str, &'static [&'static str]>,
    mutually_exclusive: Vec<&'static [&'static str]>,
    required_one_of: Vec<&'static [&'static str]>,
    constraints: Vec<&'static str>,
}

impl CommandMeta {
    fn arguments(&self) -> ArgumentMetadata {
        let mut arguments = ArgumentMetadata {
            required_flags: match self.name {
                "issue create" | "milestone create" => &["--title"],
                "pr create" => &["--head", "--title"],
                "pr review" => &["--event"],
                "pr merge" | "branch delete" | "webhook delete" | "org delete"
                | "org remove-member" | "team delete" | "team remove-member"
                | "team remove-repo" => &["--yes"],
                "webhook create" | "webhook edit" | "org create" | "org edit" | "team create"
                | "team edit" => &["--input"],
                "release create" => &["--tag", "--title"],
                "label create" => &["--name", "--color"],
                "workflow dispatch" => &["--ref"],
                "schema" => &["--json"],
                _ => &[],
            },
            ..ArgumentMetadata::default()
        };
        let names: &[&str] = match self.name {
            "org list" => &["user"],
            "org view" | "org edit" | "org delete" | "org members" | "org repos" | "team list"
            | "team create" => &["org"],
            "org remove-member" => &["org", "user"],
            "team add-member" | "team remove-member" => &["id", "user"],
            "team add-repo" | "team remove-repo" => &["id", "repo"],
            "auth git-credential" => &["operation"],
            "auth add-ssh-alias" | "auth remove-ssh-alias" => &["alias"],
            "board move" => &["number", "column"],
            "issue view" | "issue comments" | "issue comment" | "issue edit" | "issue close"
            | "issue reopen" | "pr view" | "pr comments" | "pr comment" | "pr edit"
            | "pr close" | "pr reopen" | "pr reviews" | "pr files" | "pr request-review"
            | "pr diff" | "pr checks" | "pr review" | "pr merge" | "board remove" => &["number"],
            "pr review-comments" => &["number", "review_id"],
            "team view" | "team edit" | "team delete" | "team members" | "team repos"
            | "run view" | "run watch" | "webhook view" | "webhook edit" | "webhook delete"
            | "webhook test" => &["id"],
            "release view" => &["tag"],
            "release upload" => &["release_id", "path"],
            "branch delete" => &["name"],
            "workflow dispatch" => &["file"],
            "api" => &["path"],
            "help" | "schema" => &["command", "action"],
            _ => &[],
        };
        arguments.positionals = names
            .iter()
            .map(|name| Positional {
                name,
                required: !matches!(self.name, "help" | "schema" | "org list"),
            })
            .collect();
        match self.name {
            "issue list" | "pr list" | "milestone list" | "board list" => {
                arguments
                    .enum_values
                    .insert("--state", &["open", "closed", "all"]);
            }
            "pr review" => {
                arguments
                    .enum_values
                    .insert("--event", &["approve", "request-changes", "comment"]);
            }
            "pr merge" => {
                arguments
                    .enum_values
                    .insert("--style", &["merge", "rebase", "rebase-merge", "squash"]);
            }
            "api" => {
                arguments
                    .enum_values
                    .insert("-X", &["GET", "POST", "PUT", "PATCH", "DELETE"]);
            }
            "auth git-credential" => {
                arguments
                    .enum_values
                    .insert("operation", &["get", "store", "erase"]);
            }
            _ => {}
        }
        self.argument_constraints(&mut arguments);
        self.board_and_alias_constraints(&mut arguments);
        arguments
    }

    fn board_and_alias_constraints(&self, arguments: &mut ArgumentMetadata) {
        match self.name {
            "board view" | "board move" | "board remove" => {
                arguments
                    .mutually_exclusive
                    .push(&["--board", "--board-id"]);
                arguments.constraints.push(
                    "--board or --board-id is required when the repository has no open board or more than one",
                );
            }
            "issue create" => arguments
                .mutually_exclusive
                .push(&["--board", "--board-id"]),
            "auth add-ssh-alias" | "auth remove-ssh-alias" => {
                arguments
                    .constraints
                    .push("requires --host URL or FJX_HOST; the host is never inferred");
                if self.name == "auth add-ssh-alias" {
                    arguments.constraints.push(
                        "the host needs a saved login; an alias names one host and cannot be a saved host's DNS name",
                    );
                }
            }
            _ => {}
        }
    }

    fn argument_constraints(&self, arguments: &mut ArgumentMetadata) {
        arguments.mutually_exclusive.push(&["--human", "--json"]);
        arguments
            .constraints
            .push("--fields requires --json; --human is only valid for issue view and pr view");
        if self.flags.contains(&PAGING) {
            arguments.mutually_exclusive.push(&["--page", "--all"]);
        }
        if self.flags.contains(&BODY) || self.flags.contains(&EDIT) {
            arguments
                .mutually_exclusive
                .push(&["--body", "--body-file"]);
        }
        if self.flags.contains(&METADATA) || self.flags.contains(&EDIT) {
            arguments
                .mutually_exclusive
                .push(&["--milestone", "--milestone-id"]);
        }
        if self.flags.contains(&EDIT) {
            arguments.mutually_exclusive.push(&[
                "--milestone",
                "--milestone-id",
                "--clear-milestone",
            ]);
            arguments.required_one_of.push(&[
                "--title",
                "--body",
                "--body-file",
                "--add-label",
                "--remove-label",
                "--add-label-id",
                "--remove-label-id",
                "--add-assignee",
                "--remove-assignee",
                "--milestone",
                "--milestone-id",
                "--clear-milestone",
            ]);
            if self.name == "pr edit" {
                arguments.required_one_of[0] = &[
                    "--title",
                    "--body",
                    "--body-file",
                    "--base",
                    "--add-label",
                    "--remove-label",
                    "--add-label-id",
                    "--remove-label-id",
                    "--add-assignee",
                    "--remove-assignee",
                    "--milestone",
                    "--milestone-id",
                    "--clear-milestone",
                ];
            }
            arguments
                .constraints
                .push("cannot add and remove the same label, label ID, or assignee");
        }
        match self.name {
            "auth login" => {
                arguments
                    .mutually_exclusive
                    .push(&["--with-token", "--web"]);
                arguments
                    .mutually_exclusive
                    .push(&["--with-token", "--no-browser"]);
                arguments
                    .mutually_exclusive
                    .push(&["--with-token", "--client-id"]);
                arguments.constraints.push("interactive login uses browser OAuth; piped input reads a token unless --web, --no-browser or --client-id selects OAuth");
                arguments.constraints.push("OAuth client ID order: --client-id, FJX_CLIENT_ID, saved host registration, then the built-in git-credential-oauth public client; a custom public client needs redirect URI http://127.0.0.1/");
            }
            "issue comment" | "pr comment" => {
                arguments.required_one_of.push(&["--body", "--body-file"]);
            }
            "pr request-review" => arguments.required_one_of.push(&["--reviewer", "--team"]),
            "pr review" => {
                arguments
                    .constraints
                    .push("--comments-file requires --commit");
                arguments.constraints.push("--event request-changes or comment requires one of --body, --body-file, --comments-file");
            }
            "api" => {
                arguments
                    .constraints
                    .push("DELETE requires --yes; --yes is only valid for DELETE");
                arguments
                    .constraints
                    .push("--paginate requires GET; --dry-run requires a write method");
            }
            "issue list" | "pr list" => arguments
                .constraints
                .push("--sort is forwarded to Forgejo; accepted values depend on the server"),
            "issue create" | "pr create" => arguments.constraints.push(CREATED_URL),
            _ => {}
        }
    }
}

macro_rules! command {
    ($name:literal, $usage:literal, [$($flag:expr),*], $example:literal, $fields:expr, $write:literal, $destructive:literal) => {
        CommandMeta { name: $name, usage: $usage, flags: &[$($flag),*], examples: &[$example], output_fields: $fields, write: $write, destructive: $destructive }
    };
}

static COMMANDS: &[CommandMeta] = &[
    command!(
        "org list",
        "fjx org list [USER]",
        [PAGING],
        "fjx org list --all --json",
        ORGANIZATION,
        false,
        false
    ),
    command!(
        "org view",
        "fjx org view ORG",
        [],
        "fjx org view my-org --json",
        ORGANIZATION,
        false,
        false
    ),
    command!(
        "org create",
        "fjx org create --input PATH|-",
        [JSON_INPUT],
        "fjx org create --input org.json --dry-run",
        ORGANIZATION,
        true,
        false
    ),
    command!(
        "org edit",
        "fjx org edit ORG --input PATH|-",
        [JSON_INPUT],
        "fjx org edit my-org --input org.json --dry-run",
        ORGANIZATION,
        true,
        false
    ),
    command!(
        "org delete",
        "fjx org delete ORG --yes",
        [],
        "fjx org delete my-org --yes --dry-run",
        ORG_RESULT,
        true,
        true
    ),
    command!(
        "org members",
        "fjx org members ORG",
        [PAGING],
        "fjx org members my-org --all --json",
        MEMBER,
        false,
        false
    ),
    command!(
        "org repos",
        "fjx org repos ORG",
        [PAGING],
        "fjx org repos my-org --all --json",
        ORG_REPO,
        false,
        false
    ),
    command!(
        "org remove-member",
        "fjx org remove-member ORG USER --yes",
        [],
        "fjx org remove-member my-org alice --yes --dry-run",
        ORG_RESULT,
        true,
        true
    ),
    command!(
        "team list",
        "fjx team list ORG",
        [PAGING],
        "fjx team list my-org --all --json",
        TEAM,
        false,
        false
    ),
    command!(
        "team view",
        "fjx team view ID",
        [],
        "fjx team view 7 --json",
        TEAM,
        false,
        false
    ),
    command!(
        "team create",
        "fjx team create ORG --input PATH|-",
        [JSON_INPUT],
        "fjx team create my-org --input team.json --dry-run",
        TEAM,
        true,
        false
    ),
    command!(
        "team edit",
        "fjx team edit ID --input PATH|-",
        [JSON_INPUT],
        "fjx team edit 7 --input team.json --dry-run",
        TEAM,
        true,
        false
    ),
    command!(
        "team delete",
        "fjx team delete ID --yes",
        [],
        "fjx team delete 7 --yes --dry-run",
        ORG_RESULT,
        true,
        true
    ),
    command!(
        "team members",
        "fjx team members ID",
        [PAGING],
        "fjx team members 7 --all --json",
        MEMBER,
        false,
        false
    ),
    command!(
        "team repos",
        "fjx team repos ID",
        [PAGING],
        "fjx team repos 7 --all --json",
        ORG_REPO,
        false,
        false
    ),
    command!(
        "team add-member",
        "fjx team add-member ID USER",
        [],
        "fjx team add-member 7 alice --dry-run",
        ORG_RESULT,
        true,
        false
    ),
    command!(
        "team remove-member",
        "fjx team remove-member ID USER --yes",
        [],
        "fjx team remove-member 7 alice --yes --dry-run",
        ORG_RESULT,
        true,
        true
    ),
    command!(
        "team add-repo",
        "fjx team add-repo ID OWNER/REPO",
        [],
        "fjx team add-repo 7 my-org/project --dry-run",
        ORG_RESULT,
        true,
        false
    ),
    command!(
        "team remove-repo",
        "fjx team remove-repo ID OWNER/REPO --yes",
        [],
        "fjx team remove-repo 7 my-org/project --yes --dry-run",
        ORG_RESULT,
        true,
        true
    ),
    command!(
        "webhook list",
        "fjx webhook list",
        [PAGING],
        "fjx webhook list -R owner/repo --all --json",
        WEBHOOK,
        false,
        false
    ),
    command!(
        "webhook view",
        "fjx webhook view ID",
        [],
        "fjx webhook view 1 --json",
        WEBHOOK,
        false,
        false
    ),
    command!(
        "webhook create",
        "fjx webhook create --input PATH|-",
        ["--input PATH|- (JSON file or stdin; never credentials in arguments)"],
        "fjx webhook create --input hook.json --dry-run",
        WEBHOOK,
        true,
        false
    ),
    command!(
        "webhook edit",
        "fjx webhook edit ID --input PATH|-",
        ["--input PATH|- (partial JSON update)"],
        "fjx webhook edit 1 --input hook.json --dry-run",
        WEBHOOK,
        true,
        false
    ),
    command!(
        "webhook delete",
        "fjx webhook delete ID --yes",
        [],
        "fjx webhook delete 1 --yes --dry-run",
        WEBHOOK_RESULT,
        true,
        true
    ),
    command!(
        "webhook test",
        "fjx webhook test ID",
        ["--ref REF (optional commit, branch or tag)"],
        "fjx webhook test 1 --ref main",
        WEBHOOK_RESULT,
        true,
        false
    ),
    command!(
        "auth login",
        "fjx auth login [--web | --with-token] [--client-id ID] [--no-browser]",
        [
            "--web (browser OAuth, including without a terminal)",
            "--with-token (hidden prompt or stdin)",
            "--client-id ID (override public OAuth application; also FJX_CLIENT_ID)",
            "--no-browser (print the OAuth link without opening it)"
        ],
        "fjx auth login --host https://forge.example",
        RESULT,
        true,
        false
    ),
    command!(
        "auth status",
        "fjx auth status",
        [],
        "fjx auth status --json",
        &["kind", "host", "user", "source", "ssh_aliases"],
        false,
        false
    ),
    command!(
        "auth logout",
        "fjx auth logout",
        [],
        "fjx auth logout",
        RESULT,
        true,
        false
    ),
    command!(
        "auth setup-git",
        "fjx auth setup-git",
        [],
        "fjx auth setup-git",
        RESULT,
        true,
        false
    ),
    command!(
        "auth git-credential",
        "fjx auth git-credential get|store|erase",
        [],
        "fjx auth git-credential get",
        &[],
        false,
        false
    ),
    command!(
        "auth add-ssh-alias",
        "fjx auth add-ssh-alias ALIAS --host URL",
        [],
        "fjx auth add-ssh-alias forge-ssh --host https://forge.example",
        SSH_ALIAS_RESULT,
        true,
        false
    ),
    command!(
        "auth remove-ssh-alias",
        "fjx auth remove-ssh-alias ALIAS --host URL",
        [],
        "fjx auth remove-ssh-alias forge-ssh --host https://forge.example",
        SSH_ALIAS_RESULT,
        true,
        false
    ),
    command!(
        "repo view",
        "fjx repo view",
        [],
        "fjx repo view -R owner/repo --json",
        &[
            "kind",
            "name",
            "full_name",
            "description",
            "private",
            "archived",
            "default_branch",
            "html_url"
        ],
        false,
        false
    ),
    command!(
        "issue list",
        "fjx issue list",
        [
            PAGING,
            "--state open|closed|all (default open)",
            "--label NAME (repeatable), --author NAME, --assignee NAME, --search TEXT, --milestone NAME, --since RFC3339, --before RFC3339, --sort VALUE"
        ],
        "fjx issue list --label bug --json --fields number,title",
        ISSUE,
        false,
        false
    ),
    command!(
        "issue view",
        "fjx issue view NUMBER",
        ["--human (cannot combine with --json)"],
        "fjx issue view 12 --human",
        ISSUE,
        false,
        false
    ),
    command!(
        "issue create",
        "fjx issue create --title TEXT",
        [
            BODY,
            METADATA,
            "--board TITLE | --board-id ID (Gitea: add a card in the board's default column)"
        ],
        "fjx issue create --title 'Fix crash' --label bug --dry-run",
        ISSUE,
        true,
        false
    ),
    command!(
        "issue comments",
        "fjx issue comments NUMBER",
        [PAGING],
        "fjx issue comments 12 --all --json",
        COMMENT,
        false,
        false
    ),
    command!(
        "issue edit",
        "fjx issue edit NUMBER",
        [EDIT],
        "fjx issue edit 12 --add-label bug --dry-run",
        RESULT,
        true,
        false
    ),
    command!(
        "issue comment",
        "fjx issue comment NUMBER (--body TEXT | --body-file PATH|-)",
        [BODY],
        "fjx issue comment 12 --body 'Reproduced'",
        RESULT_ID,
        true,
        false
    ),
    command!(
        "issue close",
        "fjx issue close NUMBER",
        [],
        "fjx issue close 12 --dry-run",
        ISSUE,
        true,
        false
    ),
    command!(
        "issue reopen",
        "fjx issue reopen NUMBER",
        [],
        "fjx issue reopen 12",
        ISSUE,
        true,
        false
    ),
    command!(
        "board list",
        "fjx board list",
        [PAGING, "--state open|closed|all (default open)"],
        "fjx board list -R owner/repo --json",
        BOARD,
        false,
        false
    ),
    command!(
        "board view",
        "fjx board view",
        [BOARD_FLAG],
        "fjx board view --json --fields title,cards",
        BOARD_COLUMN,
        false,
        false
    ),
    command!(
        "board move",
        "fjx board move NUMBER COLUMN",
        [BOARD_FLAG],
        "fjx board move 12 'In Progress' --dry-run",
        BOARD_RESULT,
        true,
        false
    ),
    command!(
        "board remove",
        "fjx board remove NUMBER",
        [BOARD_FLAG],
        "fjx board remove 12 --dry-run",
        BOARD_RESULT,
        true,
        false
    ),
    command!(
        "pr list",
        "fjx pr list",
        [
            PAGING,
            "--state open|closed|all (default open)",
            "--label NAME (repeatable), --author NAME, --milestone NAME, --sort VALUE"
        ],
        "fjx pr list --author alice --json",
        PULL,
        false,
        false
    ),
    command!(
        "pr view",
        "fjx pr view NUMBER",
        ["--human (cannot combine with --json)"],
        "fjx pr view 12 --human",
        PULL,
        false,
        false
    ),
    command!(
        "pr create",
        "fjx pr create --head REF --title TEXT",
        [
            "--base REF (default repository branch), --draft",
            BODY,
            METADATA
        ],
        "fjx pr create --head fix --title 'Fix crash' --dry-run",
        PULL,
        true,
        false
    ),
    command!(
        "pr edit",
        "fjx pr edit NUMBER",
        [EDIT, "--base REF"],
        "fjx pr edit 12 --base main --dry-run",
        RESULT,
        true,
        false
    ),
    command!(
        "pr comments",
        "fjx pr comments NUMBER",
        [PAGING],
        "fjx pr comments 12 --all --json",
        PR_DISCUSSION,
        false,
        false
    ),
    command!(
        "pr reviews",
        "fjx pr reviews NUMBER",
        [PAGING],
        "fjx pr reviews 12 --json",
        PR_REVIEW,
        false,
        false
    ),
    command!(
        "pr files",
        "fjx pr files NUMBER",
        [PAGING],
        "fjx pr files 12 --json",
        &[
            "kind",
            "number",
            "id",
            "filename",
            "previous_filename",
            "status",
            "additions",
            "deletions",
            "changes"
        ],
        false,
        false
    ),
    command!(
        "pr review-comments",
        "fjx pr review-comments NUMBER REVIEW_ID",
        [PAGING],
        "fjx pr review-comments 12 34 --json",
        PR_COMMENT,
        false,
        false
    ),
    command!(
        "pr request-review",
        "fjx pr request-review NUMBER",
        ["--reviewer NAME, --team NAME (repeatable; at least one required)"],
        "fjx pr request-review 12 --reviewer alice --dry-run",
        RESULT,
        true,
        false
    ),
    command!(
        "pr diff",
        "fjx pr diff NUMBER",
        [],
        "fjx pr diff 12",
        &["kind", "number", "diff"],
        false,
        false
    ),
    command!(
        "pr checks",
        "fjx pr checks NUMBER",
        [],
        "fjx pr checks 12 --json",
        &["kind", "sha", "state", "statuses"],
        false,
        false
    ),
    command!(
        "pr comment",
        "fjx pr comment NUMBER (--body TEXT | --body-file PATH|-)",
        [BODY],
        "fjx pr comment 12 --body 'Looks good'",
        RESULT_ID,
        true,
        false
    ),
    command!(
        "pr review",
        "fjx pr review NUMBER --event approve|request-changes|comment",
        [BODY, "--comments-file PATH, --commit SHA"],
        "fjx pr review 12 --event approve --dry-run",
        RESULT_ID,
        true,
        false
    ),
    command!(
        "pr merge",
        "fjx pr merge NUMBER --yes",
        [
            "--style merge|rebase|rebase-merge|squash (default merge)",
            "--title TEXT, --message TEXT, --delete-branch, --match-head SHA, --auto"
        ],
        "fjx pr merge 12 --match-head abc123 --yes --dry-run",
        RESULT,
        true,
        true
    ),
    command!(
        "pr close",
        "fjx pr close NUMBER",
        [],
        "fjx pr close 12 --dry-run",
        PULL,
        true,
        false
    ),
    command!(
        "pr reopen",
        "fjx pr reopen NUMBER",
        [],
        "fjx pr reopen 12",
        PULL,
        true,
        false
    ),
    command!(
        "run list",
        "fjx run list",
        [
            PAGING,
            "--status VALUE, --event VALUE, --ref REF, --head-sha SHA, --workflow VALUE"
        ],
        "fjx run list --status failure --json",
        RUN,
        false,
        false
    ),
    command!(
        "run view",
        "fjx run view ID",
        [],
        "fjx run view 42 --json",
        RUN,
        false,
        false
    ),
    command!(
        "run watch",
        "fjx run watch ID",
        ["--poll SECONDS (default 5), --wait SECONDS (default 600)"],
        "fjx run watch 42 --poll 5 --wait 60",
        RUN,
        false,
        false
    ),
    command!(
        "release list",
        "fjx release list",
        [PAGING],
        "fjx release list --json",
        RELEASE,
        false,
        false
    ),
    command!(
        "release view",
        "fjx release view TAG",
        [],
        "fjx release view v1.0.0 --json",
        RELEASE,
        false,
        false
    ),
    command!(
        "release create",
        "fjx release create --tag TAG --title TEXT",
        [BODY, "--target REF, --draft, --prerelease"],
        "fjx release create --tag v1.0.0 --title v1.0.0 --draft --dry-run",
        RELEASE,
        true,
        false
    ),
    command!(
        "release upload",
        "fjx release upload RELEASE_ID PATH",
        ["--name NAME (default file name)"],
        "fjx release upload 42 dist/app.tar.gz --dry-run",
        &[
            "kind",
            "id",
            "name",
            "size",
            "browser_download_url",
            "created_at"
        ],
        true,
        false
    ),
    command!(
        "label list",
        "fjx label list",
        [PAGING],
        "fjx label list --json",
        LABEL,
        false,
        false
    ),
    command!(
        "label create",
        "fjx label create --name TEXT --color HEX",
        ["--description TEXT"],
        "fjx label create --name bug --color ff0000 --dry-run",
        LABEL,
        true,
        false
    ),
    command!(
        "milestone list",
        "fjx milestone list",
        [PAGING, "--state open|closed|all (default open)"],
        "fjx milestone list --json",
        MILESTONE,
        false,
        false
    ),
    command!(
        "milestone create",
        "fjx milestone create --title TEXT",
        ["--description TEXT, --due RFC3339"],
        "fjx milestone create --title v1 --dry-run",
        MILESTONE,
        true,
        false
    ),
    command!(
        "branch list",
        "fjx branch list",
        [PAGING],
        "fjx branch list --json",
        &[
            "kind",
            "name",
            "sha",
            "protected",
            "status_checks",
            "required_approvals",
            "can_merge",
            "can_push"
        ],
        false,
        false
    ),
    command!(
        "branch delete",
        "fjx branch delete NAME --yes",
        [],
        "fjx branch delete old --yes --dry-run",
        &["kind", "action", "ok", "name"],
        true,
        true
    ),
    command!(
        "workflow dispatch",
        "fjx workflow dispatch FILE --ref REF",
        ["--field KEY=VALUE (repeatable)"],
        "fjx workflow dispatch build.yml --ref main --field target=linux --dry-run",
        &["kind", "id", "run_number", "jobs"],
        true,
        false
    ),
    command!(
        "mcp serve",
        "fjx mcp serve --host URL",
        [],
        "fjx mcp serve --host https://forge.example",
        &[],
        false,
        false
    ),
    command!(
        "mcp config",
        "fjx mcp config --host URL --json",
        [],
        "fjx mcp config --host https://forge.example --json",
        &[],
        false,
        false
    ),
    command!(
        "api",
        "fjx api PATH",
        [
            "-X GET|POST|PUT|PATCH|DELETE (default GET, POST with --input)",
            "--input PATH|-, --paginate (GET only)",
            "--dry-run (write only), --yes (DELETE only)"
        ],
        "fjx api /repos/owner/repo --json",
        &[],
        true,
        true
    ),
    command!(
        "help",
        "fjx help [COMMAND [ACTION]]",
        [],
        "fjx help pr merge",
        &[],
        false,
        false
    ),
    command!(
        "schema",
        "fjx schema [COMMAND [ACTION]] --json",
        [],
        "fjx schema issue view --json",
        &[],
        false,
        false
    ),
    command!(
        "version",
        "fjx --version",
        ["-V, --version"],
        "fjx --version",
        &[],
        false,
        false
    ),
];

const GROUPS: &[&str] = &[
    "mcp",
    "org",
    "team",
    "auth",
    "repo",
    "issue",
    "board",
    "pr",
    "run",
    "release",
    "label",
    "milestone",
    "branch",
    "workflow",
    "webhook",
];

fn select(path: &[String]) -> Result<Vec<&'static CommandMeta>, Error> {
    let name = path.join(" ");
    if path.is_empty() {
        return Ok(COMMANDS.iter().collect());
    }
    if let Some(command) = COMMANDS.iter().find(|command| command.name == name) {
        return Ok(vec![command]);
    }
    if path.len() == 1 && GROUPS.contains(&path[0].as_str()) {
        return Ok(COMMANDS
            .iter()
            .filter(|command| {
                command
                    .name
                    .strip_prefix(&name)
                    .is_some_and(|tail| tail.starts_with(' '))
            })
            .collect());
    }
    let hint = path
        .first()
        .filter(|group| GROUPS.contains(&group.as_str()))
        .map_or_else(
            || "fjx help".to_owned(),
            |group| format!("fjx help {group}"),
        );
    Err(Error::usage(format!(
        "unknown command path {name:?}; run {hint} for available commands"
    )))
}

fn format_error(error: std::fmt::Error) -> Error {
    Error::data(format!("could not format help: {error}"))
}

fn organization_help(name: &str, text: &mut String) {
    if name.starts_with("org ") || name.starts_with("team ") {
        text.push_str("Context: host and token required; no repository required. Team IDs come from team list/create. Lists use the standard 1,000-item cap.\n");
    }
    if name == "org list" {
        text.push_str("Lists your memberships by default; USER lists that account's organizations visible to you.\n");
    }
    if matches!(name, "org create" | "org edit") {
        text.push_str("Input fields: username (required on create; forbidden on edit), full_name, description, email, website, location, visibility (public/limited/private), repo_admin_change_team_access (boolean). Empty strings clear profile fields; unknown fields and null are rejected.\n");
    }
    if name == "org edit" {
        text.push_str("Reads and preserves omitted profile fields, including in dry runs; concurrent edits can race. Renaming uses the raw API.\n");
    }
    if matches!(name, "team create" | "team edit") {
        text.push_str("Input fields: name (required on create), description, permission (read/write/admin; create default read), units_map, includes_all_repositories, can_create_org_repo. Non-admin creation and permission changes require a nonempty explicit units_map, e.g. {\"repo.code\":\"read\",\"repo.issues\":\"write\"}. Unit values: none/read/write; external units: none/read. The map replaces all unit permissions; Forgejo derives the team's base permission from it. Admin grants all units and cannot combine with units_map. Unknown fields and null are rejected.\n");
    }
    if name == "team edit" {
        text.push_str("Reads the team before editing, including in dry runs; preserves omitted values and supplies the current name/permission where Forgejo requires them. Concurrent edits can race. Owners team permission changes are refused.\n");
    }
    if name == "team add-member" {
        text.push_str("Adding a team member also joins them to the organization. Forgejo enforces owner/admin permissions.\n");
    }
    if matches!(name, "team add-repo" | "team remove-repo") {
        text.push_str(
            "OWNER/REPO must belong to the team's organization; Forgejo enforces this.\n",
        );
    }
}

fn board_help(name: &str, text: &mut String) {
    if name.starts_with("board ") {
        text.push_str("Boards are Gitea repository projects. Forgejo through version 17 has no project-board API: board commands exit 7 there without writing.\n");
    }
    match name {
        "board view" => text.push_str("Lists columns in board order; each column's cards are {number, state, title, pull_request, html_url}. Plain output is one `column ID TITLE COUNT` line per column, followed by its `card COLUMN NUMBER STATE TITLE` lines.\n"),
        "board move" => text.push_str("NUMBER is an issue or pull-request number. COLUMN is a column title, matched exactly and then ignoring case; --board matches a board title the same way. An issue without a card is added to the board; a card already on it moves. Reads the board, its columns and the issue first, including in dry runs.\n"),
        "board remove" => text.push_str("Removes the card and keeps the issue; board move adds it again. Reads each column to find the card, including in dry runs.\n"),
        "issue create" => text.push_str("--board and --board-id resolve the board before creating the issue and send its ID as projects. A forge without that field would ignore it, so fjx refuses a forge without a board API. Place the card with board move.\n"),
        "auth add-ssh-alias" => text.push_str("Makes repository remotes whose SSH host is ALIAS (for example a Host entry in ~/.ssh/config) resolve to the saved login for --host. Remotes that name the host's own DNS name need no alias. The alias is saved with the login, kept across auth login, and removed by auth logout; auth status --json lists it. The remote's repository path is used as given.\n"),
        _ => {}
    }
}

pub(crate) fn render(path: &[String]) -> Result<Outcome, Error> {
    let commands = select(path)?;
    let mut text = String::new();
    if path.is_empty() {
        text.push_str(concat!(
            "fjx ",
            env!("CARGO_PKG_VERSION"),
            " - a small Forgejo client\n"
        ));
        text.push_str("\nUsage: fjx COMMAND [ACTION] [OPTIONS]\nCommands: auth, repo, org, team, issue, board, pr, run, release, label, milestone, branch, workflow, webhook, mcp\n  fjx api PATH\n  fjx help [COMMAND [ACTION]]\n  fjx schema [COMMAND [ACTION]] --json\n  fjx --version\n\nExit codes: 0 success, 1 unsuccessful work, 2 usage, 3 context/auth,\n4 network/TLS/timeout, 5 Forgejo HTTP, 6 safety, 7 incompatible data,\n8 local file/VCS, 130 interrupted\n");
    } else if commands.len() == 1 && !(path.len() == 1 && GROUPS.contains(&path[0].as_str())) {
        let command = commands[0];
        writeln!(text, "Usage: {}", command.usage).map_err(format_error)?;
        let arguments = command.arguments();
        if !arguments.required_flags.is_empty() {
            writeln!(
                text,
                "Required flags: {}",
                arguments.required_flags.join(", ")
            )
            .map_err(format_error)?;
        }
        if matches!(
            command.name,
            "issue comment" | "pr comment" | "pr request-review"
        ) {
            for alternatives in arguments.required_one_of {
                writeln!(text, "Requires one of: {}", alternatives.join(", "))
                    .map_err(format_error)?;
            }
        }
        for flag in command.flags() {
            writeln!(text, "  {flag}").map_err(format_error)?;
        }
        for example in command.examples {
            writeln!(text, "Example: {example}").map_err(format_error)?;
        }
        if command.output_fields.is_empty() {
            text.push_str("Output: command-specific data; typed field projection unsupported.\n");
        } else {
            writeln!(text, "Output fields: {}", command.output_fields.join(", "))
                .map_err(format_error)?;
        }
        organization_help(command.name, &mut text);
        board_help(command.name, &mut text);
        if command.name == "api" {
            text.push_str("Safety: raw DELETE requires --yes; writes are never retried. Raw API output has no typed field schema.\n");
        }
        if matches!(command.name, "webhook create" | "webhook edit") {
            text.push_str("Input: config {url, content_type: json, http_method: POST, secret}, events [names], active, branch_filter, authorization_header. Create defaults type to forgejo and active to true; config.url and nonempty events are required. Edit changes only supplied top-level fields; supplied config requires url. Other providers use fjx api.\nSafety: URLs, secrets and authorization headers are omitted from results and redacted in dry runs. Protect input files; prefer stdin from your secret store.\n");
        }
        if command.name == "webhook test" {
            text.push_str("Queues a push test. Success means queued, not delivered. Forgejo 17 has no delivery-history API; inspect Recent Deliveries in repository Settings > Webhooks.\n");
        }
        if command.name == "webhook edit" {
            text.push_str("Forgejo 17 resets omitted events, branch_filter and authorization_header. fjx reads and preserves them, even during dry runs; concurrent edits can race. Signing-secret rotation and workflow/action-run/package subscription changes require a replacement hook because the server ignores those edits.\n");
            text.push_str("Hooks reporting every event require a replacement: the API hides Send everything mode, whose edits can lose CI/package subscriptions. Explicit all-event selections are indistinguishable and also refused, including dry runs.\n");
        }
        if matches!(command.name, "issue create" | "pr create") {
            writeln!(text, "Stderr: {CREATED_URL}.").map_err(format_error)?;
        }
        if command.name == "pr merge" {
            text.push_str("Safety: --match-head guards the expected head SHA; --auto requests server-side automatic merge.\n");
        }
        if command.write {
            text.push_str(
                "Safety: writes are never retried and HTTP redirects are never followed.\n",
            );
        }
    } else {
        writeln!(text, "Usage: fjx {} COMMAND\nCommands:", path.join(" ")).map_err(format_error)?;
        for command in commands {
            writeln!(text, "  {}", command.usage).map_err(format_error)?;
        }
        writeln!(
            text,
            "Run fjx help {} COMMAND for flags and examples.",
            path.join(" ")
        )
        .map_err(format_error)?;
    }
    text.push_str("\nCommon: --host URL, -R OWNER/REPO, --json, --fields FIELD,... (requires --json), --help\n--human is only for issue/pr view and cannot combine with --json.\nHelp and schema need no token, repository, stdin, or network.\n");
    Ok(Outcome::text(text))
}

#[derive(Serialize)]
struct Envelope {
    schema_version: u8,
    commands: Vec<SchemaCommand>,
    common_flags: &'static [&'static str],
}

pub(crate) fn schema(path: &[String]) -> Result<Outcome, Error> {
    Outcome::json(&Envelope {
        schema_version: 1,
        commands: select(path)?
            .into_iter()
            .map(|metadata| SchemaCommand {
                flags: metadata.flags(),
                arguments: metadata.arguments(),
                metadata,
            })
            .collect(),
        common_flags: COMMON,
    })
}

pub(crate) fn output_fields_for(group: &str, action: &str) -> Option<&'static [&'static str]> {
    COMMANDS
        .iter()
        .find(|command| command.name.split_once(' ') == Some((group, action)))
        .map(|command| command.output_fields)
        .filter(|fields| !fields.is_empty())
}

pub(crate) fn output_fields(command: &crate::args::Command) -> Option<&'static [&'static str]> {
    use crate::args::{
        BranchArgs, Command, IssueArgs, LabelArgs, MilestoneArgs, PullArgs, ReleaseArgs, RunArgs,
    };
    let (group, action) = match command {
        Command::AuthLogin { .. } => ("auth", "login"),
        Command::AuthStatus => ("auth", "status"),
        Command::AuthLogout => ("auth", "logout"),
        Command::AuthSetupGit => ("auth", "setup-git"),
        Command::AuthSshAlias { remove: true, .. } => ("auth", "remove-ssh-alias"),
        Command::AuthSshAlias { .. } => ("auth", "add-ssh-alias"),
        Command::RepoView => ("repo", "view"),
        Command::Board(args) => ("board", args.action()),
        Command::Org(args) => ("org", args.action()),
        Command::Team(args) => ("team", args.action()),
        Command::Issue(args) => (
            "issue",
            match args {
                IssueArgs::List { .. } => "list",
                IssueArgs::View { .. } => "view",
                IssueArgs::Create { .. } => "create",
                IssueArgs::Comment { .. } => "comment",
                IssueArgs::Comments { .. } => "comments",
                IssueArgs::Edit { .. } => "edit",
                IssueArgs::Close { .. } => "close",
                IssueArgs::Reopen { .. } => "reopen",
            },
        ),
        Command::Pull(args) => (
            "pr",
            match args {
                PullArgs::List { .. } => "list",
                PullArgs::View { .. } => "view",
                PullArgs::Create { .. } => "create",
                PullArgs::Comment { .. } => "comment",
                PullArgs::Comments { .. } => "comments",
                PullArgs::Edit { .. } => "edit",
                PullArgs::Close { .. } => "close",
                PullArgs::Reopen { .. } => "reopen",
                PullArgs::Reviews { .. } => "reviews",
                PullArgs::Files { .. } => "files",
                PullArgs::ReviewComments { .. } => "review-comments",
                PullArgs::RequestReview { .. } => "request-review",
                PullArgs::Diff { .. } => "diff",
                PullArgs::Checks { .. } => "checks",
                PullArgs::Review { .. } => "review",
                PullArgs::Merge { .. } => "merge",
            },
        ),
        Command::Run(args) => (
            "run",
            match args {
                RunArgs::List { .. } => "list",
                RunArgs::View { .. } => "view",
                RunArgs::Watch { .. } => "watch",
            },
        ),
        Command::Release(args) => (
            "release",
            match args {
                ReleaseArgs::List { .. } => "list",
                ReleaseArgs::View { .. } => "view",
                ReleaseArgs::Create { .. } => "create",
                ReleaseArgs::Upload { .. } => "upload",
            },
        ),
        Command::Label(args) => (
            "label",
            match args {
                LabelArgs::List { .. } => "list",
                LabelArgs::Create { .. } => "create",
            },
        ),
        Command::Milestone(args) => (
            "milestone",
            match args {
                MilestoneArgs::List { .. } => "list",
                MilestoneArgs::Create { .. } => "create",
            },
        ),
        Command::Branch(args) => (
            "branch",
            match args {
                BranchArgs::List { .. } => "list",
                BranchArgs::Delete { .. } => "delete",
            },
        ),
        Command::Workflow(_) => ("workflow", "dispatch"),
        Command::Webhook(args) => ("webhook", args.action()),
        Command::McpServe => ("mcp", "serve"),
        Command::McpConfig => ("mcp", "config"),
        Command::AuthGitCredential { .. }
        | Command::Api(_)
        | Command::Help { .. }
        | Command::Schema { .. }
        | Command::Version => return None,
    };
    output_fields_for(group, action)
}
