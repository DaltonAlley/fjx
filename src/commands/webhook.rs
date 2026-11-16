use std::ffi::OsString;
use std::fmt::Write;
use std::io::{self, Read};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::args::{Args, WebhookArgs};
use crate::error::Error;
use crate::host::Host;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, collect_arrays, encode_path, reject_read_flags, reject_yes};

// Deliberately omit destination URLs, authorization headers and provider metadata:
// all can contain receiver credentials, even in read responses.
#[derive(Deserialize, Serialize)]
struct Hook {
    id: u64,
    #[serde(rename = "type")]
    provider: String,
    active: bool,
    events: Vec<String>,
    #[serde(default)]
    branch_filter: String,
    #[serde(skip_serializing)]
    authorization_header: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Input {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<Config>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    events: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_header: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_method: Option<String>,
}

pub(crate) fn run(common: &Args, args: &WebhookArgs) -> Result<Outcome, Error> {
    run_inner(common, args).map_err(|error| match error.code() {
        // Error bodies and decode errors may echo receiver credentials.
        5 => {
            let message = error.to_string();
            let status = message
                .strip_prefix("Forgejo returned HTTP ")
                .and_then(|s| s.get(..3))
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()));
            Error::api(status.map_or_else(
                || "Forgejo webhook request failed".to_owned(),
                |s| format!("Forgejo returned HTTP {s} for webhook request"),
            ))
        }
        7 => Error::data("invalid or incomplete Forgejo webhook response"),
        _ => error,
    })
}

fn run_inner(common: &Args, args: &WebhookArgs) -> Result<Outcome, Error> {
    match args {
        WebhookArgs::List { paging } => {
            reject_read_flags(common, "webhook list")?;
            let repo = RepoClient::resolve(common)?;
            let hooks: Vec<Hook> = collect_arrays(&repo, &repo.path("hooks"), paging, "webhooks")?;
            if common.json {
                Outcome::json(&hooks)
            } else {
                let mut text = String::new();
                for hook in hooks {
                    text.push_str(&plain_hook(&hook)?);
                }
                Ok(Outcome::text(text))
            }
        }
        WebhookArgs::View { id } => {
            reject_read_flags(common, "webhook view")?;
            let repo = RepoClient::resolve(common)?;
            let (hook, _): (Hook, _) = repo.get(&repo.path(&format!("hooks/{id}")), "webhook")?;
            render(common, &hook)
        }
        WebhookArgs::Create { input } => write(common, None, input),
        WebhookArgs::Edit { id, input } => write(common, Some(*id), input),
        WebhookArgs::Delete { id } => {
            if !common.yes {
                return Err(Error::safety("webhook delete requires --yes"));
            }
            let repo = RepoClient::resolve(common)?;
            let path = repo.path(&format!("hooks/{id}"));
            if common.dry_run {
                return repo.dry_run_empty(common.json, "DELETE", &path);
            }
            repo.client.send("DELETE", &path, None)?;
            result(common, "webhook.delete", *id)
        }
        WebhookArgs::Test { id, reference } => {
            reject_yes(common, "webhook test")?;
            let repo = RepoClient::resolve(common)?;
            let mut path = repo.path(&format!("hooks/{id}/tests"));
            if let Some(reference) = reference {
                write!(&mut path, "?ref={}", encode_path(reference))
                    .map_err(|_| Error::data("could not format test request"))?;
            }
            if common.dry_run {
                return repo.dry_run_empty(common.json, "POST", &path);
            }
            repo.client.send("POST", &path, None)?;
            // Forgejo acknowledges enqueueing a test, not successful delivery.
            result(common, "webhook.test.queued", *id)
        }
    }
}

fn write(common: &Args, id: Option<u64>, path: &OsString) -> Result<Outcome, Error> {
    reject_yes(common, "webhook create/edit")?;
    let mut input = read_input(path)?;
    validate(&mut input, id.is_none())?;
    let repo = RepoClient::resolve(common)?;
    let (method, path) = id.map_or_else(
        || ("POST", repo.path("hooks")),
        |id| ("PATCH", repo.path(&format!("hooks/{id}"))),
    );
    if id.is_some() {
        let (current, _): (Hook, _) = repo.get(&path, "webhook")?;
        preserve_edit(&mut input, &current)?;
    }
    if common.dry_run {
        if let Some(config) = &mut input.config {
            "[redacted]".clone_into(&mut config.url);
            if let Some(secret) = &mut config.secret {
                "[redacted]".clone_into(secret);
            }
        }
        if let Some(header) = &mut input.authorization_header {
            "[redacted]".clone_into(header);
        }
        return repo.dry_run(common.json, method, &path, &input);
    }
    let hook: Hook = repo.write(method, &path, &input, "webhook")?;
    render(common, &hook)
}

fn validate(input: &mut Input, create: bool) -> Result<(), Error> {
    if create {
        let kind = input.provider.get_or_insert_with(|| "forgejo".to_owned());
        if kind != "forgejo" {
            return Err(Error::usage("webhook create supports type forgejo"));
        }
        if input.config.is_none() || input.events.as_ref().is_none_or(Vec::is_empty) {
            return Err(Error::usage(
                "webhook create requires config.url and nonempty events",
            ));
        }
        input.active.get_or_insert(true);
    } else {
        if input.provider.is_some() {
            return Err(Error::usage("webhook edit cannot change type"));
        }
        if input.config.as_ref().is_some_and(|c| c.secret.is_some()) {
            return Err(Error::usage(
                "Forgejo cannot edit a webhook signing secret through this API; create a replacement hook",
            ));
        }
        if input.config.is_none()
            && input.events.is_none()
            && input.active.is_none()
            && input.branch_filter.is_none()
            && input.authorization_header.is_none()
        {
            return Err(Error::usage("webhook edit requires at least one field"));
        }
    }
    if let Some(events) = &input.events {
        let mut seen = std::collections::BTreeSet::new();
        if events.is_empty()
            || events.iter().any(|event| {
                event.is_empty()
                    || !event.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    || !seen.insert(event)
                    || !known_event(event)
            })
        {
            return Err(Error::usage(
                "events must be unique supported Forgejo event names",
            ));
        }
    }
    if input
        .authorization_header
        .as_ref()
        .is_some_and(|s| s.chars().any(char::is_control))
    {
        return Err(Error::usage(
            "authorization_header contains control characters",
        ));
    }
    if let Some(config) = &mut input.config {
        let url = &config.url;
        let base = url.split('?').next().unwrap_or_default();
        if url.contains(['#', '\\'])
            || url.chars().any(char::is_whitespace)
            || url.chars().any(char::is_control)
            || Host::parse(base).is_err()
        {
            return Err(Error::usage(
                "config.url must be HTTPS (HTTP is allowed only for loopback), without user information or fragments",
            ));
        }
        if config.content_type.as_deref().is_some_and(|s| s != "json")
            || config.http_method.as_deref().is_some_and(|s| s != "POST")
        {
            return Err(Error::usage(
                "webhooks require content_type json and http_method POST",
            ));
        }
        config.content_type = Some("json".to_owned());
        config.http_method = Some("POST".to_owned());
    }
    Ok(())
}

fn known_event(event: &str) -> bool {
    matches!(
        event,
        "create"
            | "delete"
            | "fork"
            | "push"
            | "repository"
            | "wiki"
            | "release"
            | "package"
            | "issues"
            | "issues_only"
            | "issue_assign"
            | "issue_label"
            | "issue_milestone"
            | "issue_comment"
            | "pull_request"
            | "pull_request_only"
            | "pull_request_assign"
            | "pull_request_label"
            | "pull_request_milestone"
            | "pull_request_comment"
            | "pull_request_review"
            | "pull_request_review_request"
            | "pull_request_sync"
            | "action_run_failure"
            | "action_run_success"
    ) || ["workflow_run_", "workflow_job_"].iter().any(|prefix| {
        event.strip_prefix(prefix).is_some_and(|state| {
            matches!(
                state,
                "blocked" | "cancelled" | "failure" | "running" | "skipped" | "success" | "waiting"
            )
        })
    })
}

fn preserve_edit(input: &mut Input, current: &Hook) -> Result<(), Error> {
    if current.provider != "forgejo" {
        return Err(Error::usage("webhook edit supports Forgejo hooks only"));
    }
    if let Some(events) = &input.events {
        // Forgejo 17's PATCH handler does not update these event flags.
        let fixed = |events: &[String]| {
            events
                .iter()
                .filter(|e| {
                    e.starts_with("workflow_")
                        || e.starts_with("action_run_")
                        || e.as_str() == "package"
                })
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        if fixed(events) != fixed(&current.events) {
            return Err(Error::usage(
                "Forgejo cannot edit workflow, action-run or package subscriptions through this API; create a replacement hook",
            ));
        }
    } else {
        let mut events = std::collections::BTreeSet::new();
        for event in &current.events {
            // Read responses use broad input aliases for narrower event flags.
            let event = match event.as_str() {
                "issues" => "issues_only",
                "pull_request" => "pull_request_only",
                "pull_request_review_approved"
                | "pull_request_review_rejected"
                | "pull_request_review_comment" => "pull_request_review",
                event => event,
            };
            if !known_event(event) {
                return Err(Error::data("unsupported stored webhook event"));
            }
            events.insert(event.to_owned());
        }
        if events.is_empty() {
            return Err(Error::usage(
                "cannot preserve an empty webhook subscription through this API",
            ));
        }
        input.events = Some(events.into_iter().collect());
    }
    input
        .branch_filter
        .get_or_insert_with(|| current.branch_filter.clone());
    input
        .authorization_header
        .get_or_insert_with(|| current.authorization_header.clone());
    Ok(())
}

fn read_input(path: &OsString) -> Result<Input, Error> {
    const LIMIT: u64 = 1024 * 1024;
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(
            std::fs::File::open(path).map_err(|_| Error::local("could not open webhook input"))?,
        )
    };
    let mut bytes = Vec::new();
    reader
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::local("could not read webhook input"))?;
    if bytes.len() as u64 > LIMIT {
        return Err(Error::usage("webhook input exceeds 1 MiB"));
    }
    // Parse errors can quote credential values, so never include their detail.
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::usage("invalid webhook JSON; see fjx webhook create --help"))
}

fn plain_hook(hook: &Hook) -> Result<String, Error> {
    let mut text = String::new();
    writeln!(
        &mut text,
        "{}\t{}\t{}\t{}\t{}",
        hook.id,
        plain_field(&hook.provider),
        hook.active,
        plain_field(&hook.events.join(",")),
        plain_field(&hook.branch_filter)
    )
    .map_err(|_| Error::data("could not format webhook"))?;
    Ok(text)
}

fn render(common: &Args, hook: &Hook) -> Result<Outcome, Error> {
    if common.json {
        Outcome::json(hook)
    } else {
        Ok(Outcome::text(plain_hook(hook)?))
    }
}

fn result(common: &Args, action: &str, id: u64) -> Result<Outcome, Error> {
    if common.json {
        Outcome::json(&json!({"kind":"result", "action":action, "id":id, "ok":true}))
    } else {
        Ok(Outcome::text(format!("{action}\t{id}\tok\n")))
    }
}
