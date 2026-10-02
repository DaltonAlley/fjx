use std::fmt::Write;

use serde_json::{Value, json};

use crate::args::{EditArgs, MetadataArgs, PageArgs};
use crate::error::Error;
use crate::output::Outcome;

use super::typed::{RepoClient, collect_arrays, read_body};

pub(crate) struct PlannedRequest {
    pub(crate) method: &'static str,
    pub(crate) path: String,
    pub(crate) body: Value,
}

pub(crate) fn resolve_assignees(repo: &RepoClient, names: &[String]) -> Result<Vec<String>, Error> {
    let login = if names.iter().any(|name| name == "@me") {
        let (user, _): (Value, _) = repo.get("user", "current user")?;
        Some(
            user["login"]
                .as_str()
                .filter(|login| !login.is_empty())
                .ok_or_else(|| Error::data("current user has no login"))?
                .to_owned(),
        )
    } else {
        None
    };
    let mut result = Vec::new();
    for name in names {
        let resolved = if name == "@me" {
            login
                .as_ref()
                .ok_or_else(|| Error::data("current user is unavailable"))?
        } else {
            name
        };
        if !result.contains(resolved) {
            result.push(resolved.clone());
        }
    }
    Ok(result)
}

fn lookup(
    repo: &RepoClient,
    suffix: &str,
    field: &str,
    names: &[String],
) -> Result<Vec<u64>, Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<Value> = collect_arrays(
        repo,
        &repo.path(suffix),
        &PageArgs {
            page: 1,
            all: true,
            limit: 50,
        },
        suffix,
    )?;
    names.iter().map(|name| {
        let matches: Vec<_> = values.iter().filter(|value| value[field].as_str() == Some(name)).collect();
        match matches.as_slice() {
            [value] => value["id"].as_u64().filter(|id| *id > 0).ok_or_else(|| Error::data(format!("{suffix} entry has invalid ID"))),
            [] => Err(Error::usage(if suffix == "labels" { format!("no applicable label named {name:?}, use an explicit ID for organization labels") } else { format!("no milestone named {name:?}") })),
            _ => Err(Error::usage(format!("ambiguous {suffix} name {name:?}, use an explicit ID"))),
        }
    }).collect()
}

pub(crate) fn resolve_labels(
    repo: &RepoClient,
    names: &[String],
    ids: &[u64],
) -> Result<Vec<u64>, Error> {
    let mut result = ids.to_vec();
    result.extend(lookup(repo, "labels", "name", names)?);
    if result.contains(&0) {
        return Err(Error::usage("label IDs must be positive"));
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

pub(crate) fn resolve_milestone(
    repo: &RepoClient,
    name: Option<&str>,
    id: Option<u64>,
) -> Result<Option<u64>, Error> {
    if name.is_some() && id.is_some() {
        return Err(Error::usage("milestone name and ID are mutually exclusive"));
    }
    if id == Some(0) {
        return Err(Error::usage("milestone IDs must be positive"));
    }
    if let Some(name) = name {
        return lookup(repo, "milestones?state=all", "title", &[name.to_owned()])
            .map(|ids| ids.first().copied());
    }
    Ok(id)
}

pub(crate) fn resolve_metadata(repo: &RepoClient, args: &MetadataArgs) -> Result<Value, Error> {
    let labels = resolve_labels(repo, &args.labels, &args.label_ids)?;
    let milestone = resolve_milestone(repo, args.milestone.as_deref(), args.milestone_id)?;
    let assignees = resolve_assignees(repo, &args.assignees)?;
    let mut value = json!({});
    if !labels.is_empty() {
        value["labels"] = json!(labels);
    }
    if !assignees.is_empty() {
        value["assignees"] = json!(assignees);
    }
    if let Some(id) = milestone {
        value["milestone"] = json!(id);
    }
    Ok(value)
}

fn edit_labels(repo: &RepoClient, edit: &EditArgs) -> Result<(Vec<u64>, Vec<u64>), Error> {
    let mut names = edit.add_labels.clone();
    names.extend(edit.remove_labels.clone());
    let ids = lookup(repo, "labels", "name", &names)?;
    let mut add = edit.add_label_ids.clone();
    let mut remove = edit.remove_label_ids.clone();
    add.extend(&ids[..edit.add_labels.len()]);
    remove.extend(&ids[edit.add_labels.len()..]);
    if add.contains(&0) || remove.contains(&0) {
        return Err(Error::usage("label IDs must be positive"));
    }
    add.sort_unstable();
    add.dedup();
    remove.sort_unstable();
    remove.dedup();
    Ok((add, remove))
}

pub(crate) fn plan_edit(
    repo: &RepoClient,
    number: u64,
    edit: &EditArgs,
) -> Result<Vec<PlannedRequest>, Error> {
    if edit.clear_milestone && (edit.milestone.is_some() || edit.milestone_id.is_some()) {
        return Err(Error::usage("cannot set and clear milestone together"));
    }
    let (add, remove) = edit_labels(repo, edit)?;
    if add.iter().any(|id| remove.contains(id)) {
        return Err(Error::usage("cannot add and remove the same label"));
    }
    let milestone = resolve_milestone(repo, edit.milestone.as_deref(), edit.milestone_id)?;
    let mut body = json!({});
    if let Some(title) = &edit.title {
        body["title"] = json!(title);
    }
    if let Some(source) = &edit.body {
        body["body"] = json!(read_body(source)?);
    }
    if let Some(base) = &edit.base {
        body["base"] = json!(base);
    }
    if let Some(id) = milestone {
        body["milestone"] = json!(id);
    }
    if edit.clear_milestone {
        body["milestone"] = json!(0);
    }
    if !edit.add_assignees.is_empty() || !edit.remove_assignees.is_empty() {
        // Forgejo only offers replacement here. Concurrent assignment changes can race this read-modify-write.
        let mut names = edit.add_assignees.clone();
        names.extend(edit.remove_assignees.clone());
        let me = if names.iter().any(|name| name == "@me") {
            resolve_assignees(repo, &["@me".to_owned()])?
                .first()
                .cloned()
        } else {
            None
        };
        let resolve = |name: &String| {
            if name == "@me" {
                me.as_ref().unwrap_or(name).clone()
            } else {
                name.clone()
            }
        };
        let adds: Vec<_> = edit.add_assignees.iter().map(resolve).collect();
        let removes: Vec<_> = edit.remove_assignees.iter().map(resolve).collect();
        if adds.iter().any(|name| removes.contains(name)) {
            return Err(Error::usage("cannot add and remove the same assignee"));
        }
        let (current, _): (Value, _) =
            repo.get(&repo.path(&format!("issues/{number}")), "issue assignees")?;
        let empty = Vec::new();
        let current_assignees = if current["assignees"].is_null() {
            &empty
        } else {
            current["assignees"]
                .as_array()
                .ok_or_else(|| Error::data("issue assignees are invalid"))?
        };
        let mut assignees: Vec<String> = current_assignees
            .iter()
            .map(|user| {
                user["login"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::data("assignee login is invalid"))
            })
            .collect::<Result<_, _>>()?;
        assignees.retain(|name| !removes.contains(name));
        for name in adds {
            if !assignees.contains(&name) {
                assignees.push(name);
            }
        }
        body["assignees"] = json!(assignees);
    }
    let mut plan = Vec::new();
    if body.as_object().is_some_and(|body| !body.is_empty()) {
        plan.push(PlannedRequest {
            method: "PATCH",
            path: repo.path(&format!("issues/{number}")),
            body,
        });
    }
    if !add.is_empty() {
        plan.push(PlannedRequest {
            method: "POST",
            path: repo.path(&format!("issues/{number}/labels")),
            body: json!({"labels":add}),
        });
    }
    for id in remove {
        plan.push(PlannedRequest {
            method: "DELETE",
            path: repo.path(&format!("issues/{number}/labels/{id}")),
            body: Value::Null,
        });
    }
    if plan.is_empty() {
        return Err(Error::usage("edit requires at least one change"));
    }
    Ok(plan)
}

pub(crate) fn execute_plan(repo: &RepoClient, plan: &[PlannedRequest]) -> Result<(), Error> {
    for (index, request) in plan.iter().enumerate() {
        let bytes =
            serde_json::to_vec(&request.body).map_err(|error| Error::data(error.to_string()))?;
        let body = if request.body.is_null() {
            None
        } else {
            Some(bytes.as_slice())
        };
        if let Err(error) = repo.client.send(request.method, &request.path, body) {
            if index > 0 {
                eprintln!(
                    "partial update: {index} prior request(s) succeeded, changes are not atomic"
                );
                for done in &plan[..index] {
                    eprintln!("succeeded: {} {}", done.method, done.path);
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn dry_run_plan(
    repo: &RepoClient,
    json: bool,
    plan: &[PlannedRequest],
) -> Result<Outcome, Error> {
    let records: Vec<_> = plan.iter().map(|request| json!({"kind":"request","method":request.method,"url":repo.client.host().api_url(&request.path),"body":request.body})).collect();
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}",
                record["method"].as_str().unwrap_or_default(),
                record["url"].as_str().unwrap_or_default(),
                record["body"]
            )
            .map_err(|error| Error::data(error.to_string()))?;
        }
        Ok(Outcome::text(text))
    }
}
