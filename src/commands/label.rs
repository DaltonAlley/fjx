use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, LabelArgs, PageArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, collect_arrays, reject_read_flags, reject_yes};

#[derive(Deserialize)]
struct LabelResponse {
    id: u64,
    name: String,
    color: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    exclusive: bool,
    #[serde(default)]
    is_archived: bool,
}

#[derive(Serialize)]
struct LabelRecord {
    kind: &'static str,
    id: u64,
    name: String,
    color: String,
    description: String,
    exclusive: bool,
    archived: bool,
}

impl From<LabelResponse> for LabelRecord {
    fn from(value: LabelResponse) -> Self {
        Self {
            kind: "label",
            id: value.id,
            name: value.name,
            color: value.color,
            description: value.description,
            exclusive: value.exclusive,
            archived: value.is_archived,
        }
    }
}

#[derive(Serialize)]
struct CreateLabel<'a> {
    name: &'a str,
    color: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

pub(crate) fn run(common: &Args, args: &LabelArgs) -> Result<Outcome, Error> {
    match args {
        LabelArgs::List { paging } => list(common, paging),
        LabelArgs::Create {
            name,
            color,
            description,
        } => create(common, name, color, description.as_deref()),
    }
}

fn list(common: &Args, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "label list")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("labels");
    let values = collect_arrays(&repo, &path, paging, "label list")?;
    render(common.json, values)
}

fn create(
    common: &Args,
    name: &str,
    color: &str,
    description: Option<&str>,
) -> Result<Outcome, Error> {
    reject_yes(common, "label create")?;
    let payload = CreateLabel {
        name,
        color,
        description,
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("labels");
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let value = repo.write("POST", &path, &payload, "label")?;
    render_one(common.json, value)
}

fn render(json: bool, values: Vec<LabelResponse>) -> Result<Outcome, Error> {
    let records: Vec<LabelRecord> = values.into_iter().map(LabelRecord::from).collect();
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}\t{}",
                record.id,
                plain_field(&record.name),
                plain_field(&record.color),
                record.exclusive,
                record.archived,
                plain_field(&record.description)
            )
            .map_err(|error| Error::data(format!("could not format label output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_one(json: bool, value: LabelResponse) -> Result<Outcome, Error> {
    let record = LabelRecord::from(value);
    if json {
        Outcome::json(&record)
    } else {
        render(
            false,
            vec![LabelResponse {
                id: record.id,
                name: record.name,
                color: record.color,
                description: record.description,
                exclusive: record.exclusive,
                is_archived: record.archived,
            }],
        )
    }
}
