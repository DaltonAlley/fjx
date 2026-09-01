use std::ffi::OsString;

use lexopt::prelude::*;

use crate::error::Error;

#[derive(Debug)]
pub(crate) struct Args {
    pub(crate) host: Option<String>,
    pub(crate) repo: Option<String>,
    pub(crate) json: bool,
    pub(crate) dry_run: bool,
    pub(crate) yes: bool,
    pub(crate) command: Command,
}

#[derive(Debug)]
pub(crate) enum Command {
    AuthLogin { with_token: bool },
    AuthStatus,
    AuthLogout,
    AuthSetupGit,
    AuthGitCredential { operation: GitCredentialOperation },
    RepoView,
    Issue(IssueArgs),
    Pull(PullArgs),
    Run(RunArgs),
    Release(ReleaseArgs),
    Label(LabelArgs),
    Milestone(MilestoneArgs),
    Branch(BranchArgs),
    Workflow(WorkflowArgs),
    Api(ApiArgs),
    Help,
    Version,
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
    List {
        state: ListState,
        paging: PageArgs,
    },
    View {
        number: u64,
    },
    Create {
        title: String,
        body: Option<BodySource>,
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
    List {
        state: ListState,
        paging: PageArgs,
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
    },
    Merge {
        number: u64,
        style: MergeStyle,
        title: Option<String>,
        message: Option<String>,
        delete_branch: bool,
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
    List { paging: PageArgs },
    View { id: u64 },
    Watch { id: u64, poll: u64, wait: u64 },
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

#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each CLI switch needs independent duplicate and command-scope checks"
)]
struct Specific {
    with_token: bool,
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
}

#[allow(
    clippy::too_many_lines,
    reason = "one flat parser keeps common flags valid at every command position"
)]
pub(crate) fn parse() -> Result<Args, Error> {
    let mut parser = lexopt::Parser::from_env();
    let (mut host, mut repo) = (None, None);
    let (mut json, mut dry_run, mut yes, mut help, mut version) =
        (false, false, false, false, false);
    let mut specific = Specific::default();
    let mut words = Vec::new();
    while let Some(argument) = parser
        .next()
        .map_err(|error| Error::usage(error.to_string()))?
    {
        match argument {
            Long("host") => set_once(&mut host, string_value(&mut parser, "--host")?, "--host")?,
            Short('R') => set_once(&mut repo, string_value(&mut parser, "-R")?, "-R")?,
            Long("json") => set_switch(&mut json, "--json")?,
            Long("dry-run") => set_switch(&mut dry_run, "--dry-run")?,
            Long("yes") => set_switch(&mut yes, "--yes")?,
            Long("with-token") => set_switch(&mut specific.with_token, "--with-token")?,
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
    let command = if help {
        Command::Help
    } else if version {
        Command::Version
    } else {
        parse_command(&words, &mut specific)?
    };
    specific.ensure_empty()?;
    Ok(Args {
        host,
        repo,
        json,
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
        [a, b] if a == "auth" && b == "login" => Ok(Command::AuthLogin {
            with_token: o.take_with_token(),
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
        [a, b] if a == "repo" && b == "view" => Ok(Command::RepoView),
        [a, b] if a == "issue" && b == "list" => Ok(Command::Issue(IssueArgs::List {
            state: o.take_state()?,
            paging: o.take_paging()?,
        })),
        [a, b, n] if a == "issue" && b == "view" => Ok(Command::Issue(IssueArgs::View {
            number: positive(n, "issue number")?,
        })),
        [a, b] if a == "issue" && b == "create" => Ok(Command::Issue(IssueArgs::Create {
            title: o.take_required_title()?,
            body: o.take_body(false)?,
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
        _ => Err(Error::usage(
            "unknown or incomplete command; run fjx --help",
        )),
    }
}

impl Specific {
    fn take_with_token(&mut self) -> bool {
        std::mem::take(&mut self.with_token)
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
        if !matches!(event, ReviewEvent::Approve) && body.is_none() {
            return Err(Error::usage(
                "review body is required for request-changes and comment",
            ));
        }
        Ok(PullArgs::Review {
            number,
            event,
            body,
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
            (self.with_token, "--with-token"),
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
