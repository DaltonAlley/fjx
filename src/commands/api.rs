use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};

use serde::Serialize;
use serde_json::Value;

use crate::args::{ApiArgs, Args};
use crate::context::Context;
use crate::error::Error;
use crate::http::Client;
use crate::output::Outcome;

use super::typed::AdvertisedTotal;

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PAGED_ITEMS: usize = 1_000;

#[derive(Clone, Copy, Debug)]
enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    fn parse(value: Option<&str>, has_input: bool) -> Result<Self, Error> {
        match value {
            None if has_input => Ok(Self::Post),
            None | Some("GET") => Ok(Self::Get),
            Some("POST") => Ok(Self::Post),
            Some("PUT") => Ok(Self::Put),
            Some("PATCH") => Ok(Self::Patch),
            Some("DELETE") => Ok(Self::Delete),
            Some(_) => Err(Error::usage("-X must be GET, POST, PUT, PATCH, or DELETE")),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    const fn is_write(self) -> bool {
        !matches!(self, Self::Get)
    }
}

#[derive(Serialize)]
struct RequestRecord<'a> {
    kind: &'static str,
    method: &'static str,
    url: &'a str,
    body: Option<&'a Value>,
}

pub(crate) fn run(common: &Args, args: &ApiArgs) -> Result<Outcome, Error> {
    let path = RawPath::parse(&args.path)?;
    let method = Method::parse(args.method.as_deref(), args.input.is_some())?;
    if args.paginate && !matches!(method, Method::Get) {
        return Err(Error::usage("--paginate requires GET"));
    }
    if common.dry_run && !method.is_write() {
        return Err(Error::usage("--dry-run requires a write request"));
    }
    if matches!(method, Method::Delete) && !common.yes {
        return Err(Error::safety("raw DELETE requires --yes"));
    }
    if common.yes && !matches!(method, Method::Delete) {
        return Err(Error::usage("--yes is only valid for raw DELETE"));
    }
    let body = args.input.as_ref().map(read_json).transpose()?;
    let context = Context::resolve(common, false, true)?;
    let token = context
        .token
        .ok_or_else(|| Error::context("token is required"))?;
    let client = Client::new(context.host, token);
    if common.dry_run {
        let url = client.host().api_url(path.as_str());
        return dry_run(common.json, method, &url, body.as_ref());
    }
    let body_bytes = body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| Error::usage(format!("invalid JSON input: {error}")))?;
    if args.paginate {
        paginate(&client, &path, common.json)
    } else {
        let response = client.send(method.as_str(), path.as_str(), body_bytes.as_deref())?;
        render_response(response.bytes, common.json)
    }
}

fn paginate(client: &Client, path: &RawPath, compact: bool) -> Result<Outcome, Error> {
    let mut joined = Vec::new();
    let mut page = 1_usize;
    let mut total = AdvertisedTotal::default();
    loop {
        let paged = path.with_page(page, 50);
        let response = client.send("GET", &paged, None)?;
        let values: Vec<Value> = serde_json::from_slice(&response.bytes).map_err(|error| {
            Error::data(format!("paged API response is not a JSON array: {error}"))
        })?;
        let collected = joined
            .len()
            .checked_add(values.len())
            .ok_or_else(|| Error::data("paged API response count overflow"))?;
        let has_next = total.has_next(
            None,
            response.total_count,
            collected,
            response.has_next,
            "paged API response",
        )?;
        if collected > MAX_PAGED_ITEMS || (collected == MAX_PAGED_ITEMS && has_next) {
            return Err(Error::data("paged API response exceeds 1,000 items"));
        }
        let received = values.len();
        joined.extend(values);
        if has_next && received == 0 {
            return Err(Error::data(
                "paged API response is empty but reports another page",
            ));
        }
        if !has_next || received == 0 {
            break;
        }
        page += 1;
    }
    Outcome::raw_json(&Value::Array(joined), compact)
}

fn render_response(bytes: Vec<u8>, compact: bool) -> Result<Outcome, Error> {
    if bytes.is_empty() && compact {
        return Outcome::json(&Value::Null);
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => Outcome::raw_json(&value, compact),
        Err(_) if compact => Err(Error::data("Forgejo response is not valid JSON")),
        Err(_) => Ok(Outcome::text(bytes)),
    }
}

fn dry_run(json: bool, method: Method, url: &str, body: Option<&Value>) -> Result<Outcome, Error> {
    if json {
        Outcome::json(&RequestRecord {
            kind: "request",
            method: method.as_str(),
            url,
            body,
        })
    } else {
        let body = body.map_or_else(|| "null".to_owned(), Value::to_string);
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\n",
            method.as_str(),
            url,
            body
        )))
    }
}

fn read_json(path: &OsString) -> Result<Value, Error> {
    let mut bytes = Vec::new();
    if path == "-" {
        io::stdin()
            .take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::local(format!("could not read API input: {error}")))?;
    } else {
        let file = fs::File::open(path)
            .map_err(|error| Error::local(format!("could not open API input: {error}")))?;
        file.take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::local(format!("could not read API input: {error}")))?;
    }
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(Error::usage("API input exceeds 16 MiB"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| Error::usage(format!("invalid JSON input: {error}")))
}

struct RawPath(String);

impl RawPath {
    fn parse(value: &str) -> Result<Self, Error> {
        let value = value.strip_prefix('/').unwrap_or(value);
        let reference_path = value.split_once('?').map_or(value, |(path, _)| path);
        if value.is_empty()
            || reference_path.starts_with('/')
            || value.contains(['#', '\\'])
            || value.bytes().any(|byte| byte.is_ascii_control())
            || value.chars().any(char::is_whitespace)
            || has_absolute_scheme(reference_path)
            || reference_path.contains("://")
            || reference_path.split('/').any(|segment| {
                segment.is_empty() || decoded_dot_segment(segment) || encoded_slash(segment)
            })
        {
            return Err(Error::usage(
                "api PATH must be a safe relative path below /api/v1",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn with_page(&self, page: usize, limit: usize) -> String {
        let (path, query) = self.0.split_once('?').unwrap_or((&self.0, ""));
        let kept = query
            .split('&')
            .filter(|part| !part.is_empty())
            .filter(|part| {
                let key = part.split_once('=').map_or(*part, |value| value.0);
                key != "page" && key != "limit"
            })
            .collect::<Vec<_>>()
            .join("&");
        if kept.is_empty() {
            format!("{path}?page={page}&limit={limit}")
        } else {
            format!("{path}?{kept}&page={page}&limit={limit}")
        }
    }
}

fn has_absolute_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn decoded_dot_segment(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "." | ".." | "%2e" | "%2e%2e" | ".%2e" | "%2e."
    )
}

fn encoded_slash(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.contains("%2f") || lower.contains("%5c")
}

#[cfg(test)]
mod tests {
    use super::RawPath;

    #[test]
    fn accepts_relative_api_paths() {
        for path in [
            "version",
            "/repos/o/r",
            "repos/o/r/issues?state=open",
            "repos/o/alpha:value",
            "repos/o/r?target=alpha:opaque",
            "repos/o/r?callback=https://example.com/x",
        ] {
            assert!(RawPath::parse(path).is_ok(), "{path}");
        }
    }

    #[test]
    fn rejects_escape_paths() {
        for path in [
            "https://other/api/v1/user",
            "//other/user",
            "../user",
            "%2e%2e/user",
            "repos/%2E./user",
            "user#part",
            "repos\\user",
            "repos/o/r\nnext",
            "repos/o/r?value=bad space",
            "repos/o/r\u{7f}next",
        ] {
            assert!(RawPath::parse(path).is_err(), "{path}");
        }
    }

    #[test]
    fn rejects_rfc3986_scheme_prefixes() {
        for path in [
            "alpha:opaque",
            "https:attacker.example/steal",
            "alpha://other/user",
            "a0+-.Z:opaque",
            "MiXeD:opaque",
            "MiXeD://other/user",
            "/Custom+V1.2-value:opaque",
        ] {
            assert!(RawPath::parse(path).is_err(), "{path}");
        }
    }
}
