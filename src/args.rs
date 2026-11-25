use std::ffi::OsString;

use lexopt::prelude::*;

use crate::error::Error;

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent global CLI switches"
)]
pub(crate) struct Args {
    pub(crate) host: Option<String>,
    pub(crate) repo: Option<String>,
    pub(crate) json: bool,
    pub(crate) fields: Vec<String>,
    pub(crate) human: bool,
    pub(crate) dry_run: bool,
    pub(crate) yes: bool,
    pub(crate) command: Command,
}

#[derive(Debug)]
pub(crate) enum Command {
    AuthLogin { options: LoginArgs },
    AuthStatus,
    AuthLogout,
    AuthSetupGit,
    AuthGitCredential { operation: GitCredentialOperation },
    AuthSshAlias { alias: String, remove: bool },
    RepoView,
    Board(BoardArgs),
    McpServe,
    McpConfig,
    Org(OrgArgs),
    Team(TeamArgs),
    Issue(IssueArgs),
    Pull(PullArgs),
    Run(RunArgs),
    Release(ReleaseArgs),
    Label(LabelArgs),
    Milestone(MilestoneArgs),
    Branch(BranchArgs),
    Workflow(WorkflowArgs),
    Webhook(WebhookArgs),
    Api(ApiArgs),
    Help { path: Vec<String> },
    Schema { path: Vec<String> },
    Version,
}

#[derive(Debug, Default)]
pub(crate) struct LoginArgs {
    pub(crate) with_token: bool,
    pub(crate) web: bool,
    pub(crate) no_browser: bool,
    pub(crate) client_id: Option<String>,
}

#[derive(Debug)]
pub(crate) enum WebhookArgs {
    List { paging: PageArgs },
    View { id: u64 },
    Create { input: OsString },
    Edit { id: u64, input: OsString },
    Delete { id: u64 },
    Test { id: u64, reference: Option<String> },
}

impl WebhookArgs {
    pub(crate) const fn action(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::View { .. } => "view",
            Self::Create { .. } => "create",
            Self::Edit { .. } => "edit",
            Self::Delete { .. } => "delete",
            Self::Test { .. } => "test",
        }
    }
}

/// A board named by title or by ID. As with labels and milestones, an ID is
/// never guessed from a numeric-looking title.
#[derive(Debug)]
pub(crate) enum BoardSelector {
    Title(String),
    Id(u64),
}

#[derive(Debug)]
pub(crate) enum BoardArgs {
    List {
        state: ListState,
        paging: PageArgs,
    },
    View {
        board: Option<BoardSelector>,
    },
    Move {
        number: u64,
        column: String,
        board: Option<BoardSelector>,
    },
    Remove {
        number: u64,
        board: Option<BoardSelector>,
    },
}

impl BoardArgs {
    pub(crate) const fn action(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::View { .. } => "view",
            Self::Move { .. } => "move",
            Self::Remove { .. } => "remove",
        }
    }
}

#[derive(Debug)]
pub(crate) enum OrgArgs {
    List {
        user: Option<String>,
        paging: PageArgs,
    },
    View {
        org: String,
    },
    Create {
        input: OsString,
    },
    Edit {
        org: String,
        input: OsString,
    },
    Delete {
        org: String,
    },
    Members {
        org: String,
        paging: PageArgs,
    },
    Repos {
        org: String,
        paging: PageArgs,
    },
    RemoveMember {
        org: String,
        user: String,
    },
}

impl OrgArgs {
    pub(crate) const fn action(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::View { .. } => "view",
            Self::Create { .. } => "create",
            Self::Edit { .. } => "edit",
            Self::Delete { .. } => "delete",
            Self::Members { .. } => "members",
            Self::Repos { .. } => "repos",
            Self::RemoveMember { .. } => "remove-member",
        }
    }
}

#[derive(Debug)]
pub(crate) enum TeamArgs {
    List { org: String, paging: PageArgs },
    View { id: u64 },
    Create { org: String, input: OsString },
    Edit { id: u64, input: OsString },
    Delete { id: u64 },
    Members { id: u64, paging: PageArgs },
    Repos { id: u64, paging: PageArgs },
    AddMember { id: u64, user: String },
    RemoveMember { id: u64, user: String },
    AddRepo { id: u64, repo: String },
    RemoveRepo { id: u64, repo: String },
}

impl TeamArgs {
    pub(crate) const fn action(&self) -> &'static str {
        match self {
            Self::List { .. } => "list",
            Self::View { .. } => "view",
            Self::Create { .. } => "create",
            Self::Edit { .. } => "edit",
            Self::Delete { .. } => "delete",
            Self::Members { .. } => "members",
            Self::Repos { .. } => "repos",
            Self::AddMember { .. } => "add-member",
            Self::RemoveMember { .. } => "remove-member",
            Self::AddRepo { .. } => "add-repo",
            Self::RemoveRepo { .. } => "remove-repo",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum GitCredentialOperation {
    Get,
    Store,
    Erase,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ListState {
    Open,
    Closed,
    All,
}

impl ListState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

#[derive(Debug)]
pub(crate) struct PageArgs {
    pub(crate) page: u64,
    pub(crate) all: bool,
    pub(crate) limit: u64,
}

#[derive(Debug)]
pub(crate) enum BodySource {
    Text(String),
    File(OsString),
}

#[derive(Debug)]
pub(crate) enum IssueArgs {
    Comments {
        number: u64,
        paging: PageArgs,
    },
    Edit {
        number: u64,
        edit: EditArgs,
    },
    List {
        state: ListState,
        paging: PageArgs,
        filters: IssueFilters,
    },
    View {
        number: u64,
    },
    Create {
        title: String,
        body: Option<BodySource>,
        metadata: MetadataArgs,
        board: Option<BoardSelector>,
    },
    Comment {
        number: u64,
        body: BodySource,
    },
    Close {
        number: u64,
    },
    Reopen {
        number: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MergeStyle {
    Merge,
    Rebase,
    RebaseMerge,
    Squash,
}

impl MergeStyle {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::RebaseMerge => "rebase-merge",
            Self::Squash => "squash",
        }
    }
}

#[derive(Debug)]
pub(crate) enum PullArgs {
    Comments {
        number: u64,
        paging: PageArgs,
    },
    Reviews {
        number: u64,
        paging: PageArgs,
    },
    Files {
        number: u64,
        paging: PageArgs,
    },
    ReviewComments {
        number: u64,
        review_id: u64,
        paging: PageArgs,
    },
    Edit {
        number: u64,
        edit: EditArgs,
    },
    RequestReview {
        number: u64,
        reviewers: Vec<String>,
        teams: Vec<String>,
    },
    List {
        state: ListState,
        paging: PageArgs,
        filters: PullFilters,
    },
    View {
        number: u64,
    },
    Create {
        head: String,
        title: String,
        base: Option<String>,
        body: Option<BodySource>,
        draft: bool,
        metadata: MetadataArgs,
    },
    Diff {
        number: u64,
    },
    Checks {
        number: u64,
    },
    Comment {
        number: u64,
        body: BodySource,
    },
    Review {
        number: u64,
        event: ReviewEvent,
        body: Option<BodySource>,
        comments_file: Option<OsString>,
        commit: Option<String>,
    },
    Merge {
        number: u64,
        style: MergeStyle,
        title: Option<String>,
        message: Option<String>,
        delete_branch: bool,
        match_head: Option<String>,
        auto: bool,
    },
    Close {
        number: u64,
    },
    Reopen {
        number: u64,
    },
}

#[derive(Debug)]
pub(crate) enum RunArgs {
    List {
        paging: PageArgs,
        filters: RunFilters,
    },
    View {
        id: u64,
    },
    Watch {
        id: u64,
        poll: u64,
        wait: u64,
    },
}

#[derive(Debug)]
pub(crate) enum ReleaseArgs {
    List {
        paging: PageArgs,
    },
    View {
        tag: String,
    },
    Create {
        tag: String,
        title: String,
        body: Option<BodySource>,
        target: Option<String>,
        draft: bool,
        prerelease: bool,
    },
    Upload {
        id: u64,
        path: OsString,
        name: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) enum LabelArgs {
    List {
        paging: PageArgs,
    },
    Create {
        name: String,
        color: String,
        description: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) enum MilestoneArgs {
    List {
        state: ListState,
        paging: PageArgs,
    },
    Create {
        title: String,
        description: Option<String>,
        due: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) enum BranchArgs {
    List { paging: PageArgs },
    Delete { name: String },
}

#[derive(Debug)]
pub(crate) struct WorkflowArgs {
    pub(crate) file: String,
    pub(crate) reference: String,
    pub(crate) fields: Vec<(String, String)>,
}

#[derive(Debug)]
pub(crate) struct ApiArgs {
    pub(crate) path: String,
    pub(crate) method: Option<String>,
    pub(crate) input: Option<OsString>,
    pub(crate) paginate: bool,
}

#[derive(Debug, Default)]
pub(crate) struct IssueFilters {
    pub(crate) labels: Vec<String>,
    pub(crate) author: Option<String>,
    pub(crate) assignee: Option<String>,
    pub(crate) search: Option<String>,
    pub(crate) milestone: Option<String>,
    pub(crate) since: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) sort: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct PullFilters {
    pub(crate) labels: Vec<String>,
    pub(crate) author: Option<String>,
    pub(crate) milestone: Option<String>,
    pub(crate) sort: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RunFilters {
    pub(crate) status: Option<String>,
    pub(crate) event: Option<String>,
    pub(crate) reference: Option<String>,
    pub(crate) head_sha: Option<String>,
    pub(crate) workflow: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct MetadataArgs {
    pub(crate) labels: Vec<String>,
    pub(crate) label_ids: Vec<u64>,
    pub(crate) assignees: Vec<String>,
    pub(crate) milestone: Option<String>,
    pub(crate) milestone_id: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct EditArgs {
    pub(crate) title: Option<String>,
    pub(crate) body: Option<BodySource>,
    pub(crate) base: Option<String>,
    pub(crate) add_labels: Vec<String>,
    pub(crate) remove_labels: Vec<String>,
    pub(crate) add_label_ids: Vec<u64>,
    pub(crate) remove_label_ids: Vec<u64>,
    pub(crate) add_assignees: Vec<String>,
    pub(crate) remove_assignees: Vec<String>,
    pub(crate) milestone: Option<String>,
    pub(crate) milestone_id: Option<u64>,
    pub(crate) clear_milestone: bool,
}

#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each CLI switch needs independent duplicate and command-scope checks"
)]
struct Specific {
    with_token: bool,
    web: bool,
    no_browser: bool,
    client_id: Option<String>,
    method: Option<String>,
    input: Option<OsString>,
    paginate: bool,
    state: Option<String>,
    page: Option<String>,
    all: bool,
    limit: Option<String>,
    title: Option<String>,
    body: Option<String>,
    body_file: Option<OsString>,
    head: Option<String>,
    base: Option<String>,
    draft: bool,
    event: Option<String>,
    style: Option<String>,
    message: Option<String>,
    delete_branch: bool,
    poll: Option<String>,
    wait: Option<String>,
    tag: Option<String>,
    target: Option<String>,
    name: Option<String>,
    color: Option<String>,
    description: Option<String>,
    due: Option<String>,
    prerelease: bool,
    reference: Option<String>,
    fields: Vec<String>,
    metadata: MetadataArgs,
    edit: EditArgs,
    filters: IssueFilters,
    run_filters: RunFilters,
    reviewers: Vec<String>,
    teams: Vec<String>,
    comments_file: Option<OsString>,
    commit: Option<String>,
    match_head: Option<String>,
    auto: bool,
    board: Option<String>,
    board_id: Option<u64>,
}

#[allow(
    clippy::too_many_lines,
    reason = "one flat parser keeps common flags valid at every command position"
)]
pub(crate) fn parse() -> Result<Args, Error> {
    parse_parser(lexopt::Parser::from_env())
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "offline integration tests exercise the private parser"
)]
pub(crate) fn parse_from(args: impl IntoIterator<Item = OsString>) -> Result<Args, Error> {
    parse_parser(lexopt::Parser::from_args(args))
}

#[allow(clippy::too_many_lines, reason = "flat command parser")]
fn parse_parser(mut parser: lexopt::Parser) -> Result<Args, Error> {
    let (mut host, mut repo) = (None, None);
    let (mut json, mut dry_run, mut yes, mut help, mut version) =
        (false, false, false, false, false);
    let mut human = false;
    let mut fields = None;
    let mut specific = Specific::default();
    let mut words = Vec::new();
    while let Some(argument) = parser
        .next()
        .map_err(|error| Error::usage(error.to_string()))?
    {
        match argument {
            Long("host") => set_once(&mut host, string_value(&mut parser, "--host")?, "--host")?,
            Short('R') => set_once(&mut repo, string_value(&mut parser, "-R")?, "-R")?,
            Long("human") => set_switch(&mut human, "--human")?,
            Long("fields") => set_once(
                &mut fields,
                string_value(&mut parser, "--fields")?,
                "--fields",
            )?,
            Long("json") => set_switch(&mut json, "--json")?,
            Long("dry-run") => set_switch(&mut dry_run, "--dry-run")?,
            Long("yes") => set_switch(&mut yes, "--yes")?,
            Long("with-token") => set_switch(&mut specific.with_token, "--with-token")?,
            Long("web") => set_switch(&mut specific.web, "--web")?,
            Long("no-browser") => set_switch(&mut specific.no_browser, "--no-browser")?,
            Long("client-id") => set_once(
                &mut specific.client_id,
                string_value(&mut parser, "--client-id")?,
                "--client-id",
            )?,
            Short('X') => set_once(&mut specific.method, string_value(&mut parser, "-X")?, "-X")?,
            Long("input") => set_once(&mut specific.input, os_value(&mut parser)?, "--input")?,
            Long("paginate") => set_switch(&mut specific.paginate, "--paginate")?,
            Long("state") => set_once(
                &mut specific.state,
                string_value(&mut parser, "--state")?,
                "--state",
            )?,
            Long("page") => set_once(
                &mut specific.page,
                string_value(&mut parser, "--page")?,
                "--page",
            )?,
            Long("all") => set_switch(&mut specific.all, "--all")?,
            Long("limit") => set_once(
                &mut specific.limit,
                string_value(&mut parser, "--limit")?,
                "--limit",
            )?,
            Long("title") => set_once(
                &mut specific.title,
                string_value(&mut parser, "--title")?,
                "--title",
            )?,
            Long("body") => set_once(
                &mut specific.body,
                string_value(&mut parser, "--body")?,
                "--body",
            )?,
            Long("body-file") => set_once(
                &mut specific.body_file,
                os_value(&mut parser)?,
                "--body-file",
            )?,
            Long("head") => set_once(
                &mut specific.head,
                string_value(&mut parser, "--head")?,
                "--head",
            )?,
            Long("base") => set_once(
                &mut specific.base,
                string_value(&mut parser, "--base")?,
                "--base",
            )?,
            Long("draft") => set_switch(&mut specific.draft, "--draft")?,
            Long("event") => set_once(
                &mut specific.event,
                string_value(&mut parser, "--event")?,
                "--event",
            )?,
            Long("style") => set_once(
                &mut specific.style,
                string_value(&mut parser, "--style")?,
                "--style",
            )?,
            Long("message") => set_once(
                &mut specific.message,
                string_value(&mut parser, "--message")?,
                "--message",
            )?,
            Long("delete-branch") => set_switch(&mut specific.delete_branch, "--delete-branch")?,
            Long("poll") => set_once(
                &mut specific.poll,
                string_value(&mut parser, "--poll")?,
                "--poll",
            )?,
            Long("wait") => set_once(
                &mut specific.wait,
                string_value(&mut parser, "--wait")?,
                "--wait",
            )?,
            Long("tag") => set_once(
                &mut specific.tag,
                string_value(&mut parser, "--tag")?,
                "--tag",
            )?,
            Long("target") => set_once(
                &mut specific.target,
                string_value(&mut parser, "--target")?,
                "--target",
            )?,
            Long("name") => set_once(
                &mut specific.name,
                string_value(&mut parser, "--name")?,
                "--name",
            )?,
            Long("color") => set_once(
                &mut specific.color,
                string_value(&mut parser, "--color")?,
                "--color",
            )?,
            Long("description") => set_once(
                &mut specific.description,
                string_value(&mut parser, "--description")?,
                "--description",
            )?,
            Long("due") => set_once(
                &mut specific.due,
                string_value(&mut parser, "--due")?,
                "--due",
            )?,
            Long("prerelease") => set_switch(&mut specific.prerelease, "--prerelease")?,
            Long("ref") => set_once(
                &mut specific.reference,
                string_value(&mut parser, "--ref")?,
                "--ref",
            )?,
            Long("field") => specific.fields.push(string_value(&mut parser, "--field")?),
            Long(
                flag @ ("label" | "assignee" | "reviewer" | "team" | "add-label" | "remove-label"
                | "add-assignee" | "remove-assignee"),
            ) => {
                let flag = flag.to_owned();
                let flag = flag.as_str();
                let value = string_value(&mut parser, flag)?;
                let value = nonempty(&value, flag)?;
                let slot = match flag {
                    "label" => &mut specific.metadata.labels,
                    "assignee" => &mut specific.metadata.assignees,
                    "reviewer" => &mut specific.reviewers,
                    "team" => &mut specific.teams,
                    "add-label" => &mut specific.edit.add_labels,
                    "remove-label" => &mut specific.edit.remove_labels,
                    "add-assignee" => &mut specific.edit.add_assignees,
                    _ => &mut specific.edit.remove_assignees,
                };
                slot.push(value);
            }
            Long(flag @ ("label-id" | "add-label-id" | "remove-label-id")) => {
                let flag = flag.to_owned();
                let flag = flag.as_str();
                let value = positive(&string_value(&mut parser, flag)?, flag)?;
                match flag {
                    "label-id" => specific.metadata.label_ids.push(value),
                    "add-label-id" => specific.edit.add_label_ids.push(value),
                    _ => specific.edit.remove_label_ids.push(value),
                }
            }
            Long("milestone-id") => set_once(
                &mut specific.metadata.milestone_id,
                positive(
                    &string_value(&mut parser, "--milestone-id")?,
                    "milestone ID",
                )?,
                "--milestone-id",
            )?,
            Long("clear-milestone") => {
                set_switch(&mut specific.edit.clear_milestone, "--clear-milestone")?;
            }
            Long("auto") => set_switch(&mut specific.auto, "--auto")?,
            Long("board") => {
                let value = string_value(&mut parser, "--board")?;
                set_once(&mut specific.board, nonempty(&value, "--board")?, "--board")?;
            }
            Long("board-id") => set_once(
                &mut specific.board_id,
                positive(&string_value(&mut parser, "--board-id")?, "board ID")?,
                "--board-id",
            )?,
            Long("comments-file") => set_once(
                &mut specific.comments_file,
                os_value(&mut parser)?,
                "--comments-file",
            )?,
            Long(
                flag @ ("author" | "search" | "milestone" | "since" | "before" | "sort" | "status"
                | "head-sha" | "workflow" | "commit" | "match-head"),
            ) => {
                let flag = flag.to_owned();
                let flag = flag.as_str();
                let value = string_value(&mut parser, flag)?;
                let value = nonempty(&value, flag)?;
                let slot = match flag {
                    "author" => &mut specific.filters.author,
                    "search" => &mut specific.filters.search,
                    "milestone" => &mut specific.metadata.milestone,
                    "since" => &mut specific.filters.since,
                    "before" => &mut specific.filters.before,
                    "sort" => &mut specific.filters.sort,
                    "status" => &mut specific.run_filters.status,
                    "head-sha" => &mut specific.run_filters.head_sha,
                    "workflow" => &mut specific.run_filters.workflow,
                    "commit" => &mut specific.commit,
                    _ => &mut specific.match_head,
                };
                set_once(slot, value, flag)?;
            }
            Short('h') | Long("help") => set_switch(&mut help, "--help")?,
            Short('V') | Long("version") => set_switch(&mut version, "--version")?,
            Value(value) => words.push(
                value
                    .into_string()
                    .map_err(|_| Error::usage("command words must be valid UTF-8"))?,
            ),
            other => return Err(Error::usage(other.unexpected().to_string())),
        }
    }
    if human && json {
        return Err(Error::usage("--human conflicts with --json"));
    }
    if fields.is_some() && !json {
        return Err(Error::usage("--fields requires --json"));
    }
    let fields = fields.map_or(Ok(Vec::new()), |value: String| {
        value
            .split(',')
            .map(|field| nonempty(field.trim(), "field"))
            .collect::<Result<Vec<_>, _>>()
    })?;
    let command = if help || words.first().is_some_and(|word| word == "help") {
        let path = if help {
            command_help_path(&words)?
        } else {
            validate_path(&words[1..])?
        };
        consume_help_flags(&path, &mut specific)?;
        Command::Help { path }
    } else if words.first().is_some_and(|word| word == "schema") {
        if !json {
            return Err(Error::usage("schema requires --json"));
        }
        Command::Schema {
            path: validate_path(&words[1..])?,
        }
    } else if version {
        Command::Version
    } else {
        parse_command(&words, &mut specific)?
    };
    let human_scope = matches!(
        &command,
        Command::Issue(IssueArgs::View { .. }) | Command::Pull(PullArgs::View { .. })
    ) || matches!(&command, Command::Help { path } if path.len() == 2 && (path[0] == "issue" || path[0] == "pr") && path[1] == "view");
    if human && !human_scope {
        return Err(Error::usage(
            "--human is only valid for issue view and pr view",
        ));
    }
    specific.ensure_empty()?;
    Ok(Args {
        host,
        repo,
        json,
        fields,
        human,
        dry_run,
        yes,
        command,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "one exhaustive match keeps each public command grammar explicit"
)]
fn parse_command(words: &[String], o: &mut Specific) -> Result<Command, Error> {
    match words {
        [a, b] if a == "mcp" && b == "serve" => Ok(Command::McpServe),
        [a, b] if a == "mcp" && b == "config" => Ok(Command::McpConfig),
        [a, ..] if a == "org" => parse_org(&words[1..], o).map(Command::Org),
        [a, ..] if a == "team" => parse_team(&words[1..], o).map(Command::Team),
        [a, b] if a == "webhook" && b == "list" => Ok(Command::Webhook(WebhookArgs::List {
            paging: o.take_paging()?,
        })),
        [a, b] if a == "webhook" && b == "create" => Ok(Command::Webhook(WebhookArgs::Create {
            input: o
                .input
                .take()
                .ok_or_else(|| Error::usage("webhook create requires --input PATH|-"))?,
        })),
        [a, b, n] if a == "webhook" => {
            let id = positive(n, "webhook ID")?;
            Ok(Command::Webhook(match b.as_str() {
                "view" => WebhookArgs::View { id },
                "edit" => WebhookArgs::Edit {
                    id,
                    input: o
                        .input
                        .take()
                        .ok_or_else(|| Error::usage("webhook edit requires --input PATH|-"))?,
                },
                "delete" => WebhookArgs::Delete { id },
                "test" => WebhookArgs::Test {
                    id,
                    reference: o.reference.take(),
                },
                _ => {
                    return Err(Error::usage(
                        "unknown webhook command; run fjx webhook --help",
                    ));
                }
            }))
        }
        [a, b] if a == "board" && b == "list" => Ok(Command::Board(BoardArgs::List {
            state: o.take_state()?,
            paging: o.take_paging()?,
        })),
        [a, b] if a == "board" && b == "view" => Ok(Command::Board(BoardArgs::View {
            board: o.take_board()?,
        })),
        [a, b, n, column] if a == "board" && b == "move" => Ok(Command::Board(BoardArgs::Move {
            number: positive(n, "issue number")?,
            column: nonempty(column, "column title")?,
            board: o.take_board()?,
        })),
        [a, b, n] if a == "board" && b == "remove" => Ok(Command::Board(BoardArgs::Remove {
            number: positive(n, "issue number")?,
            board: o.take_board()?,
        })),
        [a, b, n] if a == "issue" && b == "comments" => Ok(Command::Issue(IssueArgs::Comments {
            number: positive(n, "issue number")?,
            paging: o.take_paging()?,
        })),
        [a, b, n] if a == "issue" && b == "edit" => Ok(Command::Issue(IssueArgs::Edit {
            number: positive(n, "issue number")?,
            edit: o.take_edit(false)?,
        })),
        [a, b, n] if a == "pr" && matches!(b.as_str(), "comments" | "reviews" | "files") => {
            let number = positive(n, "pull request number")?;
            let paging = o.take_paging()?;
            Ok(Command::Pull(match b.as_str() {
                "comments" => PullArgs::Comments { number, paging },
                "reviews" => PullArgs::Reviews { number, paging },
                _ => PullArgs::Files { number, paging },
            }))
        }
        [a, b, n, id] if a == "pr" && b == "review-comments" => {
            Ok(Command::Pull(PullArgs::ReviewComments {
                number: positive(n, "pull request number")?,
                review_id: positive(id, "review ID")?,
                paging: o.take_paging()?,
            }))
        }
        [a, b, n] if a == "pr" && b == "edit" => Ok(Command::Pull(PullArgs::Edit {
            number: positive(n, "pull request number")?,
            edit: o.take_edit(true)?,
        })),
        [a, b, n] if a == "pr" && b == "request-review" => {
            if o.reviewers.is_empty() && o.teams.is_empty() {
                return Err(Error::usage("request-review requires --reviewer or --team"));
            }
            Ok(Command::Pull(PullArgs::RequestReview {
                number: positive(n, "pull request number")?,
                reviewers: std::mem::take(&mut o.reviewers),
                teams: std::mem::take(&mut o.teams),
            }))
        }
        [a, b] if a == "auth" && b == "login" => Ok(Command::AuthLogin {
            options: o.take_login()?,
        }),
        [a, b] if a == "auth" && b == "status" => Ok(Command::AuthStatus),
        [a, b] if a == "auth" && b == "logout" => Ok(Command::AuthLogout),
        [a, b] if a == "auth" && b == "setup-git" => Ok(Command::AuthSetupGit),
        [a, b, operation] if a == "auth" && b == "git-credential" => {
            let operation = match operation.as_str() {
                "get" => GitCredentialOperation::Get,
                "store" => GitCredentialOperation::Store,
                "erase" => GitCredentialOperation::Erase,
                _ => return Err(Error::usage("invalid Git credential operation")),
            };
            Ok(Command::AuthGitCredential { operation })
        }
        [a, b, alias]
            if a == "auth" && matches!(b.as_str(), "add-ssh-alias" | "remove-ssh-alias") =>
        {
            Ok(Command::AuthSshAlias {
                alias: nonempty(alias, "SSH alias")?,
                remove: b == "remove-ssh-alias",
            })
        }
        [a, b] if a == "repo" && b == "view" => Ok(Command::RepoView),
        [a, b] if a == "issue" && b == "list" => Ok(Command::Issue(IssueArgs::List {
            state: o.take_state()?,
            paging: o.take_paging()?,
            filters: o.take_issue_filters()?,
        })),
        [a, b, n] if a == "issue" && b == "view" => Ok(Command::Issue(IssueArgs::View {
            number: positive(n, "issue number")?,
        })),
        [a, b] if a == "issue" && b == "create" => Ok(Command::Issue(IssueArgs::Create {
            title: o.take_required_title()?,
            body: o.take_body(false)?,
            metadata: o.take_metadata()?,
            board: o.take_board()?,
        })),
        [a, b, n] if a == "issue" && b == "comment" => Ok(Command::Issue(IssueArgs::Comment {
            number: positive(n, "issue number")?,
            body: o
                .take_body(true)?
                .ok_or_else(|| Error::usage("issue comment requires a body"))?,
        })),
        [a, b, n] if a == "issue" && b == "close" => Ok(Command::Issue(IssueArgs::Close {
            number: positive(n, "issue number")?,
        })),
        [a, b, n] if a == "issue" && b == "reopen" => Ok(Command::Issue(IssueArgs::Reopen {
            number: positive(n, "issue number")?,
        })),
        [a, b] if a == "pr" && b == "list" => Ok(Command::Pull(PullArgs::List {
            state: o.take_state()?,
            paging: o.take_paging()?,
            filters: o.take_pull_filters(),
        })),
        [a, b, n] if a == "pr" && b == "view" => Ok(Command::Pull(PullArgs::View {
            number: positive(n, "pull request number")?,
        })),
        [a, b] if a == "pr" && b == "create" => Ok(Command::Pull(o.take_pull_create()?)),
        [a, b, n] if a == "pr" && b == "diff" => Ok(Command::Pull(PullArgs::Diff {
            number: positive(n, "pull request number")?,
        })),
        [a, b, n] if a == "pr" && b == "checks" => Ok(Command::Pull(PullArgs::Checks {
            number: positive(n, "pull request number")?,
        })),
        [a, b, n] if a == "pr" && b == "comment" => Ok(Command::Pull(PullArgs::Comment {
            number: positive(n, "pull request number")?,
            body: o
                .take_body(true)?
                .ok_or_else(|| Error::usage("pr comment requires a body"))?,
        })),
        [a, b, n] if a == "pr" && b == "review" => Ok(Command::Pull(
            o.take_review(positive(n, "pull request number")?)?,
        )),
        [a, b, n] if a == "pr" && b == "merge" => Ok(Command::Pull(
            o.take_merge(positive(n, "pull request number")?)?,
        )),
        [a, b, n] if a == "pr" && b == "close" => Ok(Command::Pull(PullArgs::Close {
            number: positive(n, "pull request number")?,
        })),
        [a, b, n] if a == "pr" && b == "reopen" => Ok(Command::Pull(PullArgs::Reopen {
            number: positive(n, "pull request number")?,
        })),
        [a, b] if a == "run" && b == "list" => Ok(Command::Run(RunArgs::List {
            paging: o.take_paging()?,
            filters: o.take_run_filters(),
        })),
        [a, b, id] if a == "run" && b == "view" => Ok(Command::Run(RunArgs::View {
            id: positive(id, "run ID")?,
        })),
        [a, b, id] if a == "run" && b == "watch" => Ok(Command::Run(RunArgs::Watch {
            id: positive(id, "run ID")?,
            poll: o.take_seconds(true)?,
            wait: o.take_seconds(false)?,
        })),
        [a, b] if a == "release" && b == "list" => Ok(Command::Release(ReleaseArgs::List {
            paging: o.take_paging()?,
        })),
        [a, b, tag] if a == "release" && b == "view" => Ok(Command::Release(ReleaseArgs::View {
            tag: nonempty(tag, "release tag")?,
        })),
        [a, b] if a == "release" && b == "create" => Ok(Command::Release(o.take_release_create()?)),
        [a, b, id, path] if a == "release" && b == "upload" => {
            Ok(Command::Release(ReleaseArgs::Upload {
                id: positive(id, "release ID")?,
                path: OsString::from(path),
                name: o.take_optional_nonempty_name()?,
            }))
        }
        [a, b] if a == "label" && b == "list" => Ok(Command::Label(LabelArgs::List {
            paging: o.take_paging()?,
        })),
        [a, b] if a == "label" && b == "create" => Ok(Command::Label(o.take_label_create()?)),
        [a, b] if a == "milestone" && b == "list" => Ok(Command::Milestone(MilestoneArgs::List {
            state: o.take_state()?,
            paging: o.take_paging()?,
        })),
        [a, b] if a == "milestone" && b == "create" => {
            Ok(Command::Milestone(o.take_milestone_create()?))
        }
        [a, b] if a == "branch" && b == "list" => Ok(Command::Branch(BranchArgs::List {
            paging: o.take_paging()?,
        })),
        [a, b, name] if a == "branch" && b == "delete" => Ok(Command::Branch(BranchArgs::Delete {
            name: nonempty(name, "branch name")?,
        })),
        [a, b, file] if a == "workflow" && b == "dispatch" => {
            Ok(Command::Workflow(o.take_workflow(file)?))
        }
        [api, path] if api == "api" => Ok(Command::Api(ApiArgs {
            path: path.clone(),
            method: o.method.take(),
            input: o.input.take(),
            paginate: std::mem::take(&mut o.paginate),
        })),
        _ => {
            let mut hint = String::from("fjx");
            if let Some(group) = words
                .first()
                .filter(|group| !subcommands(group).is_empty() || *group == "api")
            {
                hint.push(' ');
                hint.push_str(group);
                if let Some(action) = words
                    .get(1)
                    .filter(|action| subcommands(group).contains(&action.as_str()))
                {
                    hint.push(' ');
                    hint.push_str(action);
                }
            }
            Err(Error::usage(format!(
                "unknown or incomplete command; run {hint} --help"
            )))
        }
    }
}

fn input_file(o: &mut Specific) -> Result<OsString, Error> {
    o.input
        .take()
        .ok_or_else(|| Error::usage("create/edit requires --input PATH|-"))
}

fn account(value: &str) -> Result<String, Error> {
    // Reject path metacharacters and dot segments before host/auth resolution.
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::usage(
            "organization/user must be a name containing letters, digits, -, _ or .",
        ));
    }
    Ok(value.to_owned())
}

fn parse_org(words: &[String], o: &mut Specific) -> Result<OrgArgs, Error> {
    match words {
        [a] if a == "list" => Ok(OrgArgs::List {
            user: None,
            paging: o.take_paging()?,
        }),
        [a, user] if a == "list" => Ok(OrgArgs::List {
            user: Some(account(user)?),
            paging: o.take_paging()?,
        }),
        [a] if a == "create" => Ok(OrgArgs::Create {
            input: input_file(o)?,
        }),
        [a, org] => {
            let org = account(org)?;
            match a.as_str() {
                "view" => Ok(OrgArgs::View { org }),
                "edit" => Ok(OrgArgs::Edit {
                    org,
                    input: input_file(o)?,
                }),
                "delete" => Ok(OrgArgs::Delete { org }),
                "members" => Ok(OrgArgs::Members {
                    org,
                    paging: o.take_paging()?,
                }),
                "repos" => Ok(OrgArgs::Repos {
                    org,
                    paging: o.take_paging()?,
                }),
                _ => Err(Error::usage(
                    "unknown organization command; run fjx org --help",
                )),
            }
        }
        [a, org, user] if a == "remove-member" => Ok(OrgArgs::RemoveMember {
            org: account(org)?,
            user: account(user)?,
        }),
        _ => Err(Error::usage(
            "unknown or incomplete organization command; run fjx org --help",
        )),
    }
}

fn parse_team(words: &[String], o: &mut Specific) -> Result<TeamArgs, Error> {
    match words {
        [a, org] if a == "list" => Ok(TeamArgs::List {
            org: account(org)?,
            paging: o.take_paging()?,
        }),
        [a, org] if a == "create" => Ok(TeamArgs::Create {
            org: account(org)?,
            input: input_file(o)?,
        }),
        [a, id] => {
            let id = positive(id, "team ID")?;
            match a.as_str() {
                "view" => Ok(TeamArgs::View { id }),
                "edit" => Ok(TeamArgs::Edit {
                    id,
                    input: input_file(o)?,
                }),
                "delete" => Ok(TeamArgs::Delete { id }),
                "members" => Ok(TeamArgs::Members {
                    id,
                    paging: o.take_paging()?,
                }),
                "repos" => Ok(TeamArgs::Repos {
                    id,
                    paging: o.take_paging()?,
                }),
                _ => Err(Error::usage("unknown team command; run fjx team --help")),
            }
        }
        [a, id, value] => {
            let id = positive(id, "team ID")?;
            match a.as_str() {
                "add-member" => Ok(TeamArgs::AddMember {
                    id,
                    user: account(value)?,
                }),
                "remove-member" => Ok(TeamArgs::RemoveMember {
                    id,
                    user: account(value)?,
                }),
                "add-repo" | "remove-repo" => {
                    let (owner, name) = value
                        .split_once('/')
                        .ok_or_else(|| Error::usage("repository must be OWNER/REPO"))?;
                    account(owner)?;
                    account(name)?;
                    Ok(if a == "add-repo" {
                        TeamArgs::AddRepo {
                            id,
                            repo: value.clone(),
                        }
                    } else {
                        TeamArgs::RemoveRepo {
                            id,
                            repo: value.clone(),
                        }
                    })
                }
                _ => Err(Error::usage("unknown team command; run fjx team --help")),
            }
        }
        _ => Err(Error::usage(
            "unknown or incomplete team command; run fjx team --help",
        )),
    }
}

impl Specific {
    fn take_metadata(&mut self) -> Result<MetadataArgs, Error> {
        if self.metadata.milestone.is_some() && self.metadata.milestone_id.is_some() {
            return Err(Error::usage("--milestone conflicts with --milestone-id"));
        }
        Ok(std::mem::take(&mut self.metadata))
    }

    fn take_board(&mut self) -> Result<Option<BoardSelector>, Error> {
        match (self.board.take(), self.board_id.take()) {
            (Some(_), Some(_)) => Err(Error::usage("--board conflicts with --board-id")),
            (Some(title), None) => Ok(Some(BoardSelector::Title(title))),
            (None, Some(id)) => Ok(Some(BoardSelector::Id(id))),
            (None, None) => Ok(None),
        }
    }

    fn take_issue_filters(&mut self) -> Result<IssueFilters, Error> {
        let mut filters = std::mem::take(&mut self.filters);
        filters.labels = std::mem::take(&mut self.metadata.labels);
        if self.metadata.assignees.len() > 1 {
            return Err(Error::usage("issue list accepts one --assignee"));
        }
        filters.assignee = self.metadata.assignees.pop();
        filters.milestone = self.metadata.milestone.take();
        for (date, name) in [(&filters.since, "--since"), (&filters.before, "--before")] {
            if date
                .as_deref()
                .is_some_and(|value| !is_rfc3339_date_time(value))
            {
                return Err(Error::usage(format!("{name} must be an RFC3339 date-time")));
            }
        }
        Ok(filters)
    }

    fn take_pull_filters(&mut self) -> PullFilters {
        PullFilters {
            labels: std::mem::take(&mut self.metadata.labels),
            author: self.filters.author.take(),
            milestone: self.metadata.milestone.take(),
            sort: self.filters.sort.take(),
        }
    }

    fn take_run_filters(&mut self) -> RunFilters {
        let mut filters = std::mem::take(&mut self.run_filters);
        filters.event = self.event.take();
        filters.reference = self.reference.take();
        filters
    }

    fn take_edit(&mut self, pull: bool) -> Result<EditArgs, Error> {
        let metadata = self.take_metadata()?;
        if !metadata.labels.is_empty()
            || !metadata.label_ids.is_empty()
            || !metadata.assignees.is_empty()
        {
            return Err(Error::usage("edit requires --add/--remove metadata flags"));
        }
        let mut edit = std::mem::take(&mut self.edit);
        edit.title = take_optional_nonempty(&mut self.title, "--title")?;
        edit.body = self.take_body(false)?;
        if pull {
            edit.base = take_optional_nonempty(&mut self.base, "--base")?;
        }
        edit.milestone = metadata.milestone;
        edit.milestone_id = metadata.milestone_id;
        if edit.clear_milestone && (edit.milestone.is_some() || edit.milestone_id.is_some()) {
            return Err(Error::usage(
                "--clear-milestone conflicts with milestone selection",
            ));
        }
        if overlaps(&edit.add_labels, &edit.remove_labels)
            || overlaps(&edit.add_label_ids, &edit.remove_label_ids)
            || overlaps(&edit.add_assignees, &edit.remove_assignees)
        {
            return Err(Error::usage("cannot add and remove the same metadata"));
        }
        if edit.title.is_none()
            && edit.body.is_none()
            && edit.base.is_none()
            && edit.add_labels.is_empty()
            && edit.remove_labels.is_empty()
            && edit.add_label_ids.is_empty()
            && edit.remove_label_ids.is_empty()
            && edit.add_assignees.is_empty()
            && edit.remove_assignees.is_empty()
            && edit.milestone.is_none()
            && edit.milestone_id.is_none()
            && !edit.clear_milestone
        {
            return Err(Error::usage("edit requires at least one change"));
        }
        Ok(edit)
    }

    fn take_login(&mut self) -> Result<LoginArgs, Error> {
        if self.with_token && (self.web || self.no_browser || self.client_id.is_some()) {
            return Err(Error::usage(
                "--with-token cannot be combined with browser login flags",
            ));
        }
        Ok(LoginArgs {
            with_token: std::mem::take(&mut self.with_token),
            web: std::mem::take(&mut self.web),
            no_browser: std::mem::take(&mut self.no_browser),
            client_id: take_optional_nonempty(&mut self.client_id, "--client-id")?,
        })
    }

    fn take_state(&mut self) -> Result<ListState, Error> {
        match self.state.take().as_deref() {
            None | Some("open") => Ok(ListState::Open),
            Some("closed") => Ok(ListState::Closed),
            Some("all") => Ok(ListState::All),
            Some(_) => Err(Error::usage("--state must be open, closed, or all")),
        }
    }

    fn take_paging(&mut self) -> Result<PageArgs, Error> {
        if self.all && self.page.is_some() {
            return Err(Error::usage("--page and --all cannot be used together"));
        }
        let page = self.page.take().map_or(Ok(1), |v| positive(&v, "page"))?;
        let limit = self
            .limit
            .take()
            .map_or(Ok(30), |v| positive(&v, "limit"))?;
        if limit > 50 {
            return Err(Error::usage("limit must be between 1 and 50"));
        }
        Ok(PageArgs {
            page,
            all: std::mem::take(&mut self.all),
            limit,
        })
    }

    fn take_body(&mut self, required: bool) -> Result<Option<BodySource>, Error> {
        match (self.body.take(), self.body_file.take()) {
            (Some(_), Some(_)) => Err(Error::usage(
                "--body and --body-file cannot be used together",
            )),
            (Some(value), None) => Ok(Some(BodySource::Text(value))),
            (None, Some(value)) => Ok(Some(BodySource::File(value))),
            (None, None) if required => Err(Error::usage("--body or --body-file is required")),
            (None, None) => Ok(None),
        }
    }

    fn take_required_title(&mut self) -> Result<String, Error> {
        self.title
            .take()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::usage("--title is required and must not be empty"))
    }

    fn take_pull_create(&mut self) -> Result<PullArgs, Error> {
        let head = self
            .head
            .take()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::usage("--head is required and must not be empty"))?;
        let title = self.take_required_title()?;
        let base = self
            .base
            .take()
            .map(|v| {
                if v.is_empty() {
                    Err(Error::usage("--base must not be empty"))
                } else {
                    Ok(v)
                }
            })
            .transpose()?;
        Ok(PullArgs::Create {
            head,
            title,
            base,
            body: self.take_body(false)?,
            draft: std::mem::take(&mut self.draft),
            metadata: self.take_metadata()?,
        })
    }

    fn take_review(&mut self, number: u64) -> Result<PullArgs, Error> {
        let event = match self.event.take().as_deref() {
            Some("approve") => ReviewEvent::Approve,
            Some("request-changes") => ReviewEvent::RequestChanges,
            Some("comment") => ReviewEvent::Comment,
            Some(_) => {
                return Err(Error::usage(
                    "--event must be approve, request-changes, or comment",
                ));
            }
            None => return Err(Error::usage("pr review requires --event")),
        };
        let body = self.take_body(false)?;
        if self.comments_file.is_some() && self.commit.is_none() {
            return Err(Error::usage("--comments-file requires --commit"));
        }
        if !matches!(event, ReviewEvent::Approve) && body.is_none() && self.comments_file.is_none()
        {
            return Err(Error::usage(
                "review body is required for request-changes and comment",
            ));
        }
        Ok(PullArgs::Review {
            number,
            event,
            body,
            comments_file: self.comments_file.take(),
            commit: self.commit.take(),
        })
    }

    fn take_merge(&mut self, number: u64) -> Result<PullArgs, Error> {
        let style = match self.style.take().as_deref() {
            None | Some("merge") => MergeStyle::Merge,
            Some("rebase") => MergeStyle::Rebase,
            Some("rebase-merge") => MergeStyle::RebaseMerge,
            Some("squash") => MergeStyle::Squash,
            Some(_) => {
                return Err(Error::usage(
                    "--style must be merge, rebase, rebase-merge, or squash",
                ));
            }
        };
        Ok(PullArgs::Merge {
            number,
            style,
            title: self.title.take(),
            message: self.message.take(),
            delete_branch: std::mem::take(&mut self.delete_branch),
            match_head: self.match_head.take(),
            auto: std::mem::take(&mut self.auto),
        })
    }

    fn take_seconds(&mut self, poll: bool) -> Result<u64, Error> {
        let slot = if poll { &mut self.poll } else { &mut self.wait };
        slot.take().map_or(Ok(if poll { 5 } else { 600 }), |v| {
            positive(&v, if poll { "poll seconds" } else { "wait seconds" })
        })
    }

    fn take_release_create(&mut self) -> Result<ReleaseArgs, Error> {
        let tag = take_required_nonempty(&mut self.tag, "--tag")?;
        let title = self.take_required_title()?;
        let target = take_optional_nonempty(&mut self.target, "--target")?;
        Ok(ReleaseArgs::Create {
            tag,
            title,
            body: self.take_body(false)?,
            target,
            draft: std::mem::take(&mut self.draft),
            prerelease: std::mem::take(&mut self.prerelease),
        })
    }

    fn take_optional_nonempty_name(&mut self) -> Result<Option<String>, Error> {
        take_optional_nonempty(&mut self.name, "--name")
    }

    fn take_label_create(&mut self) -> Result<LabelArgs, Error> {
        let name = take_required_nonempty(&mut self.name, "--name")?;
        let color = take_required_nonempty(&mut self.color, "--color")?;
        let digits = color.strip_prefix('#').unwrap_or(&color);
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::usage(
                "--color must be a six-digit hexadecimal color",
            ));
        }
        let description = take_optional_nonempty(&mut self.description, "--description")?;
        Ok(LabelArgs::Create {
            name,
            color,
            description,
        })
    }

    fn take_milestone_create(&mut self) -> Result<MilestoneArgs, Error> {
        let due = take_optional_nonempty(&mut self.due, "--due")?;
        if due
            .as_deref()
            .is_some_and(|value| !is_rfc3339_date_time(value))
        {
            return Err(Error::usage("--due must be an RFC3339 date-time"));
        }
        Ok(MilestoneArgs::Create {
            title: self.take_required_title()?,
            description: take_optional_nonempty(&mut self.description, "--description")?,
            due,
        })
    }

    fn take_workflow(&mut self, file: &str) -> Result<WorkflowArgs, Error> {
        let file = nonempty(file, "workflow file")?;
        let reference = take_required_nonempty(&mut self.reference, "--ref")?;
        let mut fields = Vec::with_capacity(self.fields.len());
        for field in std::mem::take(&mut self.fields) {
            let (key, value) = field
                .split_once('=')
                .ok_or_else(|| Error::usage("--field must be KEY=VALUE"))?;
            if key.is_empty() {
                return Err(Error::usage("--field key must not be empty"));
            }
            if fields.iter().any(|(existing, _)| existing == key) {
                return Err(Error::usage(format!("duplicate workflow field {key:?}")));
            }
            fields.push((key.to_owned(), value.to_owned()));
        }
        Ok(WorkflowArgs {
            file,
            reference,
            fields,
        })
    }

    fn ensure_empty(&self) -> Result<(), Error> {
        let unused = [
            (!self.metadata.labels.is_empty(), "--label"),
            (!self.metadata.label_ids.is_empty(), "--label-id"),
            (!self.metadata.assignees.is_empty(), "--assignee"),
            (self.metadata.milestone.is_some(), "--milestone"),
            (self.metadata.milestone_id.is_some(), "--milestone-id"),
            (!self.edit.add_labels.is_empty(), "--add-label"),
            (!self.edit.remove_labels.is_empty(), "--remove-label"),
            (!self.edit.add_label_ids.is_empty(), "--add-label-id"),
            (!self.edit.remove_label_ids.is_empty(), "--remove-label-id"),
            (!self.edit.add_assignees.is_empty(), "--add-assignee"),
            (!self.edit.remove_assignees.is_empty(), "--remove-assignee"),
            (self.edit.clear_milestone, "--clear-milestone"),
            (self.filters.author.is_some(), "--author"),
            (self.filters.search.is_some(), "--search"),
            (self.filters.since.is_some(), "--since"),
            (self.filters.before.is_some(), "--before"),
            (self.filters.sort.is_some(), "--sort"),
            (self.run_filters.status.is_some(), "--status"),
            (self.run_filters.head_sha.is_some(), "--head-sha"),
            (self.run_filters.workflow.is_some(), "--workflow"),
            (!self.reviewers.is_empty(), "--reviewer"),
            (!self.teams.is_empty(), "--team"),
            (self.comments_file.is_some(), "--comments-file"),
            (self.commit.is_some(), "--commit"),
            (self.match_head.is_some(), "--match-head"),
            (self.auto, "--auto"),
            (self.board.is_some(), "--board"),
            (self.board_id.is_some(), "--board-id"),
            (self.with_token, "--with-token"),
            (self.web, "--web"),
            (self.no_browser, "--no-browser"),
            (self.client_id.is_some(), "--client-id"),
            (self.method.is_some(), "-X"),
            (self.input.is_some(), "--input"),
            (self.paginate, "--paginate"),
            (self.state.is_some(), "--state"),
            (self.page.is_some(), "--page"),
            (self.all, "--all"),
            (self.limit.is_some(), "--limit"),
            (self.title.is_some(), "--title"),
            (self.body.is_some(), "--body"),
            (self.body_file.is_some(), "--body-file"),
            (self.head.is_some(), "--head"),
            (self.base.is_some(), "--base"),
            (self.draft, "--draft"),
            (self.event.is_some(), "--event"),
            (self.style.is_some(), "--style"),
            (self.message.is_some(), "--message"),
            (self.delete_branch, "--delete-branch"),
            (self.poll.is_some(), "--poll"),
            (self.wait.is_some(), "--wait"),
            (self.tag.is_some(), "--tag"),
            (self.target.is_some(), "--target"),
            (self.name.is_some(), "--name"),
            (self.color.is_some(), "--color"),
            (self.description.is_some(), "--description"),
            (self.due.is_some(), "--due"),
            (self.prerelease, "--prerelease"),
            (self.reference.is_some(), "--ref"),
            (!self.fields.is_empty(), "--field"),
        ]
        .into_iter()
        .find(|(used, _)| *used);
        if let Some((_, flag)) = unused {
            Err(Error::usage(format!(
                "{flag} is not valid for this command"
            )))
        } else {
            Ok(())
        }
    }
}

fn overlaps<T: PartialEq>(add: &[T], remove: &[T]) -> bool {
    add.iter().any(|value| remove.contains(value))
}

fn subcommands(group: &str) -> &'static [&'static str] {
    match group {
        "auth" => &[
            "login",
            "status",
            "logout",
            "setup-git",
            "git-credential",
            "add-ssh-alias",
            "remove-ssh-alias",
        ],
        "repo" => &["view"],
        "board" => &["list", "view", "move", "remove"],
        "org" => &[
            "list",
            "view",
            "create",
            "edit",
            "delete",
            "members",
            "repos",
            "remove-member",
        ],
        "team" => &[
            "list",
            "view",
            "create",
            "edit",
            "delete",
            "members",
            "repos",
            "add-member",
            "remove-member",
            "add-repo",
            "remove-repo",
        ],
        "issue" => &[
            "list", "view", "create", "comment", "comments", "edit", "close", "reopen",
        ],
        "pr" => &[
            "list",
            "view",
            "create",
            "diff",
            "checks",
            "comment",
            "comments",
            "reviews",
            "files",
            "review-comments",
            "edit",
            "request-review",
            "review",
            "merge",
            "close",
            "reopen",
        ],
        "run" => &["list", "view", "watch"],
        "release" => &["list", "view", "create", "upload"],
        "label" | "milestone" => &["list", "create"],
        "branch" => &["list", "delete"],
        "workflow" => &["dispatch"],
        "webhook" => &["list", "view", "create", "edit", "delete", "test"],
        _ => &[],
    }
}

fn validate_path(path: &[String]) -> Result<Vec<String>, Error> {
    let valid = match path {
        [] => true,
        [group] => {
            !subcommands(group).is_empty() || matches!(group.as_str(), "api" | "help" | "schema")
        }
        [group, command] => subcommands(group).contains(&command.as_str()),
        _ => false,
    };
    if valid {
        Ok(path.to_vec())
    } else {
        Err(Error::usage("unknown help/schema command path"))
    }
}

fn command_help_path(words: &[String]) -> Result<Vec<String>, Error> {
    if words
        .first()
        .is_some_and(|word| matches!(word.as_str(), "help" | "schema"))
    {
        return validate_path(&words[1..]);
    }
    let length = if words.first().is_some_and(|word| word == "api") {
        1
    } else {
        words.len().min(2)
    };
    let path = validate_path(&words[..length])?;
    let maximum = match path.as_slice() {
        [group, action] if matches!(group.as_str(), "org" | "team") => {
            if matches!(
                action.as_str(),
                "add-member" | "remove-member" | "add-repo" | "remove-repo"
            ) {
                4
            } else if group == "org" && action == "create" {
                2
            } else {
                3
            }
        }
        [group, command] if group == "board" => match command.as_str() {
            "move" => 4,
            "remove" => 3,
            _ => 2,
        },
        [group, command] if group == "pr" && command == "review-comments" => 4,
        [group, command] if group == "release" && command == "upload" => 4,
        [_, command]
            if matches!(
                command.as_str(),
                "list" | "create" | "login" | "status" | "logout" | "setup-git"
            ) =>
        {
            2
        }
        [_] if path[0] == "api" => 2,
        _ => 3,
    };
    if words.len() > maximum {
        return Err(Error::usage("too many command arguments"));
    }
    Ok(path)
}

#[allow(
    clippy::too_many_lines,
    reason = "help consumes only command-scoped options without requiring inputs"
)]
fn consume_help_flags(path: &[String], o: &mut Specific) -> Result<(), Error> {
    let group = path.first().map_or("", String::as_str);
    let command = path.get(1).map_or("", String::as_str);
    if group == "webhook" && matches!(command, "create" | "edit") {
        o.input.take();
    }
    if group == "webhook" && command == "test" {
        o.reference.take();
    }
    if matches!(group, "org" | "team") {
        if matches!(command, "create" | "edit") {
            o.input.take();
        }
        if matches!(command, "list" | "members" | "repos") {
            o.page.take();
            o.limit.take();
            o.all = false;
        }
    }
    if group == "api" {
        o.method.take();
        o.input.take();
        o.paginate = false;
    }
    if group == "auth" && command == "login" {
        o.with_token = false;
        o.web = false;
        o.no_browser = false;
        o.client_id.take();
    }
    if command == "list"
        || (matches!(group, "issue" | "pr")
            && matches!(
                command,
                "comments" | "reviews" | "files" | "review-comments"
            ))
    {
        o.page.take();
        o.limit.take();
        o.all = false;
    }
    if command == "list" && matches!(group, "issue" | "pr" | "milestone" | "board") {
        o.state.take();
    }
    if (group == "board" && command != "list") || (group == "issue" && command == "create") {
        o.board.take();
        o.board_id.take();
    }
    if command == "list" && matches!(group, "issue" | "pr") {
        o.metadata.labels.clear();
        o.metadata.milestone.take();
        o.filters.author.take();
        o.filters.sort.take();
        if group == "issue" {
            o.metadata.assignees.clear();
            o.filters.search.take();
            o.filters.since.take();
            o.filters.before.take();
        }
    }
    if group == "run" && command == "list" {
        o.run_filters = RunFilters::default();
        o.event.take();
        o.reference.take();
    }
    if command == "create" && matches!(group, "issue" | "pr") {
        o.metadata = MetadataArgs::default();
    }
    if command == "edit" && matches!(group, "issue" | "pr") {
        o.edit = EditArgs::default();
        o.metadata.milestone.take();
        o.metadata.milestone_id.take();
        o.title.take();
        o.body.take();
        o.body_file.take();
        if group == "pr" {
            o.base.take();
        }
    }
    if command == "create" && matches!(group, "issue" | "pr" | "release" | "milestone") {
        o.title.take();
    }
    if (command == "create" && matches!(group, "issue" | "pr" | "release"))
        || (matches!(command, "comment" | "review") && matches!(group, "issue" | "pr"))
    {
        o.body.take();
        o.body_file.take();
    }
    if group == "pr" {
        match command {
            "create" => {
                o.head.take();
                o.base.take();
                o.draft = false;
            }
            "review" => {
                o.event.take();
                o.comments_file.take();
                o.commit.take();
            }
            "merge" => {
                o.style.take();
                o.title.take();
                o.message.take();
                o.delete_branch = false;
                o.match_head.take();
                o.auto = false;
            }
            "request-review" => {
                o.reviewers.clear();
                o.teams.clear();
            }
            _ => {}
        }
    }
    if group == "run" && command == "watch" {
        o.poll.take();
        o.wait.take();
    }
    if group == "release" && command == "create" {
        o.tag.take();
        o.target.take();
        o.draft = false;
        o.prerelease = false;
    }
    if group == "release" && command == "upload" {
        o.name.take();
    }
    if group == "label" && command == "create" {
        o.name.take();
        o.color.take();
        o.description.take();
    }
    if group == "milestone" && command == "create" {
        o.description.take();
        o.due.take();
    }
    if group == "workflow" && command == "dispatch" {
        o.reference.take();
        o.fields.clear();
    }
    o.ensure_empty()
}

fn positive(value: &str, name: &str) -> Result<u64, Error> {
    value
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::usage(format!("{name} must be a positive integer")))
}

fn is_rfc3339_date_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }

    let Some(year) = decimal(bytes, 0, 4) else {
        return false;
    };
    let Some(month) = decimal(bytes, 5, 7) else {
        return false;
    };
    let Some(day) = decimal(bytes, 8, 10) else {
        return false;
    };
    let Some(hour) = decimal(bytes, 11, 13) else {
        return false;
    };
    let Some(minute) = decimal(bytes, 14, 16) else {
        return false;
    };
    let Some(second) = decimal(bytes, 17, 19) else {
        return false;
    };

    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return false;
    }

    let zone_start = if bytes[19] == b'.' {
        let fraction_end = bytes[20..]
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .map_or(bytes.len(), |index| index + 20);
        if fraction_end == 20 {
            return false;
        }
        fraction_end
    } else {
        19
    };

    if bytes.get(zone_start) == Some(&b'Z') {
        return zone_start + 1 == bytes.len();
    }
    if !matches!(bytes.get(zone_start), Some(b'+' | b'-'))
        || zone_start + 6 != bytes.len()
        || bytes[zone_start + 3] != b':'
    {
        return false;
    }
    let Some(offset_hour) = decimal(bytes, zone_start + 1, zone_start + 3) else {
        return false;
    };
    let Some(offset_minute) = decimal(bytes, zone_start + 4, zone_start + 6) else {
        return false;
    };
    offset_hour <= 23 && offset_minute <= 59
}

fn decimal(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    bytes.get(start..end)?.iter().try_fold(0, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(byte - b'0'))
    })
}

fn nonempty(value: &str, name: &str) -> Result<String, Error> {
    if value.is_empty() {
        Err(Error::usage(format!("{name} must not be empty")))
    } else {
        Ok(value.to_owned())
    }
}

fn take_required_nonempty(slot: &mut Option<String>, flag: &str) -> Result<String, Error> {
    slot.take()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::usage(format!("{flag} is required and must not be empty")))
}

fn take_optional_nonempty(slot: &mut Option<String>, flag: &str) -> Result<Option<String>, Error> {
    slot.take()
        .map(|value| {
            if value.is_empty() {
                Err(Error::usage(format!("{flag} must not be empty")))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn string_value(parser: &mut lexopt::Parser, flag: &str) -> Result<String, Error> {
    os_value(parser)?
        .into_string()
        .map_err(|_| Error::usage(format!("{flag} value must be valid UTF-8")))
}

fn os_value(parser: &mut lexopt::Parser) -> Result<OsString, Error> {
    parser
        .value()
        .map_err(|error| Error::usage(error.to_string()))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), Error> {
    if slot.replace(value).is_some() {
        Err(Error::usage(format!("{flag} may be used only once")))
    } else {
        Ok(())
    }
}

fn set_switch(slot: &mut bool, flag: &str) -> Result<(), Error> {
    if *slot {
        Err(Error::usage(format!("{flag} may be used only once")))
    } else {
        *slot = true;
        Ok(())
    }
}
