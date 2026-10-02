use crate::error::Error;
use crate::output::Outcome;
use serde::Serialize;
use std::fmt::Write;

const RESULT: &[&str] = &["kind", "action", "ok", "number", "html_url"];
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
                "pr merge" | "branch delete" => &["--yes"],
                "release create" => &["--tag", "--title"],
                "label create" => &["--name", "--color"],
                "workflow dispatch" => &["--ref"],
                "schema" => &["--json"],
                _ => &[],
            },
            ..ArgumentMetadata::default()
        };
        let names: &[&str] = match self.name {
            "auth git-credential" => &["operation"],
            "issue view" | "issue comments" | "issue comment" | "issue edit" | "issue close"
            | "issue reopen" | "pr view" | "pr comments" | "pr comment" | "pr edit"
            | "pr close" | "pr reopen" | "pr reviews" | "pr files" | "pr request-review"
            | "pr diff" | "pr checks" | "pr review" | "pr merge" => &["number"],
            "pr review-comments" => &["number", "review_id"],
            "run view" | "run watch" => &["id"],
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
                required: !matches!(self.name, "help" | "schema"),
            })
            .collect();
        match self.name {
            "issue list" | "pr list" | "milestone list" => {
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
        arguments
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
        "auth login",
        "fjx auth login [--with-token]",
        ["--with-token (read token from stdin)"],
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
        &["kind", "host", "user", "source"],
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
        [BODY, METADATA],
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
    "auth",
    "repo",
    "issue",
    "pr",
    "run",
    "release",
    "label",
    "milestone",
    "branch",
    "workflow",
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

pub(crate) fn render(path: &[String]) -> Result<Outcome, Error> {
    let commands = select(path)?;
    let mut text = String::new();
    if path.is_empty() {
        text.push_str(concat!(
            "fjx ",
            env!("CARGO_PKG_VERSION"),
            " - a small Forgejo client\n"
        ));
        text.push_str("\nUsage: fjx COMMAND [ACTION] [OPTIONS]\nCommands: auth, repo, issue, pr, run, release, label, milestone, branch, workflow\n  fjx api PATH\n  fjx help [COMMAND [ACTION]]\n  fjx schema [COMMAND [ACTION]] --json\n  fjx --version\n\nExit codes: 0 success, 1 unsuccessful work, 2 usage, 3 context/auth,\n4 network/TLS/timeout, 5 Forgejo HTTP, 6 safety, 7 incompatible data,\n8 local file/VCS, 130 interrupted\n");
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
        if command.name == "api" {
            text.push_str("Safety: raw DELETE requires --yes; writes are never retried. Raw API output has no typed field schema.\n");
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
        Command::RepoView => ("repo", "view"),
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
        Command::AuthGitCredential { .. }
        | Command::Api(_)
        | Command::Help { .. }
        | Command::Schema { .. }
        | Command::Version => return None,
    };
    output_fields_for(group, action)
}
