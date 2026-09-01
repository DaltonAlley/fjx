use serde::{Deserialize, Serialize};

use crate::args::Args;
use crate::context::Context;
use crate::error::Error;
use crate::http::Client;
use crate::output::{Outcome, plain_field};

#[derive(Deserialize)]
struct RepositoryResponse {
    name: String,
    full_name: String,
    description: Option<String>,
    private: bool,
    archived: bool,
    default_branch: String,
    html_url: String,
}

#[derive(Serialize)]
struct RepositoryRecord<'a> {
    kind: &'static str,
    name: &'a str,
    full_name: &'a str,
    description: &'a Option<String>,
    private: bool,
    archived: bool,
    default_branch: &'a str,
    html_url: &'a str,
}

pub(crate) fn view(args: &Args) -> Result<Outcome, Error> {
    if args.dry_run || args.yes {
        return Err(Error::usage(
            "write safety flags are not valid for repo view",
        ));
    }
    let context = Context::resolve(args, true, true)?;
    let repo = context
        .repo
        .as_ref()
        .ok_or_else(|| Error::context("repository is required"))?;
    let relative = format!("repos/{}/{}", repo.owner(), repo.name());
    let client = Client::new(
        context.host,
        context
            .token
            .ok_or_else(|| Error::context("token is required"))?,
    );
    let response = client.send("GET", &relative, None)?;
    let repository: RepositoryResponse = serde_json::from_slice(&response.bytes)
        .map_err(|error| Error::data(format!("Forgejo returned an invalid repository: {error}")))?;
    if args.json {
        Outcome::json(&RepositoryRecord {
            kind: "repo",
            name: &repository.name,
            full_name: &repository.full_name,
            description: &repository.description,
            private: repository.private,
            archived: repository.archived,
            default_branch: &repository.default_branch,
            html_url: &repository.html_url,
        })
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\t{}\n",
            plain_field(&repository.full_name),
            plain_field(repository.description.as_deref().unwrap_or("")),
            plain_field(&repository.default_branch),
            plain_field(&repository.html_url)
        )))
    }
}
