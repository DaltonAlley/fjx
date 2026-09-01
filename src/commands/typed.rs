use std::fs;
use std::io::{self, Read};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::args::{Args, BodySource, PageArgs};
use crate::context::Context;
use crate::error::Error;
use crate::http::{Client, Response};
use crate::output::Outcome;

const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ITEMS: usize = 1_000;

#[derive(Default)]
pub(super) struct AdvertisedTotal(Option<usize>);

impl AdvertisedTotal {
    pub(super) fn has_next(
        &mut self,
        body: Option<usize>,
        header: Option<usize>,
        collected: usize,
        link_has_next: bool,
        noun: &str,
    ) -> Result<bool, Error> {
        let advertised = match (body, header) {
            (Some(body), Some(header)) if body != header => {
                return Err(Error::data(format!(
                    "{noun} body and header totals disagree"
                )));
            }
            (Some(total), _) | (_, Some(total)) => Some(total),
            (None, None) => None,
        };
        if let Some(advertised) = advertised {
            if self.0.is_some_and(|expected| expected != advertised) {
                return Err(Error::data(format!("{noun} total changed between pages")));
            }
            self.0 = Some(advertised);
        }
        if self.0.is_some_and(|total| collected > total) {
            return Err(Error::data(format!("{noun} exceeds its advertised total")));
        }
        if link_has_next && self.0.is_some_and(|total| collected == total) {
            return Err(Error::data(format!(
                "{noun} reports another page after its advertised total"
            )));
        }
        Ok(link_has_next || self.0.is_some_and(|total| collected < total))
    }
}

pub(crate) struct RepoClient {
    pub(crate) client: Client,
    owner: String,
    repo: String,
}

impl RepoClient {
    pub(crate) fn resolve(args: &Args) -> Result<Self, Error> {
        let context = Context::resolve(args, true, true)?;
        let repo = context
            .repo
            .ok_or_else(|| Error::context("repository is required"))?;
        let token = context
            .token
            .ok_or_else(|| Error::context("token is required"))?;
        Ok(Self {
            client: Client::new(context.host, token),
            owner: repo.owner().to_owned(),
            repo: repo.name().to_owned(),
        })
    }

    pub(crate) fn path(&self, suffix: &str) -> String {
        format!(
            "repos/{}/{}/{}",
            self.owner,
            self.repo,
            suffix.trim_start_matches('/')
        )
    }

    pub(crate) fn get<T: DeserializeOwned>(
        &self,
        relative: &str,
        noun: &str,
    ) -> Result<(T, Response), Error> {
        let response = self.client.send("GET", relative, None)?;
        let value = decode(&response.bytes, noun)?;
        Ok((value, response))
    }

    pub(crate) fn write<T: DeserializeOwned, B: Serialize>(
        &self,
        method: &str,
        relative: &str,
        body: &B,
        noun: &str,
    ) -> Result<T, Error> {
        let bytes = serde_json::to_vec(body)
            .map_err(|error| Error::data(format!("could not encode request: {error}")))?;
        let response = self.client.send(method, relative, Some(&bytes))?;
        decode(&response.bytes, noun)
    }

    pub(crate) fn write_empty<B: Serialize>(
        &self,
        method: &str,
        relative: &str,
        body: &B,
    ) -> Result<(), Error> {
        let bytes = serde_json::to_vec(body)
            .map_err(|error| Error::data(format!("could not encode request: {error}")))?;
        self.client.send(method, relative, Some(&bytes))?;
        Ok(())
    }

    pub(crate) fn dry_run<B: Serialize>(
        &self,
        json: bool,
        method: &'static str,
        relative: &str,
        body: &B,
    ) -> Result<Outcome, Error> {
        let value = serde_json::to_value(body)
            .map_err(|error| Error::data(format!("could not encode request: {error}")))?;
        let url = self.client.host().api_url(relative);
        if json {
            Outcome::json(&RequestRecord {
                kind: "request",
                method,
                url: &url,
                body: &value,
            })
        } else {
            Ok(Outcome::text(format!("{method}\t{url}\t{value}\n")))
        }
    }

    pub(crate) fn dry_run_empty(
        &self,
        json: bool,
        method: &'static str,
        relative: &str,
    ) -> Result<Outcome, Error> {
        let url = self.client.host().api_url(relative);
        if json {
            Outcome::json(&RequestRecord {
                kind: "request",
                method,
                url: &url,
                body: &Value::Null,
            })
        } else {
            Ok(Outcome::text(format!("{method}\t{url}\tnull\n")))
        }
    }
}

#[derive(Serialize)]
struct RequestRecord<'a> {
    kind: &'static str,
    method: &'static str,
    url: &'a str,
    body: &'a Value,
}

pub(crate) fn reject_read_flags(args: &Args, command: &str) -> Result<(), Error> {
    if args.dry_run || args.yes {
        Err(Error::usage(format!(
            "write safety flags are not valid for {command}"
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn reject_yes(args: &Args, command: &str) -> Result<(), Error> {
    if args.yes {
        Err(Error::usage(format!("--yes is not valid for {command}")))
    } else {
        Ok(())
    }
}

pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8], noun: &str) -> Result<T, Error> {
    serde_json::from_slice(bytes)
        .map_err(|error| Error::data(format!("Forgejo returned invalid {noun}: {error}")))
}

pub(crate) fn read_body(source: &BodySource) -> Result<String, Error> {
    match source {
        BodySource::Text(value) => Ok(value.clone()),
        BodySource::File(path) => {
            let mut bytes = Vec::new();
            if path == "-" {
                io::stdin()
                    .take(MAX_BODY_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| Error::local(format!("could not read body: {error}")))?;
            } else {
                fs::File::open(path)
                    .map_err(|error| Error::local(format!("could not open body file: {error}")))?
                    .take(MAX_BODY_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| Error::local(format!("could not read body file: {error}")))?;
            }
            if bytes.len() as u64 > MAX_BODY_BYTES {
                return Err(Error::usage("body file exceeds 16 MiB"));
            }
            String::from_utf8(bytes).map_err(|_| Error::usage("body file must be valid UTF-8"))
        }
    }
}

pub(crate) fn collect_arrays<T: DeserializeOwned>(
    repo: &RepoClient,
    base: &str,
    paging: &PageArgs,
    noun: &str,
) -> Result<Vec<T>, Error> {
    let mut result = Vec::new();
    let mut page = paging.page;
    let mut total = AdvertisedTotal::default();
    loop {
        let separator = if base.contains('?') { '&' } else { '?' };
        let path = format!("{base}{separator}page={page}&limit={}", paging.limit);
        let (values, response): (Vec<T>, _) = repo.get(&path, noun)?;
        let collected = result
            .len()
            .checked_add(values.len())
            .ok_or_else(|| Error::data(format!("{noun} count overflow")))?;
        let has_next = total.has_next(
            None,
            response.total_count,
            collected,
            response.has_next,
            noun,
        )?;
        if collected > MAX_ITEMS || (collected == MAX_ITEMS && has_next) {
            return Err(Error::data(format!("{noun} exceeds 1,000 items")));
        }
        let received = values.len();
        result.extend(values);
        if paging.all && has_next && received == 0 {
            return Err(Error::data(format!(
                "{noun} is empty but reports another page"
            )));
        }
        if !paging.all || !has_next || received == 0 {
            break;
        }
        page = page
            .checked_add(1)
            .ok_or_else(|| Error::data("page number overflow"))?;
    }
    Ok(result)
}

pub(crate) fn encode_path(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_path;

    #[test]
    fn encodes_ref_as_one_path_part() {
        assert_eq!(encode_path("feature/a b"), "feature%2Fa%20b");
    }
}
