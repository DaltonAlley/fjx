use std::io::Read;
use std::time::Duration;

use crate::config::Token;
use crate::error::Error;
use crate::host::Host;
use crate::output::plain_field;

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 64 * 1024;
const MAX_ERROR_MESSAGE_CHARS: usize = 300;

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
        self.response(result)
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
        self.response(result)
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        self.agent
            .request(method, url)
            .set("Authorization", &format!("token {}", self.token.as_str()))
            .set("Accept", "application/json")
            .set("User-Agent", concat!("fjx/", env!("CARGO_PKG_VERSION")))
    }

    fn response(&self, result: Result<ureq::Response, ureq::Error>) -> Result<Response, Error> {
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                let detail = error_message(response, self.token.as_str());
                return Err(Error::api(match detail {
                    Some(message) => format!("Forgejo returned HTTP {status}: {message}"),
                    None => format!("Forgejo returned HTTP {status}"),
                }));
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

/// Extract Forgejo's JSON `message` from an error response. The read is bounded,
/// the text is truncated, the request token is redacted, and control characters
/// use the reversible plain-output encoding so a server cannot drive the terminal.
fn error_message(response: ureq::Response, token: &str) -> Option<String> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_ERROR_BODY_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    format_error_message(&bytes, token)
}

fn format_error_message(bytes: &[u8], token: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let message = value.get("message")?.as_str()?.trim();
    if message.is_empty() {
        return None;
    }
    let message = if token.is_empty() {
        message.to_owned()
    } else {
        message.replace(token, "[redacted]")
    };
    let mut bounded: String = message.chars().take(MAX_ERROR_MESSAGE_CHARS).collect();
    if bounded.len() < message.len() {
        bounded.push_str("...");
    }
    Some(plain_field(&bounded).to_string())
}

#[cfg(test)]
mod tests {
    use super::{MAX_ERROR_MESSAGE_CHARS, format_error_message};

    #[test]
    fn error_message_is_bounded_redacted_and_escaped() {
        assert_eq!(
            format_error_message(br#"{"message":"bad\u001b[31m token-1\nnext"}"#, "token-1"),
            Some("bad\\u{1B}[31m [redacted]\\nnext".to_owned())
        );
        let long = format!(r#"{{"message":"{}"}}"#, "x".repeat(1_000));
        let message = format_error_message(long.as_bytes(), "t").unwrap_or_default();
        assert_eq!(message.len(), MAX_ERROR_MESSAGE_CHARS + 3);
        assert!(message.ends_with("..."));
        for body in [
            &b"not json"[..],
            br#"{"message":""}"#,
            br#"{"message":4}"#,
            b"[]",
        ] {
            assert_eq!(format_error_message(body, "t"), None);
        }
    }
}
