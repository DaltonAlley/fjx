use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, ListState, MilestoneArgs, PageArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, collect_arrays, reject_read_flags, reject_yes};

#[derive(Deserialize)]
struct MilestoneResponse {
    id: u64,
    title: String,
    description: String,
    state: String,
    open_issues: u64,
    closed_issues: u64,
    due_on: Option<String>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
}

#[derive(Serialize)]
struct MilestoneRecord {
    kind: &'static str,
    id: u64,
    title: String,
    description: String,
    state: &'static str,
    open_issues: u64,
    closed_issues: u64,
    due_on: Option<String>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
}

impl TryFrom<MilestoneResponse> for MilestoneRecord {
    type Error = Error;

    fn try_from(value: MilestoneResponse) -> Result<Self, Self::Error> {
        let state = match value.state.as_str() {
            "open" => "open",
            "closed" => "closed",
            other => {
                return Err(Error::data(format!(
                    "Forgejo returned unknown milestone state {other:?}"
                )));
            }
        };
        Ok(Self {
            kind: "milestone",
            id: value.id,
            title: value.title,
            description: value.description,
            state,
            open_issues: value.open_issues,
            closed_issues: value.closed_issues,
            due_on: value.due_on,
            created_at: value.created_at,
            updated_at: value.updated_at,
            closed_at: value.closed_at,
        })
    }
}

#[derive(Serialize)]
struct CreateMilestone<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    due_on: Option<&'a str>,
}

pub(crate) fn run(common: &Args, args: &MilestoneArgs) -> Result<Outcome, Error> {
    match args {
        MilestoneArgs::List { state, paging } => list(common, *state, paging),
        MilestoneArgs::Create {
            title,
            description,
            due,
        } => create(common, title, description.as_deref(), due.as_deref()),
    }
}

fn list(common: &Args, state: ListState, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "milestone list")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("milestones?state={}", state.as_str()));
    let values = collect_arrays(&repo, &path, paging, "milestone list")?;
    render(common.json, values)
}

fn create(
    common: &Args,
    title: &str,
    description: Option<&str>,
    due_on: Option<&str>,
) -> Result<Outcome, Error> {
    reject_yes(common, "milestone create")?;
    let payload = CreateMilestone {
        title,
        description,
        due_on,
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("milestones");
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let value = repo.write("POST", &path, &payload, "milestone")?;
    render_one(common.json, value)
}

fn records(values: Vec<MilestoneResponse>) -> Result<Vec<MilestoneRecord>, Error> {
    values.into_iter().map(MilestoneRecord::try_from).collect()
}

fn render(json: bool, values: Vec<MilestoneResponse>) -> Result<Outcome, Error> {
    let records = records(values)?;
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}\t{}",
                record.id,
                record.state,
                plain_field(&record.title),
                record.open_issues,
                record.closed_issues,
                plain_field(record.due_on.as_deref().unwrap_or(""))
            )
            .map_err(|error| Error::data(format!("could not format milestone output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_one(json: bool, value: MilestoneResponse) -> Result<Outcome, Error> {
    let record = MilestoneRecord::try_from(value)?;
    if json {
        Outcome::json(&record)
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            record.id,
            record.state,
            plain_field(&record.title),
            record.open_issues,
            record.closed_issues,
            plain_field(record.due_on.as_deref().unwrap_or(""))
        )))
    }
}
