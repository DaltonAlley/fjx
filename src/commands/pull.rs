use std::ffi::OsStr;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, BodySource, MetadataArgs, PageArgs, PullArgs, ReviewEvent};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{
    MAX_ITEMS, RepoClient, collect_arrays, encode_path, read_body, reject_read_flags, reject_yes,
};

const STATUS_PAGE_LIMIT: usize = 50;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InlineComment {
    body: String,
    path: String,
    #[serde(default)]
    old_position: i64,
    #[serde(default)]
    new_position: i64,
}

fn read_inline_comments(path: &OsStr, commit: Option<&str>) -> Result<Vec<InlineComment>, Error> {
    if !commit.is_some_and(|sha| {
        matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        return Err(Error::usage(
            "inline review comments require an explicit --commit SHA",
        ));
    }
    let text = read_body(&BodySource::File(path.to_owned()))?;
    let comments: Vec<InlineComment> = serde_json::from_str(&text)
        .map_err(|e| Error::usage(format!("invalid review comments JSON: {e}")))?;
    if comments.is_empty() || comments.len() > MAX_ITEMS {
        return Err(Error::usage(
            "review comments must contain 1 to 1,000 items",
        ));
    }
    for comment in &comments {
        if comment.body.trim().is_empty()
            || comment
                .body
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
            || comment.path.is_empty()
            || comment.path.starts_with('/')
            || comment
                .path
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
            || comment.path.chars().any(char::is_control)
            || comment.old_position < 0
            || comment.new_position < 0
            || (comment.old_position == 0) == (comment.new_position == 0)
        {
            return Err(Error::usage(
                "review comment requires a safe body, repository path, and exactly one positive line position",
            ));
        }
    }
    Ok(comments)
}

fn collection(
    common: &Args,
    number: u64,
    suffix: &str,
    paging: &PageArgs,
    kind: &str,
) -> Result<Outcome, Error> {
    reject_read_flags(common, "pr collection")?;
    let repo = RepoClient::resolve(common)?;
    let values: Vec<serde_json::Value> = collect_arrays(&repo, &repo.path(suffix), paging, kind)?;
    let mut records = Vec::new();
    let mut text = String::new();
    for value in values {
        let mut record = if kind == "file" {
            let filename = value
                .get("filename")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::data("changed file has no filename"))?;
            serde_json::json!({"kind":kind,"number":number,"id":filename,"filename":filename,"previous_filename":value.get("previous_filename"),"status":value.get("status"),"additions":value.get("additions"),"deletions":value.get("deletions"),"changes":value.get("changes")})
        } else {
            let id = value
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .filter(|id| *id > 0)
                .ok_or_else(|| Error::data("review or comment has no stable ID"))?;
            serde_json::json!({"kind":kind,"number":number,"id":id,"body":value.get("body"),"author":value.pointer("/user/login"),"state":value.get("state"),"html_url":value.get("html_url"),"created_at":value.get("created_at"),"updated_at":value.get("updated_at"),"commit_id":value.get("commit_id"),"path":value.get("path"),"old_position":value.get("original_position"),"new_position":value.get("position"),"pull_request_review_id":value.get("pull_request_review_id")})
        };
        if kind == "review" {
            for key in [
                "created_at",
                "updated_at",
                "path",
                "old_position",
                "new_position",
                "pull_request_review_id",
            ] {
                record.as_object_mut().map(|object| object.remove(key));
            }
            for key in ["submitted_at", "dismissed", "stale", "comments_count"] {
                record[key] = value.get(key).cloned().unwrap_or(serde_json::Value::Null);
            }
        } else if kind == "comment" {
            for key in [
                "state",
                "commit_id",
                "path",
                "old_position",
                "new_position",
                "pull_request_review_id",
            ] {
                record.as_object_mut().map(|object| object.remove(key));
            }
        }
        writeln!(
            &mut text,
            "{}\t{}\t{}",
            plain_field(
                &record["id"]
                    .as_str()
                    .map_or_else(|| record["id"].to_string(), str::to_owned)
            ),
            plain_field(record["author"].as_str().unwrap_or("")),
            plain_field(
                record["body"]
                    .as_str()
                    .or(record["filename"].as_str())
                    .unwrap_or("")
            )
        )
        .map_err(|e| Error::data(e.to_string()))?;
        records.push(record);
    }
    if common.json {
        Outcome::json(&records)
    } else {
        Ok(Outcome::text(text))
    }
}

fn request_review(
    common: &Args,
    number: u64,
    reviewers: &[String],
    teams: &[String],
) -> Result<Outcome, Error> {
    reject_yes(common, "pr request-review")?;
    if reviewers.is_empty() && teams.is_empty() {
        return Err(Error::usage("request-review requires reviewers or teams"));
    }
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}/requested_reviewers"));
    let payload = serde_json::json!({"reviewers":reviewers,"team_reviewers":teams});
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    repo.write_empty("POST", &path, &payload)?;
    result(common.json, "pr.request-review", number, None)
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct Branch {
    r#ref: String,
    sha: String,
}

#[derive(Deserialize)]
struct PullResponse {
    #[serde(default)]
    merged: bool,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
    #[serde(default)]
    labels: Option<Vec<serde_json::Value>>,
    milestone: Option<serde_json::Value>,
    #[serde(default)]
    assignees: Option<Vec<serde_json::Value>>,
    number: u64,
    title: String,
    body: String,
    html_url: String,
    user: User,
    state: String,
    draft: bool,
    mergeable: Option<bool>,
    base: Branch,
    head: Branch,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct PullRecord {
    merged: bool,
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
    labels: Vec<String>,
    milestone: Option<serde_json::Value>,
    assignees: Vec<String>,
    kind: &'static str,
    number: u64,
    title: String,
    body: String,
    html_url: String,
    author: String,
    state: &'static str,
    draft: bool,
    mergeable: Option<bool>,
    base: String,
    head: String,
    head_sha: String,
    created_at: String,
    updated_at: String,
}

impl TryFrom<PullResponse> for PullRecord {
    type Error = Error;

    fn try_from(value: PullResponse) -> Result<Self, Self::Error> {
        let state = match value.state.as_str() {
            "open" => "open",
            "closed" => "closed",
            other => {
                return Err(Error::data(format!(
                    "Forgejo returned unknown pull request state {other:?}"
                )));
            }
        };
        Ok(Self {
            merged: value.merged,
            merged_at: value.merged_at,
            merge_commit_sha: value.merge_commit_sha,
            labels: value
                .labels
                .unwrap_or_default()
                .iter()
                .filter_map(|v| v["name"].as_str().map(str::to_owned))
                .collect(),
            milestone: value
                .milestone
                .map(|v| serde_json::json!({"id":v["id"],"title":v["title"]})),
            assignees: value
                .assignees
                .unwrap_or_default()
                .iter()
                .filter_map(|v| v["login"].as_str().map(str::to_owned))
                .collect(),
            kind: "pull_request",
            number: value.number,
            title: value.title,
            body: value.body,
            html_url: value.html_url,
            author: value.user.login,
            state,
            draft: value.draft,
            mergeable: value.mergeable,
            base: value.base.r#ref,
            head: value.head.r#ref,
            head_sha: value.head.sha,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Serialize)]
struct CreatePull<'a> {
    #[serde(flatten)]
    metadata: serde_json::Value,
    head: &'a str,
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(Serialize)]
struct EditPull {
    state: &'static str,
}

#[derive(Serialize)]
struct CommentBody<'a> {
    body: &'a str,
}

#[derive(Deserialize)]
struct CommentResponse {
    id: Option<u64>,
    html_url: Option<String>,
}

#[derive(Serialize)]
struct ReviewBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comments: Option<Vec<InlineComment>>,
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(Deserialize)]
struct ReviewResponse {
    id: Option<u64>,
    html_url: Option<String>,
}

#[derive(Serialize)]
struct MergeBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    head_commit_id: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    merge_when_checks_succeed: bool,
    #[serde(rename = "Do")]
    style: &'static str,
    #[serde(rename = "MergeTitleField", skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    #[serde(rename = "MergeMessageField", skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    delete_branch_after_merge: bool,
}

#[derive(Serialize)]
struct ResultRecord {
    kind: &'static str,
    action: &'static str,
    ok: bool,
    number: Option<u64>,
    html_url: Option<String>,
}

#[derive(Serialize)]
struct DiffRecord {
    kind: &'static str,
    number: u64,
    diff: String,
}

#[derive(Deserialize)]
struct CombinedStatus {
    sha: String,
    state: String,
    statuses: Option<Vec<CommitStatus>>,
    total_count: Option<usize>,
}

#[derive(Deserialize)]
struct CommitStatus {
    context: String,
    status: String,
    description: Option<String>,
    target_url: Option<String>,
}

#[derive(Serialize)]
struct ChecksRecord {
    kind: &'static str,
    sha: String,
    state: &'static str,
    statuses: Vec<StatusRecord>,
}

#[derive(Serialize)]
struct StatusRecord {
    context: String,
    state: &'static str,
    description: Option<String>,
    target_url: Option<String>,
}

#[allow(
    clippy::too_many_lines,
    reason = "dispatch keeps PR command routes together"
)]
pub(crate) fn run(common: &Args, args: &PullArgs) -> Result<Outcome, Error> {
    match args {
        PullArgs::List {
            state,
            paging,
            filters,
        } => {
            reject_read_flags(common, "pr list")?;
            let repo = RepoClient::resolve(common)?;
            let mut path = repo.path(&format!("pulls?state={}", state.as_str()));
            for id in super::triage::resolve_labels(&repo, &filters.labels, &[])? {
                write!(&mut path, "&labels={id}").map_err(|e| Error::data(e.to_string()))?;
            }
            let milestone =
                super::triage::resolve_milestone(&repo, filters.milestone.as_deref(), None)?;
            for (key, value) in [
                (
                    "poster",
                    super::triage::resolve_assignees(
                        &repo,
                        &filters.author.iter().cloned().collect::<Vec<_>>(),
                    )?
                    .first()
                    .cloned(),
                ),
                ("milestone", milestone.map(|id| id.to_string())),
                ("sort", filters.sort.clone()),
            ] {
                if let Some(value) = value {
                    write!(&mut path, "&{key}={}", encode_path(&value))
                        .map_err(|e| Error::data(e.to_string()))?;
                }
            }
            let values: Vec<PullResponse> =
                collect_arrays(&repo, &path, paging, "pull request list")?;
            render_list(common.json, values)
        }
        PullArgs::View { number } => view(common, *number),
        PullArgs::Create {
            head,
            title,
            base,
            body,
            draft,
            metadata,
        } => create(
            common,
            head,
            title,
            base.as_deref(),
            body.as_ref(),
            *draft,
            metadata,
        ),
        PullArgs::Diff { number } => diff(common, *number),
        PullArgs::Checks { number } => checks(common, *number),
        PullArgs::Comment { number, body } => comment(common, *number, body),
        PullArgs::Review {
            number,
            event,
            body,
            comments_file,
            commit,
        } => review(
            common,
            *number,
            *event,
            body.as_ref(),
            comments_file.as_deref(),
            commit.as_deref(),
        ),
        PullArgs::Merge {
            number,
            style,
            title,
            message,
            delete_branch,
            match_head,
            auto,
        } => merge(
            common,
            *number,
            style.as_str(),
            title.as_deref(),
            message.as_deref(),
            *delete_branch,
            match_head.as_deref(),
            *auto,
        ),
        PullArgs::Comments { number, paging } => collection(
            common,
            *number,
            &format!("issues/{number}/comments"),
            paging,
            "comment",
        ),
        PullArgs::Reviews { number, paging } => collection(
            common,
            *number,
            &format!("pulls/{number}/reviews"),
            paging,
            "review",
        ),
        PullArgs::Files { number, paging } => collection(
            common,
            *number,
            &format!("pulls/{number}/files"),
            paging,
            "file",
        ),
        PullArgs::ReviewComments {
            number,
            review_id,
            paging,
        } => collection(
            common,
            *number,
            &format!("pulls/{number}/reviews/{review_id}/comments"),
            paging,
            "review_comment",
        ),
        PullArgs::Edit { number, edit } => {
            reject_yes(common, "pr edit")?;
            let repo = RepoClient::resolve(common)?;
            let mut plan = super::triage::plan_edit(&repo, *number, edit)?;
            for request in &mut plan {
                if request.method == "PATCH"
                    && request.path == repo.path(&format!("issues/{number}"))
                {
                    request.path = repo.path(&format!("pulls/{number}"));
                }
            }
            if common.dry_run {
                return super::triage::dry_run_plan(&repo, common.json, &plan);
            }
            super::triage::execute_plan(&repo, &plan)?;
            result(common.json, "pr.edit", *number, None)
        }
        PullArgs::RequestReview {
            number,
            reviewers,
            teams,
        } => request_review(common, *number, reviewers, teams),
        PullArgs::Close { number } => edit_state(common, *number, "closed"),
        PullArgs::Reopen { number } => edit_state(common, *number, "open"),
    }
}

fn view(common: &Args, number: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "pr view")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}"));
    let (value, _) = repo.get::<PullResponse>(&path, "pull request")?;
    if common.human {
        return render_human(value);
    }
    render_one(common.json, value)
}

#[allow(
    clippy::too_many_arguments,
    reason = "create payload mirrors the CLI options"
)]
fn create(
    common: &Args,
    head: &str,
    title: &str,
    base: Option<&str>,
    source: Option<&BodySource>,
    draft: bool,
    metadata: &MetadataArgs,
) -> Result<Outcome, Error> {
    reject_yes(common, "pr create")?;
    let body = source.map(read_body).transpose()?;
    // Forgejo 15.0.7 has no draft request field. Its default WIP prefix marks the
    // pull request as a draft during the same create request.
    let draft_title = draft.then(|| format!("WIP: {title}"));
    let repo = RepoClient::resolve(common)?;
    let payload = CreatePull {
        metadata: super::triage::resolve_metadata(&repo, metadata)?,
        head,
        title: draft_title.as_deref().unwrap_or(title),
        base,
        body: body.as_deref(),
    };
    let path = repo.path("pulls");
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let value: PullResponse = repo.write("POST", &path, &payload, "pull request")?;
    render_one(common.json, value)
}

fn diff(common: &Args, number: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "pr diff")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}.diff"));
    let response = repo.client.send("GET", &path, None)?;
    let text = String::from_utf8(response.bytes)
        .map_err(|_| Error::data("Forgejo returned a non-UTF-8 diff"))?;
    if common.json {
        Outcome::json(&DiffRecord {
            kind: "diff",
            number,
            diff: text,
        })
    } else {
        Ok(Outcome::text(text))
    }
}

fn checks(common: &Args, number: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "pr checks")?;
    let repo = RepoClient::resolve(common)?;
    let pull_path = repo.path(&format!("pulls/{number}"));
    let (pull, _) = repo.get::<PullResponse>(&pull_path, "pull request")?;
    let path = repo.path(&format!("commits/{}/status", encode_path(&pull.head.sha)));
    let combined = collect_statuses(&repo, &path)?;
    if combined.sha != pull.head.sha {
        return Err(Error::data(
            "combined status SHA does not match pull request head",
        ));
    }
    let state = normalize_check(&combined.state);
    if state == "success"
        && combined.statuses.as_ref().is_none_or(|statuses| {
            statuses.is_empty()
                || statuses
                    .iter()
                    .any(|status| normalize_check(&status.status) != "success")
        })
    {
        return Err(Error::data(
            "successful combined status contradicts child statuses",
        ));
    }
    let statuses = combined
        .statuses
        .unwrap_or_default()
        .into_iter()
        .map(|status| StatusRecord {
            context: status.context,
            state: normalize_check(&status.status),
            description: status.description,
            target_url: status.target_url,
        })
        .collect();
    let record = ChecksRecord {
        kind: "checks",
        sha: combined.sha,
        state,
        statuses,
    };
    let outcome = if common.json {
        Outcome::json(&record)?
    } else {
        let mut text = format!("{}\t{}\n", plain_field(&record.sha), record.state);
        for status in &record.statuses {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}",
                plain_field(&status.context),
                status.state,
                plain_field(status.description.as_deref().unwrap_or("")),
                plain_field(status.target_url.as_deref().unwrap_or(""))
            )
            .map_err(|error| Error::data(format!("could not format checks output: {error}")))?;
        }
        Outcome::text(text)
    };
    Ok(if state == "success" {
        outcome
    } else {
        outcome.with_code(1)
    })
}

fn collect_statuses(repo: &RepoClient, base: &str) -> Result<CombinedStatus, Error> {
    let mut page = 1_u64;
    let mut metadata = None;
    let mut statuses = Vec::new();
    let mut expected_total = None;
    loop {
        let path = format!("{base}?page={page}&limit={STATUS_PAGE_LIMIT}");
        let (combined, response) = repo.get::<CombinedStatus>(&path, "combined status")?;
        let CombinedStatus {
            sha,
            state,
            statuses: page_statuses,
            total_count,
        } = combined;
        if let Some((expected_sha, expected_state)) = &metadata {
            if sha != *expected_sha {
                return Err(Error::data("combined status SHA changed between pages"));
            }
            if state != *expected_state {
                return Err(Error::data("combined status state changed between pages"));
            }
        } else {
            metadata = Some((sha, state));
        }
        let page_statuses = page_statuses.unwrap_or_default();
        let collected = statuses
            .len()
            .checked_add(page_statuses.len())
            .ok_or_else(|| Error::data("combined status count overflow"))?;
        let advertised_total = match (total_count, response.total_count) {
            (Some(body), Some(header)) if body != header => {
                return Err(Error::data(
                    "combined status body and header totals disagree",
                ));
            }
            (Some(total), _) | (_, Some(total)) => Some(total),
            (None, None) => None,
        };
        if let Some(advertised) = advertised_total {
            if expected_total.is_some_and(|expected| expected != advertised) {
                return Err(Error::data("combined status total changed between pages"));
            }
            expected_total = Some(advertised);
        }
        if expected_total.is_some_and(|total| collected > total) {
            return Err(Error::data("combined status exceeds its advertised total"));
        }
        if response.has_next && expected_total.is_some_and(|total| collected == total) {
            return Err(Error::data(
                "combined status reports another page after its advertised total",
            ));
        }
        let next_by_total = expected_total.is_some_and(|total| collected < total);
        let has_next = response.has_next || next_by_total;
        if collected > MAX_ITEMS || (collected == MAX_ITEMS && has_next) {
            return Err(Error::data("combined status exceeds 1,000 items"));
        }
        if has_next && page_statuses.is_empty() {
            return Err(Error::data(
                "combined status is empty but reports another page",
            ));
        }
        statuses.extend(page_statuses);
        if !has_next {
            break;
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| Error::data("page number overflow"))?;
    }
    let (sha, state) =
        metadata.ok_or_else(|| Error::data("Forgejo returned no combined status response"))?;
    Ok(CombinedStatus {
        sha,
        state,
        statuses: Some(statuses),
        total_count: None,
    })
}

fn comment(common: &Args, number: u64, source: &BodySource) -> Result<Outcome, Error> {
    reject_yes(common, "pr comment")?;
    let body = read_body(source)?;
    let payload = CommentBody { body: &body };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("issues/{number}/comments"));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let response: CommentResponse = repo.write("POST", &path, &payload, "pull request comment")?;
    if common.json {
        return Outcome::json(
            &serde_json::json!({"kind":"result","action":"pr.comment","ok":true,"number":number,"html_url":response.html_url,"id":response.id}),
        );
    }
    result(common.json, "pr.comment", number, response.html_url)
}

fn review(
    common: &Args,
    number: u64,
    event: ReviewEvent,
    source: Option<&BodySource>,
    comments_file: Option<&OsStr>,
    commit: Option<&str>,
) -> Result<Outcome, Error> {
    reject_yes(common, "pr review")?;
    if comments_file == Some(OsStr::new("-"))
        && matches!(source, Some(BodySource::File(path)) if path == "-")
    {
        return Err(Error::usage(
            "body-file and comments-file cannot both read stdin",
        ));
    }
    let comments = comments_file
        .map(|path| read_inline_comments(path, commit))
        .transpose()?;
    let body = source.map(read_body).transpose()?;
    let event = match event {
        ReviewEvent::Approve => "APPROVED",
        ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        ReviewEvent::Comment => "COMMENT",
    };
    let payload = ReviewBody {
        commit_id: commit,
        comments,
        event,
        body: body.as_deref(),
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}/reviews"));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let response: ReviewResponse = repo.write("POST", &path, &payload, "pull request review")?;
    if common.json {
        return Outcome::json(
            &serde_json::json!({"kind":"result","action":"pr.review","ok":true,"number":number,"html_url":response.html_url,"id":response.id}),
        );
    }
    result(common.json, "pr.review", number, response.html_url)
}

#[allow(
    clippy::too_many_arguments,
    reason = "merge payload mirrors CLI options"
)]
fn merge(
    common: &Args,
    number: u64,
    style: &'static str,
    title: Option<&str>,
    message: Option<&str>,
    delete_branch: bool,
    match_head: Option<&str>,
    auto: bool,
) -> Result<Outcome, Error> {
    if !common.yes {
        return Err(Error::safety("pull request merge requires --yes"));
    }
    let payload = MergeBody {
        head_commit_id: match_head,
        merge_when_checks_succeed: auto,
        style,
        title,
        message,
        delete_branch_after_merge: delete_branch,
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}/merge"));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    repo.write_empty("POST", &path, &payload)?;
    result(common.json, "pr.merge", number, None)
}

fn edit_state(common: &Args, number: u64, state: &'static str) -> Result<Outcome, Error> {
    reject_yes(common, "pr state change")?;
    let payload = EditPull { state };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}"));
    if common.dry_run {
        return repo.dry_run(common.json, "PATCH", &path, &payload);
    }
    let value: PullResponse = repo.write("PATCH", &path, &payload, "pull request")?;
    render_one(common.json, value)
}

fn normalize_check(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "success" => "success",
        "failure" | "error" => "failure",
        "pending" => "pending",
        _ => "unknown",
    }
}

fn render_list(json: bool, values: Vec<PullResponse>) -> Result<Outcome, Error> {
    let records: Vec<PullRecord> = values
        .into_iter()
        .map(PullRecord::try_from)
        .collect::<Result<_, _>>()?;
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}",
                record.number,
                record.state,
                plain_field(&record.title),
                plain_field(&record.author),
                plain_field(&record.html_url)
            )
            .map_err(|error| Error::data(format!("could not format pull output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_human(value: PullResponse) -> Result<Outcome, Error> {
    let record = PullRecord::try_from(value)?;
    let labels = record.labels.join(", ");
    let assignees = record.assignees.join(", ");
    Ok(Outcome::text(format!(
        "#{} {}\nState: {}\nAuthor: {}\nBase: {}\nHead: {} ({})\nMerged: {}\nLabels: {}\nMilestone: {}\nAssignees: {}\nURL: {}\n\n{}\n",
        record.number,
        plain_field(&record.title),
        record.state,
        plain_field(&record.author),
        plain_field(&record.base),
        plain_field(&record.head),
        plain_field(&record.head_sha),
        record.merged,
        plain_field(&labels),
        plain_field(
            record
                .milestone
                .as_ref()
                .and_then(|m| m["title"].as_str())
                .unwrap_or("")
        ),
        plain_field(&assignees),
        plain_field(&record.html_url),
        crate::output::human_text(&record.body)
    )))
}

fn render_one(json: bool, value: PullResponse) -> Result<Outcome, Error> {
    let record = PullRecord::try_from(value)?;
    if json {
        Outcome::json(&record)
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\t{}\t{}\n",
            record.number,
            record.state,
            plain_field(&record.title),
            plain_field(&record.author),
            plain_field(&record.html_url)
        )))
    }
}

fn result(
    json: bool,
    action: &'static str,
    number: u64,
    html_url: Option<String>,
) -> Result<Outcome, Error> {
    if json {
        Outcome::json(&ResultRecord {
            kind: "result",
            action,
            ok: true,
            number: Some(number),
            html_url,
        })
    } else {
        Ok(Outcome::text(format!("{action}\t{number}\tok\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::{PullRecord, PullResponse, normalize_check};

    #[test]
    fn forgejo_pull_fixture_decodes() {
        let value: PullResponse = serde_json::from_str(include_str!(
            "../../tests/fixtures/forgejo-15.0.7/pull-request.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        let record = PullRecord::try_from(value).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(record.head_sha, "abc123");
    }

    #[test]
    fn check_states_are_closed() {
        assert_eq!(normalize_check("error"), "failure");
        assert_eq!(normalize_check("new-server-state"), "unknown");
    }
}
