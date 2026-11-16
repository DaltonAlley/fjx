use std::io::Read;
use std::time::Duration;

use crate::config::Token;
use crate::error::Error;
use crate::host::Host;
use crate::output::plain_field;

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ERROR_BODY_BYTES: u64 = 64 * 1024;
const MAX_ERROR_MESSAGE_CHARS: usize = 300;

// Set only by the MCP worker. Ordinary CLI calls retain their existing policy.
#[derive(Clone)]
pub(crate) struct RequestControl {
    pub(crate) cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) deadline: std::time::Instant,
    pub(crate) secrets: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

thread_local! {
    static CONTROL: std::cell::RefCell<Option<RequestControl>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn controlled<T>(control: RequestControl, run: impl FnOnce() -> T) -> T {
    CONTROL.with(|slot| *slot.borrow_mut() = Some(control));
    let result = run();
    CONTROL.with(|slot| *slot.borrow_mut() = None);
    result
}

pub(crate) fn has_request_control() -> bool {
    CONTROL.with(|slot| slot.borrow().is_some())
}

fn timeout(default: Duration) -> Duration {
    CONTROL.with(|slot| {
        slot.borrow().as_ref().map_or(default, |control| {
            control
                .deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_secs(15))
                .max(Duration::from_millis(1))
        })
    })
}

pub(crate) fn checkpoint() -> Result<(), Error> {
    CONTROL.with(|slot| {
        if let Some(control) = slot.borrow().as_ref() {
            if control.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(Error::network("MCP request cancelled"));
            }
            if std::time::Instant::now() >= control.deadline {
                return Err(Error::network("MCP request deadline exceeded"));
            }
        }
        Ok(())
    })
}

#[derive(serde::Deserialize)]
pub(crate) struct OAuthGrant {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: u64,
    token_type: String,
}

/// OAuth errors deliberately omit server bodies: they can echo codes or tokens.
pub(crate) fn oauth_token(host: &Host, body: &serde_json::Value) -> Result<OAuthGrant, Error> {
    checkpoint()?;
    let bytes =
        serde_json::to_vec(body).map_err(|_| Error::local("could not encode OAuth request"))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(timeout(Duration::from_mins(1)))
        .build();
    let result = agent
        .post(&host.web_url("login/oauth/access_token"))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .set("User-Agent", concat!("fjx/", env!("CARGO_PKG_VERSION")))
        .send_bytes(&bytes);
    let response = match result {
        Ok(response) if (200..300).contains(&response.status()) => response,
        Ok(response) | Err(ureq::Error::Status(_, response)) => {
            return Err(Error::api(format!(
                "OAuth token request returned HTTP {}; run fjx auth login --web to sign in again",
                response.status()
            )));
        }
        Err(ureq::Error::Transport(_)) => {
            return Err(Error::network(
                "OAuth token request failed; it was not retried",
            ));
        }
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_ERROR_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::network("could not read OAuth token response"))?;
    if bytes.len() as u64 > MAX_ERROR_BODY_BYTES {
        return Err(Error::data("OAuth token response exceeds 64 KiB"));
    }
    checkpoint()?;
    let grant: OAuthGrant =
        serde_json::from_slice(&bytes).map_err(|_| Error::data("invalid OAuth token response"))?;
    if !grant.token_type.eq_ignore_ascii_case("bearer") || grant.expires_in == 0 {
        return Err(Error::data(
            "OAuth response requires a Bearer token and positive expires_in",
        ));
    }
    Ok(grant)
}

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
        CONTROL.with(|slot| {
            if let Some(control) = slot.borrow().as_ref()
                && let Ok(mut secrets) = control.secrets.lock()
            {
                secrets.push(token.as_str().to_owned());
            }
        });
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
        checkpoint()?;
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
        let request = self.agent.request(method, url);
        let request = if has_request_control() {
            request.timeout(timeout(Duration::from_secs(15)))
        } else {
            request
        };
        request
            .set(
                "Authorization",
                &format!("{} {}", self.token.scheme(), self.token.as_str()),
            )
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
        checkpoint()?;
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
        checkpoint()?;
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
    fn request_control_bounds_time_and_observes_cancellation() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::{Duration, Instant};
        let control = super::RequestControl {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(30),
            secrets: Arc::new(Mutex::new(Vec::new())),
        };
        assert!(!super::has_request_control());
        super::controlled(control.clone(), || {
            assert!(super::checkpoint().is_ok());
            assert!(super::timeout(Duration::from_mins(1)) <= Duration::from_secs(15));
            control.cancelled.store(true, Ordering::Relaxed);
            assert!(super::checkpoint().is_err());
        });
        assert!(!super::has_request_control());
        assert_eq!(
            super::timeout(Duration::from_mins(1)),
            Duration::from_mins(1)
        );
        let expired = super::RequestControl {
            deadline: Instant::now(),
            ..control
        };
        expired.cancelled.store(false, Ordering::Relaxed);
        super::controlled(expired, || assert!(super::checkpoint().is_err()));
        assert!(super::checkpoint().is_ok());
    }

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
