use std::collections::BTreeMap;
use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, WorkflowArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, encode_path, reject_yes};

#[derive(Serialize)]
struct DispatchPayload<'a> {
    r#ref: &'a str,
    inputs: BTreeMap<&'a str, &'a str>,
    return_run_info: bool,
}

#[derive(Deserialize)]
struct DispatchResponse {
    id: u64,
    run_number: u64,
    jobs: Vec<String>,
}

#[derive(Serialize)]
struct DispatchRecord {
    kind: &'static str,
    id: u64,
    run_number: u64,
    jobs: Vec<String>,
}

pub(crate) fn run(common: &Args, args: &WorkflowArgs) -> Result<Outcome, Error> {
    reject_yes(common, "workflow dispatch")?;
    let inputs = args
        .fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let payload = DispatchPayload {
        r#ref: &args.reference,
        inputs,
        return_run_info: true,
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!(
        "actions/workflows/{}/dispatches",
        encode_path(&args.file)
    ));
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let response: DispatchResponse = repo.write("POST", &path, &payload, "workflow dispatch")?;
    let record = DispatchRecord {
        kind: "workflow_dispatch",
        id: response.id,
        run_number: response.run_number,
        jobs: response.jobs,
    };
    if common.json {
        Outcome::json(&record)
    } else {
        let mut text = format!("{}\t{}\n", record.id, record.run_number);
        for job in record.jobs {
            writeln!(&mut text, "{}", plain_field(&job)).map_err(|error| {
                Error::data(format!(
                    "could not format workflow dispatch output: {error}"
                ))
            })?;
        }
        Ok(Outcome::text(text))
    }
}
