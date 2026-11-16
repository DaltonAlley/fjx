//! Browser authorization for public Forgejo/Gitea clients (RFC 8252 and RFC 7636).
mod crypto;
#[cfg(windows)]
mod windows;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{OAuthCredentials, Token};
use crate::error::Error;
use crate::host::Host;
use crate::http;

const LOGIN_TIMEOUT: Duration = Duration::from_mins(5);

// The public git-credential-oauth application is pre-registered by both forges.
// It needs no client secret. Explicit/environment/saved registrations override it.
pub(crate) const DEFAULT_CLIENT_ID: &str = "a4792ccc-144e-407e-86c9-5e7d8d9c3269";

pub(crate) fn now() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| Error::local("system clock is before the Unix epoch"))
}

pub(crate) fn login(
    host: &Host,
    client_id: &str,
    no_browser: bool,
) -> Result<(Token, OAuthCredentials), Error> {
    crate::config::validate_client_id(client_id)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| Error::local("could not open the OAuth loopback callback"))?;
    let address = listener
        .local_addr()
        .map_err(|_| Error::local("could not read OAuth callback address"))?;
    let redirect_uri = format!("http://{address}/");
    let verifier = crypto::base64url(&random_bytes()?);
    let state = crypto::base64url(&random_bytes()?);
    let challenge = crypto::base64url(&crypto::sha256(verifier.as_bytes()));
    let url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&code_challenge_method=S256&code_challenge={}&state={}",
        host.web_url("login/oauth/authorize"),
        encode(client_id),
        encode(&redirect_uri),
        challenge,
        state
    );
    // Progress is on stderr; stdout remains a single final result, including JSON.
    if client_id == DEFAULT_CLIENT_ID {
        eprintln!(
            "Using the shared git-credential-oauth application. If it is disabled on this server, use --client-id with a registered public application."
        );
    }
    eprintln!("Open this URL to authorize fjx (expires in 5 minutes):\n{url}");
    eprintln!(
        "Use a browser on this computer. For SSH, forward callback port {} or use --with-token.",
        address.port()
    );
    if !no_browser && !open_browser(&url) {
        eprintln!("Could not open a browser; open the URL above manually.");
    }
    let code = wait_for_code(&listener, &address.to_string(), &state, LOGIN_TIMEOUT)?;
    let grant = http::oauth_token(
        host,
        &serde_json::json!({
            "client_id": client_id, "code": code, "grant_type": "authorization_code",
            "redirect_uri": redirect_uri, "code_verifier": verifier,
        }),
    )?;
    credentials(grant, client_id, None)
}

pub(crate) fn refresh(
    host: &Host,
    saved: &OAuthCredentials,
) -> Result<(Token, OAuthCredentials), Error> {
    let grant = http::oauth_token(
        host,
        &serde_json::json!({
            "client_id": saved.client_id, "refresh_token": saved.refresh_token, "grant_type": "refresh_token",
        }),
    )?;
    credentials(grant, &saved.client_id, Some(&saved.refresh_token))
}

fn credentials(
    grant: http::OAuthGrant,
    client_id: &str,
    previous_refresh: Option<&str>,
) -> Result<(Token, OAuthCredentials), Error> {
    let token = Token::oauth(grant.access_token)?;
    let refresh_token = grant
        .refresh_token
        .or_else(|| previous_refresh.map(str::to_owned))
        .ok_or_else(|| Error::data("OAuth response has no refresh token"))?;
    // Validate before persisting; neither parser includes credential values in errors.
    Token::parse(refresh_token.clone())?;
    let expires_at = now()?
        .checked_add(grant.expires_in)
        .ok_or_else(|| Error::data("invalid OAuth token expiry"))?;
    Ok((
        token,
        OAuthCredentials {
            client_id: client_id.to_owned(),
            refresh_token,
            expires_at,
        },
    ))
}

#[cfg(windows)]
fn random_bytes() -> Result<[u8; 32], Error> {
    windows::random_bytes()
}

#[cfg(unix)]
fn random_bytes() -> Result<[u8; 32], Error> {
    let mut bytes = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| Error::local("could not read the system random number generator"))?;
    Ok(bytes)
}

#[cfg(not(any(unix, windows)))]
fn random_bytes() -> Result<[u8; 32], Error> {
    Err(Error::local(
        "OAuth requires a supported system random number generator",
    ))
}

fn open_browser(url: &str) -> bool {
    let mut command = if let Some(browser) = std::env::var_os("FJX_BROWSER") {
        // One executable, without shell interpretation or command-line interpolation.
        Command::new(browser)
    } else if cfg!(windows) {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    let child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // Some openers stay alive as long as the browser; reap without delaying login.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

fn wait_for_code(
    listener: &TcpListener,
    authority: &str,
    state: &str,
    timeout: Duration,
) -> Result<String, Error> {
    listener
        .set_nonblocking(true)
        .map_err(|_| Error::local("could not configure OAuth callback"))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let limit = remaining.min(Duration::from_secs(2));
                if stream.set_read_timeout(Some(limit)).is_err()
                    || stream.set_write_timeout(Some(limit)).is_err()
                {
                    continue;
                }
                if let Some(result) = callback(&mut stream, authority, state) {
                    return result;
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err(Error::local("could not accept OAuth callback")),
        }
    }
    Err(Error::context(
        "browser login timed out after 5 minutes; run fjx auth login again",
    ))
}

fn callback(stream: &mut TcpStream, authority: &str, state: &str) -> Option<Result<String, Error>> {
    let request = read_headers(stream);
    let result = request
        .as_deref()
        .and_then(|request| parse_callback(request, authority, state));
    let (status, message) = match &result {
        Some(Ok(_)) => (
            "200 OK",
            "Authorization received. Return to the terminal to check that fjx completed login.",
        ),
        Some(Err(_)) => (
            "400 Bad Request",
            "Authorization was declined. Return to the terminal.",
        ),
        None => (
            "400 Bad Request",
            "Invalid callback. Return to the authorization page to continue.",
        ),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
        message.len()
    );
    result
}

fn read_headers(stream: &mut TcpStream) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut byte = [0];
    while bytes.len() < 8192 && Instant::now() < deadline {
        if stream.read(&mut byte).ok()? != 1 {
            return None;
        }
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

fn parse_callback(request: &str, authority: &str, state: &str) -> Option<Result<String, Error>> {
    let mut lines = request.split("\r\n");
    let mut first = lines.next()?.split(' ');
    if first.next()? != "GET" {
        return None;
    }
    let target = first.next()?;
    if first.next()? != "HTTP/1.1" || first.next().is_some() {
        return None;
    }
    let mut hosts = lines.filter_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("host").then(|| value.trim())
    });
    if hosts.next()? != authority || hosts.next().is_some() {
        return None;
    }
    let query = target.strip_prefix("/?")?;
    let (mut received_state, mut code, mut error) = (None, None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        let key = decode(key)?;
        let value = decode(value)?;
        let slot = match key.as_str() {
            "state" => &mut received_state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    if received_state.as_deref()? != state {
        return None;
    }
    match (code, error) {
        (Some(code), None)
            if !code.is_empty() && code.len() <= 4096 && !code.chars().any(char::is_control) =>
        {
            Some(Ok(code))
        }
        (None, Some(_)) => Some(Err(Error::context(
            "OAuth authorization was declined or failed",
        ))),
        _ => None,
    }
}

fn encode(value: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn decode(value: &str) -> Option<String> {
    let mut decoded = Vec::new();
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'%' => {
                let hi = char::from(bytes.next()?).to_digit(16)?;
                let lo = char::from(bytes.next()?).to_digit(16)?;
                u8::try_from(hi * 16 + lo).ok()?
            }
            b'+' => b' ',
            other => other,
        });
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_validates_state_host_path_and_duplicates() {
        let request =
            |query: &str| format!("GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n");
        assert!(
            matches!(parse_callback(&request("code=abc%2Bdef&state=secret"), "127.0.0.1:1234", "secret"), Some(Ok(code)) if code == "abc+def")
        );
        assert!(matches!(
            parse_callback(
                &request("error=access_denied&state=secret"),
                "127.0.0.1:1234",
                "secret"
            ),
            Some(Err(_))
        ));
        for query in [
            "code=x",
            "code=x&state=wrong",
            "code=x&state=secret&state=secret",
            "code=x&code=y&state=secret",
            "code=x&error=x&state=secret",
            "code=%0A&state=secret",
            "code=%zz&state=secret",
        ] {
            assert!(parse_callback(&request(query), "127.0.0.1:1234", "secret").is_none());
        }
        assert!(
            parse_callback(&request("code=x&state=secret"), "evil.example", "secret").is_none()
        );
        assert!(
            parse_callback(
                &request("code=x&state=secret").replace("GET /?", "GET /other?"),
                "127.0.0.1:1234",
                "secret"
            )
            .is_none()
        );
        assert!(
            parse_callback(
                &request("code=x&state=secret").replace("Host:", "Host: evil\r\nHost:"),
                "127.0.0.1:1234",
                "secret"
            )
            .is_none()
        );
    }
    #[test]
    fn system_randomness_is_available() {
        let first = random_bytes().unwrap_or_else(|_| panic!("system RNG failed"));
        let second = random_bytes().unwrap_or_else(|_| panic!("system RNG failed"));
        assert_ne!(first, second);
        assert_eq!(crypto::base64url(&first).len(), 43);
    }
    #[test]
    fn callback_timeout_is_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|_| panic!("bind"));
        assert!(wait_for_code(&listener, "unused", "unused", Duration::from_millis(1)).is_err());
    }
}
