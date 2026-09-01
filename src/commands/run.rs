use std::fmt::Write;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::args::{Args, PageArgs, RunArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{AdvertisedTotal, MAX_ITEMS, RepoClient, reject_read_flags};

#[derive(Deserialize)]
struct ActionRunResponse {
    id: u64,
    title: String,
    event: String,
    status: String,
    #[serde(default)]
    prettyref: Option<String>,
    #[serde(default)]
    commit_sha: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    updated: Option<String>,
}

#[derive(Deserialize)]
struct ActionRunList {
    total_count: u64,
    workflow_runs: Vec<ActionRunResponse>,
}

#[derive(Serialize)]
struct RunRecord {
    kind: &'static str,
    id: u64,
    name: String,
    event: String,
    status: &'static str,
    conclusion: Option<&'static str>,
    head_branch: Option<String>,
    head_sha: Option<String>,
    html_url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl RunRecord {
    fn from_response(value: ActionRunResponse) -> Self {
        let (status, conclusion) = normalize_status(&value.status);
        Self {
            kind: "run",
            id: value.id,
            name: value.title,
            event: value.event,
            status,
            conclusion,
            head_branch: value.prettyref,
            head_sha: value.commit_sha,
            html_url: value.html_url,
            created_at: value.created,
            updated_at: value.updated,
        }
    }

    fn terminal_code(&self) -> Option<u8> {
        match self.conclusion {
            Some("success") => Some(0),
            Some(_) => Some(1),
            None => None,
        }
    }
}

pub(crate) fn run(common: &Args, args: &RunArgs) -> Result<Outcome, Error> {
    match args {
        RunArgs::List { paging } => list(common, paging),
        RunArgs::View { id } => view(common, *id),
        RunArgs::Watch { id, poll, wait } => watch(common, *id, *poll, *wait),
    }
}

fn list(common: &Args, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "run list")?;
    let repo = RepoClient::resolve(common)?;
    let mut values = Vec::new();
    let mut page = paging.page;
    let mut total = AdvertisedTotal::default();
    loop {
        let path = repo.path(&format!("actions/runs?page={page}&limit={}", paging.limit));
        let (response, headers) = repo.get::<ActionRunList>(&path, "action run list")?;
        let body_total = usize::try_from(response.total_count)
            .map_err(|_| Error::data("action run list total does not fit this platform"))?;
        let collected = values
            .len()
            .checked_add(response.workflow_runs.len())
            .ok_or_else(|| Error::data("action run list count overflow"))?;
        let has_next = total.has_next(
            Some(body_total),
            headers.total_count,
            collected,
            headers.has_next,
            "action run list",
        )?;
        if collected > MAX_ITEMS || (collected == MAX_ITEMS && has_next) {
            return Err(Error::data("action run list exceeds 1,000 items"));
        }
        let received = response.workflow_runs.len();
        values.extend(response.workflow_runs);
        if paging.all && has_next && received == 0 {
            return Err(Error::data(
                "action run list is empty but reports another page",
            ));
        }
        if !paging.all || !has_next || received == 0 {
            break;
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| Error::data("page number overflow"))?;
    }
    render_list(common.json, values)
}

fn view(common: &Args, id: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "run view")?;
    let repo = RepoClient::resolve(common)?;
    let value = get_run(&repo, id)?;
    render(common.json, &RunRecord::from_response(value), 0)
}

fn watch(common: &Args, id: u64, poll: u64, wait: u64) -> Result<Outcome, Error> {
    reject_read_flags(common, "run watch")?;
    let repo = RepoClient::resolve(common)?;
    let start = Instant::now();
    let deadline = Duration::from_secs(wait);
    let mut first = true;
    loop {
        if !first && start.elapsed() >= deadline {
            return Err(Error::network(format!(
                "timed out waiting {wait} seconds for action run {id}"
            )));
        }
        first = false;
        let record = RunRecord::from_response(get_run(&repo, id)?);
        if let Some(code) = record.terminal_code() {
            return render(common.json, &record, code);
        }
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            return Err(Error::network(format!(
                "timed out waiting {wait} seconds for action run {id}"
            )));
        }
        let remaining = deadline
            .checked_sub(elapsed)
            .ok_or_else(|| Error::network("action run wait deadline passed"))?;
        thread::sleep(Duration::from_secs(poll).min(remaining));
    }
}

fn get_run(repo: &RepoClient, id: u64) -> Result<ActionRunResponse, Error> {
    let path = repo.path(&format!("actions/runs/{id}"));
    repo.get(&path, "action run").map(|(value, _)| value)
}

fn normalize_status(value: &str) -> (&'static str, Option<&'static str>) {
    match value.to_ascii_lowercase().as_str() {
        "queued" | "pending" | "requested" => ("queued", None),
        "running" | "in_progress" => ("in_progress", None),
        "waiting" | "blocked" => ("waiting", None),
        "success" => ("completed", Some("success")),
        "failure" | "error" => ("completed", Some("failure")),
        "cancelled" => ("completed", Some("cancelled")),
        "skipped" => ("completed", Some("skipped")),
        "neutral" => ("completed", Some("neutral")),
        "timed_out" => ("completed", Some("timed_out")),
        "action_required" => ("completed", Some("action_required")),
        _ => ("unknown", None),
    }
}

fn render_list(json: bool, values: Vec<ActionRunResponse>) -> Result<Outcome, Error> {
    let records: Vec<RunRecord> = values.into_iter().map(RunRecord::from_response).collect();
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}",
                record.id,
                record.status,
                record.conclusion.unwrap_or(""),
                plain_field(&record.name),
                plain_field(record.html_url.as_deref().unwrap_or(""))
            )
            .map_err(|error| Error::data(format!("could not format run output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render(json: bool, record: &RunRecord, code: u8) -> Result<Outcome, Error> {
    let outcome = if json {
        Outcome::json(&record)?
    } else {
        Outcome::text(format!(
            "{}\t{}\t{}\t{}\t{}\n",
            record.id,
            record.status,
            record.conclusion.unwrap_or(""),
            plain_field(&record.name),
            plain_field(record.html_url.as_deref().unwrap_or(""))
        ))
    };
    Ok(outcome.with_code(code))
}

#[cfg(test)]
mod tests {
    use super::{ActionRunResponse, RunRecord, normalize_status};

    #[test]
    fn forgejo_run_fixture_decodes() {
        let value: ActionRunResponse = serde_json::from_str(include_str!(
            "../../tests/fixtures/forgejo-15.0.7/action-run.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        let record = RunRecord::from_response(value);
        assert_eq!(record.conclusion, Some("success"));
    }

    #[test]
    fn maps_forgejo_action_states() {
        assert_eq!(normalize_status("running"), ("in_progress", None));
        assert_eq!(
            normalize_status("cancelled"),
            ("completed", Some("cancelled"))
        );
    }
}
