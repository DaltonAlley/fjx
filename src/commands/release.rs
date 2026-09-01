use std::ffi::OsString;
use std::fmt::Write;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::args::{Args, BodySource, PageArgs, ReleaseArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{
    RepoClient, collect_arrays, encode_path, read_body, reject_read_flags, reject_yes,
};

#[derive(Deserialize)]
struct ReleaseResponse {
    id: u64,
    tag_name: String,
    name: String,
    body: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    target_commitish: String,
    created_at: String,
    published_at: Option<String>,
}

#[derive(Serialize)]
struct ReleaseRecord {
    kind: &'static str,
    id: u64,
    tag: String,
    title: String,
    body: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    target: String,
    created_at: String,
    published_at: Option<String>,
}

impl From<ReleaseResponse> for ReleaseRecord {
    fn from(value: ReleaseResponse) -> Self {
        Self {
            kind: "release",
            id: value.id,
            tag: value.tag_name,
            title: value.name,
            body: value.body,
            html_url: value.html_url,
            draft: value.draft,
            prerelease: value.prerelease,
            target: value.target_commitish,
            created_at: value.created_at,
            published_at: value.published_at,
        }
    }
}

#[derive(Serialize)]
struct CreateRelease<'a> {
    tag_name: &'a str,
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_commitish: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    draft: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    prerelease: bool,
}

#[derive(Deserialize)]
struct AttachmentResponse {
    id: u64,
    name: String,
    size: u64,
    browser_download_url: String,
    created_at: String,
}

#[derive(Serialize)]
struct AttachmentRecord {
    kind: &'static str,
    id: u64,
    name: String,
    size: u64,
    browser_download_url: String,
    created_at: String,
}

impl From<AttachmentResponse> for AttachmentRecord {
    fn from(value: AttachmentResponse) -> Self {
        Self {
            kind: "release_asset",
            id: value.id,
            name: value.name,
            size: value.size,
            browser_download_url: value.browser_download_url,
            created_at: value.created_at,
        }
    }
}

#[derive(Serialize)]
struct UploadPlan<'a> {
    kind: &'static str,
    method: &'static str,
    url: &'a str,
    body: UploadBody<'a>,
}

#[derive(Serialize)]
struct UploadBody<'a> {
    path: String,
    name: &'a str,
    size: u64,
}

pub(crate) fn run(common: &Args, args: &ReleaseArgs) -> Result<Outcome, Error> {
    match args {
        ReleaseArgs::List { paging } => list(common, paging),
        ReleaseArgs::View { tag } => view(common, tag),
        ReleaseArgs::Create {
            tag,
            title,
            body,
            target,
            draft,
            prerelease,
        } => create(
            common,
            tag,
            title,
            body.as_ref(),
            target.as_deref(),
            *draft,
            *prerelease,
        ),
        ReleaseArgs::Upload { id, path, name } => upload(common, *id, path, name.as_deref()),
    }
}

fn list(common: &Args, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "release list")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("releases");
    let values = collect_arrays(&repo, &path, paging, "release list")?;
    render_list(common.json, values)
}

fn view(common: &Args, tag: &str) -> Result<Outcome, Error> {
    reject_read_flags(common, "release view")?;
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("releases/tags/{}", encode_path(tag)));
    let (value, _) = repo.get(&path, "release")?;
    render_one(common.json, value)
}

#[allow(
    clippy::too_many_arguments,
    reason = "fields are the release creation contract"
)]
fn create(
    common: &Args,
    tag: &str,
    title: &str,
    source: Option<&BodySource>,
    target: Option<&str>,
    draft: bool,
    prerelease: bool,
) -> Result<Outcome, Error> {
    reject_yes(common, "release create")?;
    let body = source.map(read_body).transpose()?;
    let payload = CreateRelease {
        tag_name: tag,
        name: title,
        body: body.as_deref(),
        target_commitish: target,
        draft,
        prerelease,
    };
    let repo = RepoClient::resolve(common)?;
    let path = repo.path("releases");
    if common.dry_run {
        return repo.dry_run(common.json, "POST", &path, &payload);
    }
    let value = repo.write("POST", &path, &payload, "release")?;
    render_one(common.json, value)
}

fn upload(
    common: &Args,
    id: u64,
    file_path: &OsString,
    requested_name: Option<&str>,
) -> Result<Outcome, Error> {
    reject_yes(common, "release upload")?;
    let metadata = fs::metadata(file_path)
        .map_err(|error| Error::local(format!("could not inspect release asset: {error}")))?;
    if !metadata.is_file() {
        return Err(Error::usage("release asset path must name a regular file"));
    }
    let default_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| Error::usage("release asset name must be valid UTF-8"))?;
    let name = requested_name.unwrap_or(default_name);
    let repo = RepoClient::resolve(common)?;
    let path = repo.path(&format!("releases/{id}/assets?name={}", encode_path(name)));
    if common.dry_run {
        let url = repo.client.host().api_url(&path);
        if common.json {
            return Outcome::json(&UploadPlan {
                kind: "request",
                method: "POST",
                url: &url,
                body: UploadBody {
                    path: file_path.to_string_lossy().into_owned(),
                    name,
                    size: metadata.len(),
                },
            });
        }
        let body = serde_json::to_string(&UploadBody {
            path: file_path.to_string_lossy().into_owned(),
            name,
            size: metadata.len(),
        })
        .map_err(|error| Error::data(format!("could not encode upload plan: {error}")))?;
        return Ok(Outcome::text(format!(
            "POST\t{}\t{}\n",
            plain_field(&url),
            body
        )));
    }
    let file = fs::File::open(file_path)
        .map_err(|error| Error::local(format!("could not open release asset: {error}")))?;
    let response = repo.client.send_stream(
        "POST",
        &path,
        "application/octet-stream",
        metadata.len(),
        file,
    )?;
    let asset: AttachmentResponse = super::typed::decode(&response.bytes, "release asset")?;
    render_asset(common.json, &AttachmentRecord::from(asset))
}

fn render_list(json: bool, values: Vec<ReleaseResponse>) -> Result<Outcome, Error> {
    let records: Vec<ReleaseRecord> = values.into_iter().map(ReleaseRecord::from).collect();
    if json {
        Outcome::json(&records)
    } else {
        let mut text = String::new();
        for record in records {
            writeln!(
                &mut text,
                "{}\t{}\t{}\t{}\t{}\t{}",
                record.id,
                plain_field(&record.tag),
                plain_field(&record.title),
                record.draft,
                record.prerelease,
                plain_field(&record.html_url)
            )
            .map_err(|error| Error::data(format!("could not format release output: {error}")))?;
        }
        Ok(Outcome::text(text))
    }
}

fn render_one(json: bool, value: ReleaseResponse) -> Result<Outcome, Error> {
    let record = ReleaseRecord::from(value);
    if json {
        Outcome::json(&record)
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            record.id,
            plain_field(&record.tag),
            plain_field(&record.title),
            record.draft,
            record.prerelease,
            plain_field(&record.html_url)
        )))
    }
}

fn render_asset(json: bool, asset: &AttachmentRecord) -> Result<Outcome, Error> {
    if json {
        Outcome::json(&asset)
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\t{}\n",
            asset.id,
            plain_field(&asset.name),
            asset.size,
            plain_field(&asset.browser_download_url)
        )))
    }
}
