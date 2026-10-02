use std::fmt::Write;

use serde::{Deserialize, Serialize};

use super::triage;
use crate::args::{Args, IssueArgs, MetadataArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};
use serde_json::{Value, json};

use super::typed::{RepoClient, collect_arrays, read_body, reject_read_flags, reject_yes};

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize, Serialize)]
struct Milestone {
    id: u64,
    title: String,
}

#[derive(Deserialize)]
struct IssueResponse {
    number: u64,
    title: String,
    body: String,
    html_url: String,
    user: User,
    state: String,
    labels: Option<Vec<Label>>,
    assignees: Option<Vec<User>>,
    milestone: Option<Milestone>,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct IssueRecord {
    kind: &'static str,
    number: u64,
    title: String,
    body: String,
    html_url: String,
    author: String,
    state: &'static str,
    labels: Vec<String>,
    assignees: Vec<String>,
    milestone: Option<Milestone>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<IssueResponse> for IssueRecord {
    type Error = Error;

    fn try_from(value: IssueResponse) -> Result<Self, Self::Error> {
        let state = match value.state.as_str() {
            "open" => "open",
            "closed" => "closed",
            other => {
                return Err(Error::data(format!(
                    "Forgejo returned unknown issue state {other:?}"
                )));
            }
        };
        Ok(Self {
            kind: "issue",
            number: value.number,
            title: value.title,
            body: value.body,
            html_url: value.html_url,
            author: value.user.login,
            state,
            labels: value
                .labels
                .unwrap_or_default()
                .into_iter()
                .map(|label| label.name)
                .collect(),
            assignees: value
                .assignees
                .unwrap_or_default()
                .into_iter()
                .map(|user| user.login)
                .collect(),
            milestone: value.milestone,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Serialize)]
struct EditIssue {
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
struct ResultRecord {
    kind: &'static str,
    action: &'static str,
    ok: bool,
    number: Option<u64>,
    html_url: Option<String>,
}

pub(crate) fn run(common: &Args, args: &IssueArgs) -> Result<Outcome, Error> {
    match args {
        IssueArgs::List {
            state,
            paging,
            filters,
        } => {
            reject_read_flags(common, "issue list")?;
            let repo = RepoClient::resolve(common)?;
            let mut path = repo.path(&format!("issues?state={}&type=issues", state.as_str()));
            let me = if filters.author.as_deref() == Some("@me")
                || filters.assignee.as_deref() == Some("@me")
            {
                triage::resolve_assignees(&repo, &["@me".to_owned()])?
                    .first()
                    .cloned()
            } else {
                None
            };
            for (key, value) in [
                ("created_by", &filters.author),
                ("assigned_by", &filters.assignee),
                ("q", &filters.search),
                ("milestones", &filters.milestone),
                ("since", &filters.since),
                ("before", &filters.before),
                ("sort", &filters.sort),
            ] {
                if let Some(value) = value {
                    let value = if value == "@me" && matches!(key, "created_by" | "assigned_by") {
                        me.as_ref().unwrap_or(value)
                    } else {
                        value
                    };
                    let _ = write!(path, "&{key}={}", super::typed::encode_path(value));
                }
            }
            if !filters.labels.is_empty() {
                let _ = write!(
                    path,
                    "&labels={}",
                    super::typed::encode_path(&filters.labels.join(","))
                );
            }
            let values: Vec<IssueResponse> = collect_arrays(&repo, &path, paging, "issue list")?;
            render_list(common.json, values)
        }
        IssueArgs::View { number } => {
            reject_read_flags(common, "issue view")?;
            let repo = RepoClient::resolve(common)?;
            let path = repo.path(&format!("issues/{number}"));
            let (value, _) = repo.get::<IssueResponse>(&path, "issue")?;
            if common.human {
                render_human(value)
            } else {
                render_one(common.json, value)
            }
        }
        IssueArgs::Create {
            title,
            body,
            metadata,
        } => create(common, title, body.as_ref(), metadata),
        IssueArgs::Comments { number, paging } => {
            reject_read_flags(common, "issue comments")?;
            let repo = RepoClient::resolve(common)?;
            let values: Vec<Value> = collect_arrays(
                &repo,
                &repo.path(&format!("issues/{number}/comments")),
                paging,
                "issue comments",
            )?;
            render_comments(common.json, values)
        }
        IssueArgs::Edit { number, edit } => {
            reject_yes(common, "issue edit")?;
            let repo = RepoClient::resolve(common)?;
            let plan = triage::plan_edit(&repo, *number, edit)?;
            if common.dry_run {
                return triage::dry_run_plan(&repo, common.json, &plan);
            }
            triage::execute_plan(&repo, &plan)?;
            result(common.json, "issue.edit", *number, None)
        }
        IssueArgs::Comment { number, body } => comment(common, *number, body),
        IssueArgs::Close { number } => edit_state(common, *number, "closed"),
        IssueArgs::Reopen { number } => edit_state(common, *number, "open"),
    }
}

fn create(
    common: &Args,
    title: &str,
    source: Option<&crate::args::BodySource>,
    metadata: &MetadataArgs,
) -> Result<Outcome, Error> {
    reject_yes(common, "issue create")?;
    let body = source.map(read_body).transpose()?;
    let repo = RepoClient::resolve(common)?;
    let mut payload = triage::resolve_metadata(&repo, metadata)?;
    payload["title"] = json!(title);
    if let Some(body) = body {
        payload["body"] = json!(body);
    }
    let path = repo.path("issues");
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let value: IssueResponse = repo.write("POST", &path, &payload, "issue")?;
    render_one(common.json, value)
}

fn comment(common: &Args, number: u64, source: &crate::args::BodySource) -> Result<Outcome, Error> {
    reject_yes(common, "issue comment")?;
    let body = read_body(source)?;
    let payload = CommentBody { body: &body };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("issues/{number}/comments"));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let response: CommentResponse = repo.write("POST", &path, &payload, "issue comment")?;
    if common.json {
        Outcome::json(
            &json!({"kind":"result", "action":"issue.comment", "ok":true, "number":number, "id":response.id, "html_url":response.html_url}),
        )
    } else {
        result(false, "issue.comment", number, response.html_url)
    }
}

fn edit_state(common: &Args, number: u64, state: &'static str) -> Result<Outcome, Error> {
    reject_yes(common, "issue state change")?;
    let payload = EditIssue { state };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("issues/{number}"));
    if common.dry_run {
        return repo.dry_run(common.json, "PATCH", &path, &payload);
    }
    let value: IssueResponse = repo.write("PATCH", &path, &payload, "issue")?;
    render_one(common.json, value)
}

fn render_human(value: IssueResponse) -> Result<Outcome, Error> {
    let record = IssueRecord::try_from(value)?;
    Ok(Outcome::text(format!(
        "#{} {}\nState: {}\nAuthor: {}\nURL: {}\nLabels: {}\nAssignees: {}\nMilestone: {}\n\n{}\n",
        record.number,
        plain_field(&record.title),
        record.state,
        plain_field(&record.author),
        plain_field(&record.html_url),
        plain_field(&record.labels.join(", ")),
        plain_field(&record.assignees.join(", ")),
        plain_field(&record.milestone.map_or_else(
            || "none".to_owned(),
            |milestone| format!("{} ({})", milestone.title, milestone.id)
        )),
        crate::output::human_text(&record.body)
    )))
}

fn render_comments(json: bool, values: Vec<Value>) -> Result<Outcome, Error> {
    let mut records = Vec::new();
    for value in values {
        let id = value["id"]
            .as_u64()
            .ok_or_else(|| Error::data("comment ID is invalid"))?;
        let field = |key: &str| {
            value[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::data(format!("comment {key} is invalid")))
        };
        let author = value["user"]["login"]
            .as_str()
            .ok_or_else(|| Error::data("comment author is invalid"))?;
        records.push(json!({"kind":"comment", "id":id, "body":field("body")?, "author":author, "html_url":field("html_url")?, "created_at":field("created_at")?, "updated_at":field("updated_at")?}));
    }
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}\t{}",
                record["id"],
                plain_field(record["author"].as_str().unwrap_or_default()),
                plain_field(record["body"].as_str().unwrap_or_default()),
                plain_field(record["html_url"].as_str().unwrap_or_default()),
                plain_field(record["created_at"].as_str().unwrap_or_default()),
                plain_field(record["updated_at"].as_str().unwrap_or_default())
            )
            .map_err(|error| Error::data(error.to_string()))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_list(json: bool, values: Vec<IssueResponse>) -> Result<Outcome, Error> {
    let records: Vec<IssueRecord> = values
        .into_iter()
        .map(IssueRecord::try_from)
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
            .map_err(|error| Error::data(format!("could not format issue output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_one(json: bool, value: IssueResponse) -> Result<Outcome, Error> {
    let record = IssueRecord::try_from(value)?;
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
    use super::{IssueRecord, IssueResponse};

    #[test]
    fn forgejo_issue_fixture_decodes() {
        let value: IssueResponse = serde_json::from_str(include_str!(
            "../../tests/fixtures/forgejo-15.0.7/issue.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        let record = IssueRecord::try_from(value).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(record.number, 12);
        assert_eq!(record.author, "dalton");
    }
}
