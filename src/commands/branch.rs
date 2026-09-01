use std::fmt::Write;

use serde::{Deserialize, Serialize};

use crate::args::{Args, BranchArgs, PageArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, collect_arrays, encode_path, reject_read_flags};

#[derive(Deserialize)]
struct Commit {
    id: String,
}

#[derive(Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these independent booleans are Forgejo's private branch response schema"
)]
struct BranchResponse {
    name: String,
    commit: Commit,
    protected: bool,
    enable_status_check: bool,
    required_approvals: u64,
    user_can_merge: bool,
    user_can_push: bool,
}

#[derive(Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the stable branch record reports independent repository permissions and policies"
)]
struct BranchRecord {
    kind: &'static str,
    name: String,
    sha: String,
    protected: bool,
    status_checks: bool,
    required_approvals: u64,
    can_merge: bool,
    can_push: bool,
}

impl From<BranchResponse> for BranchRecord {
    fn from(value: BranchResponse) -> Self {
        Self {
            kind: "branch",
            name: value.name,
            sha: value.commit.id,
            protected: value.protected,
            status_checks: value.enable_status_check,
            required_approvals: value.required_approvals,
            can_merge: value.user_can_merge,
            can_push: value.user_can_push,
        }
    }
}

#[derive(Serialize)]
struct DeleteRecord<'a> {
    kind: &'static str,
    action: &'static str,
    ok: bool,
    name: &'a str,
}

pub(crate) fn run(common: &Args, args: &BranchArgs) -> Result<Outcome, Error> {
    match args {
        BranchArgs::List { paging } => list(common, paging),
        BranchArgs::Delete { name } => delete(common, name),
    }
}

fn list(common: &Args, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "branch list")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("branches");
    let values: Vec<BranchResponse> = collect_arrays(&repo, &path, paging, "branch list")?;
    let records: Vec<BranchRecord> = values.into_iter().map(BranchRecord::from).collect();
    if common.json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}\t{}",
                plain_field(&record.name),
                plain_field(&record.sha),
                record.protected,
                record.status_checks,
                record.can_merge,
                record.can_push
            )
            .map_err(|error| Error::data(format!("could not format branch output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn delete(common: &Args, name: &str) -> Result<Outcome, Error> {
    if !common.yes {
        return Err(Error::safety("branch delete requires --yes"));
    }
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("branches/{}", encode_path(name)));
    if common.dry_run {
        return repo.dry_run_empty(common.json, "DELETE", &path);
    }
    repo.client.send("DELETE", &path, None)?;
    if common.json {
        Outcome::json(&DeleteRecord {
            kind: "result",
            action: "branch.delete",
            ok: true,
            name,
        })
    } else {
        Ok(Outcome::text(format!(
            "branch.delete\t{}\tok\n",
            plain_field(name)
        )))
    }
}
