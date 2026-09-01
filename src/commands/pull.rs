use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, BodySource, PullArgs, ReviewEvent};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{
    MAX_ITEMS, RepoClient, collect_arrays, encode_path, read_body, reject_read_flags, reject_yes,
};

const STATUS_PAGE_LIMIT: usize = 50;

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
    html_url: Option<String>,
}

#[derive(Serialize)]
struct ReviewBody<'a> {
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(Deserialize)]
struct ReviewResponse {
    html_url: Option<String>,
}

#[derive(Serialize)]
struct MergeBody<'a> {
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

pub(crate) fn run(common: &Args, args: &PullArgs) -> Result<Outcome, Error> {
    match args {
        PullArgs::List { state, paging } => {
            reject_read_flags(common, "pr list")?;
            let repo = RepoClient::resolve(common)?;
            let path = repo.path(&format!("pulls?state={}", state.as_str()));
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
        } => create(common, head, title, base.as_deref(), body.as_ref(), *draft),
        PullArgs::Diff { number } => diff(common, *number),
        PullArgs::Checks { number } => checks(common, *number),
        PullArgs::Comment { number, body } => comment(common, *number, body),
        PullArgs::Review {
            number,
            event,
            body,
        } => review(common, *number, *event, body.as_ref()),
        PullArgs::Merge {
            number,
            style,
            title,
            message,
            delete_branch,
        } => merge(
            common,
            *number,
            style.as_str(),
            title.as_deref(),
            message.as_deref(),
            *delete_branch,
        ),
        PullArgs::Close { number } => edit_state(common, *number, "closed"),
        PullArgs::Reopen { number } => edit_state(common, *number, "open"),
    }
}

fn view(common: &Args, number: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "pr view")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}"));
    let (value, _) = repo.get::<PullResponse>(&path, "pull request")?;
    render_one(common.json, value)
}

fn create(
    common: &Args,
    head: &str,
    title: &str,
    base: Option<&str>,
    source: Option<&BodySource>,
    draft: bool,
) -> Result<Outcome, Error> {
    reject_yes(common, "pr create")?;
    let body = source.map(read_body).transpose()?;
    // Forgejo 15.0.7 has no draft request field. Its default WIP prefix marks the
    // pull request as a draft during the same create request.
    let draft_title = draft.then(|| format!("WIP: {title}"));
    let payload = CreatePull {
        head,
        title: draft_title.as_deref().unwrap_or(title),
        base,
        body: body.as_deref(),
    };
    let repo = RepoClient::resolve(common)?;
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
    let state = normalize_check(&combined.state);
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
    result(common.json, "pr.comment", number, response.html_url)
}

fn review(
    common: &Args,
    number: u64,
    event: ReviewEvent,
    source: Option<&BodySource>,
) -> Result<Outcome, Error> {
    reject_yes(common, "pr review")?;
    let body = source.map(read_body).transpose()?;
    let event = match event {
        ReviewEvent::Approve => "APPROVED",
        ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        ReviewEvent::Comment => "COMMENT",
    };
    let payload = ReviewBody {
        event,
        body: body.as_deref(),
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("pulls/{number}/reviews"));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let response: ReviewResponse = repo.write("POST", &path, &payload, "pull request review")?;
    result(common.json, "pr.review", number, response.html_url)
}

fn merge(
    common: &Args,
    number: u64,
    style: &'static str,
    title: Option<&str>,
    message: Option<&str>,
    delete_branch: bool,
) -> Result<Outcome, Error> {
    if !common.yes {
        return Err(Error::safety("pull request merge requires --yes"));
    }
    let payload = MergeBody {
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
