use std::collections::BTreeMap;
use std::ffi::OsString;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::args::{Args, BodySource, OrgArgs, PageArgs, TeamArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{
    ApiClient, collect_arrays, encode_path, read_body, reject_read_flags, reject_yes,
};

#[derive(Deserialize, Serialize)]
struct Organization {
    id: u64,
    name: String,
    full_name: String,
    description: String,
    email: String,
    website: String,
    location: String,
    visibility: String,
    repo_admin_change_team_access: bool,
}

#[derive(Deserialize, Serialize)]
struct Team {
    id: u64,
    name: String,
    description: String,
    permission: String,
    units_map: BTreeMap<String, String>,
    includes_all_repositories: bool,
    can_create_org_repo: bool,
}

#[derive(Deserialize, Serialize)]
struct Member {
    id: u64,
    login: String,
    full_name: String,
}

#[derive(Deserialize, Serialize)]
struct Repository {
    id: u64,
    full_name: String,
    private: bool,
    html_url: String,
}

trait Summary: Serialize {
    const KIND: &'static str;
    fn plain(&self) -> String;
}

impl Summary for Organization {
    const KIND: &'static str = "organization";
    fn plain(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\n",
            self.id,
            plain_field(&self.name),
            plain_field(&self.visibility),
            plain_field(&self.full_name)
        )
    }
}

impl Summary for Team {
    const KIND: &'static str = "team";
    fn plain(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            self.id,
            plain_field(&self.name),
            plain_field(&self.permission),
            self.includes_all_repositories,
            self.can_create_org_repo,
            plain_field(&self.description)
        )
    }
}

impl Summary for Member {
    const KIND: &'static str = "member";
    fn plain(&self) -> String {
        format!(
            "{}\t{}\t{}\n",
            self.id,
            plain_field(&self.login),
            plain_field(&self.full_name)
        )
    }
}

impl Summary for Repository {
    const KIND: &'static str = "repository";
    fn plain(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\n",
            self.id,
            plain_field(&self.full_name),
            self.private,
            plain_field(&self.html_url)
        )
    }
}

#[derive(Serialize)]
struct Record<T: Summary> {
    kind: &'static str,
    #[serde(flatten)]
    value: T,
}

fn render<T: Summary>(common: &Args, values: Vec<T>, list: bool) -> Result<Outcome, Error> {
    if common.json {
        let records: Vec<_> = values
            .into_iter()
            .map(|value| Record {
                kind: T::KIND,
                value,
            })
            .collect();
        if list {
            Outcome::json(&records)
        } else {
            Outcome::json(&records[0])
        }
    } else {
        Ok(Outcome::text(
            values.iter().map(Summary::plain).collect::<String>(),
        ))
    }
}

fn list<T: Summary + serde::de::DeserializeOwned>(
    common: &Args,
    path: &str,
    paging: &PageArgs,
) -> Result<Outcome, Error> {
    reject_read_flags(common, "organization/team list")?;
    let client = ApiClient::resolve(common)?;
    let values: Vec<T> = collect_arrays(&client, path, paging, T::KIND)?;
    render(common, values, true)
}

fn view<T: Summary + serde::de::DeserializeOwned>(
    common: &Args,
    path: &str,
) -> Result<Outcome, Error> {
    reject_read_flags(common, "organization/team view")?;
    let client = ApiClient::resolve(common)?;
    let (value, _): (T, _) = client.get(path, T::KIND)?;
    render(common, vec![value], false)
}

pub(crate) fn run(common: &Args, args: &OrgArgs) -> Result<Outcome, Error> {
    match args {
        OrgArgs::List { user, paging } => {
            let path = user.as_ref().map_or_else(
                || "user/orgs".to_owned(),
                |user| format!("users/{}/orgs", encode_path(user)),
            );
            list::<Organization>(common, &path, paging)
        }
        OrgArgs::View { org } => view::<Organization>(common, &org_path(org)),
        OrgArgs::Members { org, paging } => {
            list::<Member>(common, &format!("{}/members", org_path(org)), paging)
        }
        OrgArgs::Repos { org, paging } => {
            list::<Repository>(common, &format!("{}/repos", org_path(org)), paging)
        }
        OrgArgs::Create { input } => write_org(common, None, input),
        OrgArgs::Edit { org, input } => write_org(common, Some(org), input),
        OrgArgs::Delete { org } => mutate(common, "DELETE", &org_path(org), "org.delete", org),
        OrgArgs::RemoveMember { org, user } => mutate(
            common,
            "DELETE",
            &format!("{}/members/{}", org_path(org), encode_path(user)),
            "org.remove-member",
            &format!("{org}/{user}"),
        ),
    }
}

fn org_path(org: &str) -> String {
    format!("orgs/{}", encode_path(org))
}

pub(crate) fn run_team(common: &Args, args: &TeamArgs) -> Result<Outcome, Error> {
    match args {
        TeamArgs::List { org, paging } => {
            list::<Team>(common, &format!("{}/teams", org_path(org)), paging)
        }
        TeamArgs::View { id } => view::<Team>(common, &format!("teams/{id}")),
        TeamArgs::Members { id, paging } => {
            list::<Member>(common, &format!("teams/{id}/members"), paging)
        }
        TeamArgs::Repos { id, paging } => {
            list::<Repository>(common, &format!("teams/{id}/repos"), paging)
        }
        TeamArgs::Create { org, input } => {
            write_team(common, &format!("{}/teams", org_path(org)), input, false)
        }
        TeamArgs::Edit { id, input } => write_team(common, &format!("teams/{id}"), input, true),
        TeamArgs::Delete { id } => mutate(
            common,
            "DELETE",
            &format!("teams/{id}"),
            "team.delete",
            &id.to_string(),
        ),
        TeamArgs::AddMember { id, user } | TeamArgs::RemoveMember { id, user } => {
            let add = matches!(args, TeamArgs::AddMember { .. });
            mutate(
                common,
                if add { "PUT" } else { "DELETE" },
                &format!("teams/{id}/members/{}", encode_path(user)),
                if add {
                    "team.add-member"
                } else {
                    "team.remove-member"
                },
                &format!("{id}/{user}"),
            )
        }
        TeamArgs::AddRepo { id, repo } | TeamArgs::RemoveRepo { id, repo } => {
            let add = matches!(args, TeamArgs::AddRepo { .. });
            let repo_path = repo
                .split('/')
                .map(encode_path)
                .collect::<Vec<_>>()
                .join("/");
            mutate(
                common,
                if add { "PUT" } else { "DELETE" },
                &format!("teams/{id}/repos/{repo_path}"),
                if add {
                    "team.add-repo"
                } else {
                    "team.remove-repo"
                },
                &format!("{id}/{repo}"),
            )
        }
    }
}

fn mutate(
    common: &Args,
    method: &'static str,
    path: &str,
    action: &str,
    target: &str,
) -> Result<Outcome, Error> {
    if method == "DELETE" {
        if !common.yes {
            return Err(Error::safety(format!("{action} requires --yes")));
        }
    } else {
        reject_yes(common, action)?;
    }
    let client = ApiClient::resolve(common)?;
    if common.dry_run {
        return client.dry_run_empty(common.json, method, path);
    }
    client.client.send(method, path, None)?;
    if common.json {
        Outcome::json(&json!({"kind":"result", "action":action, "ok":true, "target":target}))
    } else {
        Ok(Outcome::text(format!(
            "{action}\t{}\tok\n",
            plain_field(target)
        )))
    }
}

// Explicit optional fields distinguish omission from clearing. Unknown fields and
// JSON null are rejected before resolving credentials or making any requests.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OrgInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_admin_change_team_access: Option<bool>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TeamInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    units_map: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    includes_all_repositories: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_create_org_repo: Option<bool>,
}

fn read_input<T: serde::de::DeserializeOwned>(path: &OsString) -> Result<T, Error> {
    let text = read_body(&BodySource::File(path.clone()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Error::usage(format!("invalid JSON input: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Error::usage("input must be a JSON object"))?;
    if object.is_empty() || object.values().any(Value::is_null) {
        return Err(Error::usage(
            "input must have at least one field; null values are not supported",
        ));
    }
    serde_json::from_value(value).map_err(|e| Error::usage(format!("invalid input: {e}")))
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::usage(
            "username must contain letters, digits, -, _ or .",
        ));
    }
    Ok(())
}

fn write_org(common: &Args, org: Option<&String>, input: &OsString) -> Result<Outcome, Error> {
    reject_yes(common, "org create/edit")?;
    let mut input: OrgInput = read_input(input)?;
    if let Some(visibility) = &input.visibility
        && !matches!(visibility.as_str(), "public" | "limited" | "private")
    {
        return Err(Error::usage(
            "visibility must be public, limited or private",
        ));
    }
    if org.is_some() {
        if input.username.is_some() {
            return Err(Error::usage("org edit cannot rename an organization"));
        }
    } else {
        let name = input
            .username
            .as_deref()
            .ok_or_else(|| Error::usage("org create input requires username"))?;
        validate_name(name)?;
    }
    let client = ApiClient::resolve(common)?;
    let (method, path) = if let Some(org) = org {
        let path = org_path(org);
        let (current, _): (Organization, _) = client.get(&path, "organization")?;
        // Forgejo 15 overwrites these strings when omitted from PATCH.
        input.full_name.get_or_insert(current.full_name);
        input.description.get_or_insert(current.description);
        input.website.get_or_insert(current.website);
        input.location.get_or_insert(current.location);
        ("PATCH", path)
    } else {
        ("POST", "orgs".to_owned())
    };
    if common.dry_run {
        return client.dry_run(common.json, method, &path, &input);
    }
    let value: Organization = client.write(method, &path, &input, "organization")?;
    render(common, vec![value], false)
}

fn validate_team(input: &TeamInput, edit: bool) -> Result<(), Error> {
    if (!edit && input.name.is_none())
        || input
            .name
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.chars().any(char::is_control))
    {
        return Err(Error::usage(
            "team create requires a nonempty name; team names cannot contain control characters",
        ));
    }
    if let Some(permission) = &input.permission
        && !matches!(permission.as_str(), "read" | "write" | "admin")
    {
        return Err(Error::usage("permission must be read, write or admin"));
    }
    if let Some(units) = &input.units_map {
        if units.is_empty() {
            return Err(Error::usage("units_map must not be empty"));
        }
        for (unit, permission) in units {
            if !matches!(
                unit.as_str(),
                "repo.code"
                    | "repo.issues"
                    | "repo.ext_issues"
                    | "repo.wiki"
                    | "repo.ext_wiki"
                    | "repo.pulls"
                    | "repo.releases"
                    | "repo.projects"
                    | "repo.packages"
                    | "repo.actions"
            ) {
                return Err(Error::usage(format!("unsupported team unit {unit:?}")));
            }
            if !matches!(permission.as_str(), "none" | "read" | "write") {
                return Err(Error::usage(
                    "unit permissions must be none, read or write; use permission admin for repository administration",
                ));
            }
            if matches!(unit.as_str(), "repo.ext_issues" | "repo.ext_wiki") && permission == "write"
            {
                return Err(Error::usage(
                    "external issue/wiki units support only none or read",
                ));
            }
        }
        if input.permission.as_deref() == Some("admin") {
            return Err(Error::usage(
                "admin grants all repository units; omit units_map",
            ));
        }
    }
    if (!edit || input.permission.as_deref().is_some_and(|p| p != "admin"))
        && input.permission.as_deref() != Some("admin")
        && input.units_map.is_none()
    {
        return Err(Error::usage(
            "non-admin team creation or permission changes require an explicit units_map",
        ));
    }
    Ok(())
}

fn write_team(common: &Args, path: &str, source: &OsString, edit: bool) -> Result<Outcome, Error> {
    reject_yes(common, "team create/edit")?;
    let mut input: TeamInput = read_input(source)?;
    validate_team(&input, edit)?;
    let client = ApiClient::resolve(common)?;
    if edit {
        let (current, _): (Team, _) = client.get(path, "team")?;
        if current.permission == "owner"
            && (input.permission.is_some()
                || input.units_map.is_some()
                || input.includes_all_repositories.is_some()
                || input.can_create_org_repo.is_some())
        {
            return Err(Error::safety(
                "Owners team permissions are managed by Forgejo",
            ));
        }
        input.name.get_or_insert(current.name);
        // Forgejo only applies includes_all_repositories and recalculates access
        // when permission is supplied. Preserve it for unrelated edits.
        if input.units_map.is_some() || input.includes_all_repositories.is_some() {
            input.permission.get_or_insert(current.permission);
        }
        if input.units_map.is_some() {
            validate_team(&input, true)?;
        }
    } else {
        input.permission.get_or_insert_with(|| "read".to_owned());
    }
    let method = if edit { "PATCH" } else { "POST" };
    if common.dry_run {
        return client.dry_run(common.json, method, path, &input);
    }
    let value: Team = client.write(method, path, &input, "team")?;
    render(common, vec![value], false)
}
