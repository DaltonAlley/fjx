use std::io::Read;
use std::time::Duration;

use crate::config::Token;
use crate::error::Error;
use crate::host::Host;

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct Client {
    agent: ureq::Agent,
    host: Host,
    token: Token,
}

pub(crate) struct Response {
    pub(crate) bytes: Vec<u8>,
    pub(crate) has_next: bool,
    pub(crate) total_count: Option<usize>,
}

impl Client {
    pub(crate) fn new(host: Host, token: Token) -> Self {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_mins(1))
            .timeout_write(Duration::from_mins(1))
            .build();
        Self { agent, host, token }
    }

    pub(crate) fn host(&self) -> &Host {
        &self.host
    }

    pub(crate) fn send(
        &self,
        method: &str,
        relative: &str,
        body: Option<&[u8]>,
    ) -> Result<Response, Error> {
        let url = self.host.api_url(relative);
        let request = self.request(method, &url);
        let result = if let Some(bytes) = body {
            request
                .set("Content-Type", "application/json")
                .send_bytes(bytes)
        } else {
            request.call()
        };
        Self::response(result)
    }

    pub(crate) fn send_stream(
        &self,
        method: &str,
        relative: &str,
        content_type: &str,
        content_length: u64,
        reader: impl Read + Send + 'static,
    ) -> Result<Response, Error> {
        let url = self.host.api_url(relative);
        let result = self
            .request(method, &url)
            .set("Content-Type", content_type)
            .set("Content-Length", &content_length.to_string())
            .send(reader);
        Self::response(result)
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        self.agent
            .request(method, url)
            .set("Authorization", &format!("token {}", self.token.as_str()))
            .set("Accept", "application/json")
            .set("User-Agent", concat!("fjx/", env!("CARGO_PKG_VERSION")))
    }

    fn response(result: Result<ureq::Response, ureq::Error>) -> Result<Response, Error> {
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) => {
                return Err(Error::api(format!("Forgejo returned HTTP {status}")));
            }
            Err(ureq::Error::Transport(_)) => {
                return Err(Error::network("Forgejo request failed"));
            }
        };
        if !(200..=299).contains(&response.status()) {
            return Err(Error::api(format!(
                "Forgejo returned HTTP {}",
                response.status()
            )));
        }
        let has_next = response
            .header("Link")
            .is_some_and(|value| value.split(',').any(|part| part.contains("rel=\"next\"")));
        let total_count = response
            .header("x-total-count")
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| Error::data("Forgejo returned an invalid X-Total-Count header"))
            })
            .transpose()?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::network(format!("could not read Forgejo response: {error}")))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(Error::data("Forgejo response exceeds 16 MiB"));
        }
        Ok(Response {
            bytes,
            has_next,
            total_count,
        })
    }
}
