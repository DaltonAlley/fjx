#![allow(clippy::expect_used, reason = "local OAuth test fixtures")]

#[path = "../src/oauth/crypto.rs"]
mod crypto;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn command(config: &PathBuf) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    for key in [
        "FJX_HOST",
        "FJX_REPO",
        "FJX_TOKEN",
        "FORGEJO_TOKEN",
        "FJX_CLIENT_ID",
        "FJX_BROWSER",
    ] {
        command.env_remove(key);
    }
    command.env("FJX_CONFIG", config).stdin(Stdio::null());
    command
}

struct Fixture {
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fjx-oauth-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).expect("fixture directory");
        Self {
            config: root.join("hosts.json"),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let count = stream.read(&mut buffer).expect("read request");
        assert!(count > 0, "incomplete request");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 65536, "oversized fixture request");
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("UTF8 request")
}

fn server(responses: Vec<(u16, String)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    let host = format!("http://{}/forgejo", listener.local_addr().expect("address"));
    listener.set_nonblocking(true).expect("nonblocking");
    let task = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            // Accepted sockets can inherit nonblocking mode on Windows.
            stream.set_nonblocking(false).expect("blocking stream");
            requests.push(request(&mut stream));
            if requests
                .last()
                .is_some_and(|request| request.contains("\"grant_type\":\"refresh_token\""))
            {
                thread::sleep(Duration::from_millis(100));
            }
            let redirect = if (300..400).contains(&status) {
                "Location: http://127.0.0.1:1/must-not-follow\r\n"
            } else {
                ""
            };
            write!(stream, "HTTP/1.1 {status} Fixture\r\n{redirect}Content-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).expect("fixture response");
        }
        requests
    });
    (host, task)
}

fn grant(access: &str, refresh: &str, expires: u64) -> String {
    serde_json::json!({"access_token": access, "refresh_token": refresh, "token_type": "bearer", "expires_in": expires}).to_string()
}

fn decode(value: &str) -> String {
    let mut bytes = value.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let a = char::from(bytes.next().expect("hex"))
                .to_digit(16)
                .expect("hex");
            let b = char::from(bytes.next().expect("hex"))
                .to_digit(16)
                .expect("hex");
            u8::try_from(a * 16 + b).expect("byte")
        } else {
            byte
        });
    }
    String::from_utf8(decoded).expect("UTF8")
}

struct Login {
    child: Option<Child>,
    stderr: Option<thread::JoinHandle<String>>,
    query: BTreeMap<String, String>,
}

impl Login {
    fn start(config: &PathBuf, host: &str, extra: &[&str]) -> Self {
        let mut command = command(config);
        command
            .args(["auth", "login", "--no-browser", "--host", host])
            .args(extra);
        Self::spawn(command, host)
    }

    fn spawn(mut command: Command, host: &str) -> Self {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start CLI");
        let stderr = child.stderr.take().expect("stderr");
        let (sender, receiver) = mpsc::channel();
        let task = thread::spawn(move || {
            let mut output = String::new();
            for line in BufReader::new(stderr).lines() {
                let line = line.expect("progress line");
                if line.starts_with("http://") {
                    let _ = sender.send(line.clone());
                }
                output.push_str(&line);
                output.push('\n');
            }
            output
        });
        let mut login = Self {
            child: Some(child),
            stderr: Some(task),
            query: BTreeMap::new(),
        };
        let url = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("authorization URL");
        assert!(url.starts_with(&format!("{host}/login/oauth/authorize?")));
        login.query = url
            .split_once('?')
            .expect("query")
            .1
            .split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').expect("query pair");
                (decode(key), decode(value))
            })
            .collect();
        assert_eq!(login.query["code_challenge_method"], "S256");
        assert_eq!(login.query["response_type"], "code");
        assert_eq!(login.query["state"].len(), 43);
        login
    }

    fn callback(&self, query: &str) -> String {
        let authority = self.query["redirect_uri"]
            .strip_prefix("http://")
            .expect("loopback")
            .trim_end_matches('/');
        assert!(authority.starts_with("127.0.0.1:"));
        let mut stream = TcpStream::connect(authority).expect("callback connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        write!(
            stream,
            "GET /?{query} HTTP/1.1\r\nHost: {authority}\r\n\r\n"
        )
        .expect("send callback");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read callback");
        response
    }

    fn finish(mut self) -> (Output, String) {
        let output = self
            .child
            .take()
            .expect("child")
            .wait_with_output()
            .expect("CLI result");
        let stderr = self
            .stderr
            .take()
            .expect("reader")
            .join()
            .expect("progress reader");
        (output, stderr)
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn browser_login_pkce_persists_then_refreshes_before_api_use() {
    let fixture = Fixture::new();
    let (host, task) = server(vec![
        (200, grant("access-one", "refresh-one", 1)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, grant("access-two", "refresh-two", 3600)),
        (200, r#"{"login":"alice"}"#.to_owned()),
    ]);
    let login = Login::start(&fixture.config, &host, &["--json"]);
    assert_eq!(
        login.query["client_id"],
        "a4792ccc-144e-407e-86c9-5e7d8d9c3269"
    );
    assert!(
        login
            .callback("code=ignored&state=wrong")
            .starts_with("HTTP/1.1 400")
    );
    let response = login.callback(&format!(
        "code=approved-code&state={}",
        login.query["state"]
    ));
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(!response.contains("approved-code"));
    let challenge = login.query["code_challenge"].clone();
    let (output, progress) = login.finish();
    assert!(output.status.success(), "{progress}");
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("single JSON result");
    assert_eq!(result["action"], "auth.login");
    assert!(!progress.contains("access-one"));
    assert!(!progress.contains("refresh-one"));
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config).expect("saved file"))
            .expect("saved JSON");
    assert_eq!(
        saved["hosts"][&host]["oauth"]["client_id"],
        "a4792ccc-144e-407e-86c9-5e7d8d9c3269"
    );
    let output = command(&fixture.config)
        .args(["auth", "status", "--host", &host, "--json"])
        .output()
        .expect("status");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = task.join().expect("fixture server");
    let body: serde_json::Value =
        serde_json::from_str(requests[0].split_once("\r\n\r\n").expect("body").1)
            .expect("grant body");
    assert!(requests[0].starts_with("POST /forgejo/login/oauth/access_token "));
    assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
    assert_eq!(body["code"], "approved-code");
    assert_eq!(body["client_id"], "a4792ccc-144e-407e-86c9-5e7d8d9c3269");
    assert_eq!(
        crypto::base64url(&crypto::sha256(
            body["code_verifier"].as_str().expect("verifier").as_bytes()
        )),
        challenge
    );
    assert!(requests[1].starts_with("GET /forgejo/api/v1/user "));
    assert!(requests[1].contains("Authorization: Bearer access-one"));
    assert!(requests[2].contains("refresh_token"));
    assert!(requests[2].contains("refresh-one"));
    assert!(requests[2].contains("a4792ccc-144e-407e-86c9-5e7d8d9c3269"));
    assert!(requests[3].contains("Authorization: Bearer access-two"));
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config).expect("saved file"))
            .expect("saved JSON");
    assert_eq!(saved["hosts"][&host]["token"], "access-two");
    assert_eq!(
        saved["hosts"][&host]["oauth"]["refresh_token"],
        "refresh-two"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("access-two"));
}

#[test]
fn denial_does_not_save_credentials_or_exchange_tokens() {
    let fixture = Fixture::new();
    let host = "http://127.0.0.1:1/forgejo";
    let login = Login::start(&fixture.config, host, &["--client-id", "fjx-tests"]);
    let response = login.callback(&format!(
        "error=access_denied&state={}",
        login.query["state"]
    ));
    assert!(response.starts_with("HTTP/1.1 400"));
    let (output, progress) = login.finish();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(progress.contains("declined"));
    assert!(!fixture.config.exists());
}

#[test]
fn exchange_redirects_and_sensitive_error_bodies_are_not_disclosed() {
    for status in [302, 400] {
        let fixture = Fixture::new();
        let (host, task) = server(vec![(
            status,
            r#"{"access_token":"secret-access","message":"approved-code secret-refresh"}"#
                .to_owned(),
        )]);
        let login = Login::start(&fixture.config, &host, &["--client-id", "fjx-tests"]);
        login.callback(&format!(
            "code=approved-code&state={}",
            login.query["state"]
        ));
        let (output, progress) = login.finish();
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stdout.is_empty());
        for secret in ["secret-access", "secret-refresh", "approved-code"] {
            assert!(!progress.contains(secret));
        }
        assert!(!fixture.config.exists());
        assert_eq!(task.join().expect("server").len(), 1);
    }
}

#[test]
fn refresh_failure_preserves_credentials_and_never_runs_the_command() {
    let fixture = Fixture::new();
    let (host, task) = server(vec![
        (200, grant("access-one", "refresh-one", 1)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (
            400,
            r#"{"error":"invalid_grant","message":"refresh-one"}"#.to_owned(),
        ),
    ]);
    let login = Login::start(&fixture.config, &host, &["--client-id", "fjx-tests"]);
    login.callback(&format!(
        "code=approved-code&state={}",
        login.query["state"]
    ));
    assert!(login.finish().0.status.success());
    let before = fs::read(&fixture.config).expect("saved config");
    let output = command(&fixture.config)
        .args(["api", "user", "--host", &host])
        .output()
        .expect("API");
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("refresh-one"));
    assert_eq!(fs::read(&fixture.config).expect("saved config"), before);
    assert_eq!(task.join().expect("server").len(), 3);
}

#[test]
fn conflicting_flags_and_invalid_client_ids_fail_without_opening_a_browser() {
    let fixture = Fixture::new();
    for args in [
        vec!["auth", "login", "--with-token", "--web"],
        vec!["auth", "login", "--with-token", "--no-browser"],
        vec!["auth", "login", "--with-token", "--client-id", "test"],
        vec!["auth", "status", "--no-browser"],
        vec!["repo", "view", "--client-id", "test"],
        vec!["auth", "login", "--web", "--web"],
        vec!["auth", "login", "--client-id", ""],
    ] {
        let output = command(&fixture.config)
            .args(args)
            .output()
            .expect("invalid flags");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
    let output = command(&fixture.config)
        .args([
            "auth",
            "login",
            "--web",
            "--host",
            "https://forge.example/prefix",
            "--client-id",
            "invalid client",
        ])
        .output()
        .expect("invalid ID");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("OAuth client ID must contain only"));
    assert!(output.stdout.is_empty());
}

#[test]
fn environment_client_id_overrides_default_but_an_explicit_id_wins() {
    let fixture = Fixture::new();
    let host = "http://127.0.0.1:1/forgejo";
    for (explicit, expected) in [
        (None, "environment-client"),
        (Some("explicit-client"), "explicit-client"),
    ] {
        let mut command = command(&fixture.config);
        command
            .args(["auth", "login", "--no-browser", "--host", host])
            .env("FJX_CLIENT_ID", "environment-client");
        if let Some(id) = explicit {
            command.args(["--client-id", id]);
        }
        let login = Login::spawn(command, host);
        assert_eq!(login.query["client_id"], expected);
        login.callback(&format!(
            "error=access_denied&state={}",
            login.query["state"]
        ));
        assert_eq!(login.finish().0.status.code(), Some(3));
    }
    let output = command(&fixture.config)
        .args(["auth", "login", "--web", "--host", host])
        .env("FJX_CLIENT_ID", "")
        .output()
        .expect("empty environment client");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OAuth client ID must contain only"));
}

#[test]
fn simultaneous_commands_share_one_refresh_and_keep_rotated_credentials() {
    let fixture = Fixture::new();
    let (host, task) = server(vec![
        (200, grant("access-one", "refresh-one", 1)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, grant("access-two", "refresh-two", 3600)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, r#"{"login":"alice"}"#.to_owned()),
    ]);
    let login = Login::start(&fixture.config, &host, &["--client-id", "fjx-tests"]);
    login.callback(&format!(
        "code=approved-code&state={}",
        login.query["state"]
    ));
    assert!(login.finish().0.status.success());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let config = fixture.config.clone();
        let host = host.clone();
        let barrier = barrier.clone();
        tasks.push(thread::spawn(move || {
            barrier.wait();
            command(&config)
                .args(["auth", "status", "--host", &host])
                .output()
                .expect("status")
        }));
    }
    barrier.wait();
    for task in tasks {
        let output = task.join().expect("status task");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let requests = task.join().expect("server");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("\"grant_type\":\"refresh_token\""))
            .count(),
        1
    );
    assert!(requests[3].contains("Bearer access-two"));
    assert!(requests[4].contains("Bearer access-two"));
}

#[test]
fn saved_client_id_is_reused_and_token_login_clears_oauth_metadata() {
    let fixture = Fixture::new();
    let (host, task) = server(vec![
        (200, grant("access-one", "refresh-one", 3600)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, grant("access-two", "refresh-two", 3600)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, r#"{"login":"bob"}"#.to_owned()),
        (200, r#"{"login":"bob"}"#.to_owned()),
    ]);
    for flags in [vec!["--client-id", "fjx-tests"], vec![]] {
        let login = Login::start(&fixture.config, &host, &flags);
        assert_eq!(login.query["client_id"], "fjx-tests");
        login.callback(&format!(
            "code=approved-code&state={}",
            login.query["state"]
        ));
        assert!(login.finish().0.status.success());
    }
    let mut child = command(&fixture.config)
        .args(["auth", "login", "--with-token", "--host", &host])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("token login");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"personal-token\n")
        .expect("token input");
    assert!(child.wait_with_output().expect("login").status.success());
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.config).expect("config")).expect("JSON");
    assert!(saved["hosts"][&host].get("oauth").is_none());
    assert_eq!(saved["hosts"][&host]["user"], "bob");
    assert!(
        command(&fixture.config)
            .args(["auth", "status", "--host", &host])
            .output()
            .expect("status")
            .status
            .success()
    );
    let requests = task.join().expect("server");
    assert!(requests[4].contains("Authorization: token personal-token"));
    assert!(requests[5].contains("Authorization: token personal-token"));
}

#[test]
fn git_helper_refreshes_and_environment_tokens_bypass_expired_oauth() {
    let fixture = Fixture::new();
    let (host, task) = server(vec![
        (200, grant("access-one", "refresh-one", 1)),
        (200, r#"{"login":"alice"}"#.to_owned()),
        (200, r#"{"login":"env-user"}"#.to_owned()),
        (200, grant("access-two", "refresh-two", 3600)),
    ]);
    let login = Login::start(&fixture.config, &host, &["--client-id", "fjx-tests"]);
    login.callback(&format!(
        "code=approved-code&state={}",
        login.query["state"]
    ));
    assert!(login.finish().0.status.success());
    assert!(
        command(&fixture.config)
            .env("FJX_TOKEN", "environment-token")
            .args(["auth", "status", "--host", &host])
            .output()
            .expect("env status")
            .status
            .success()
    );
    let authority = host
        .strip_prefix("http://")
        .expect("scheme")
        .split('/')
        .next()
        .expect("authority");
    let mut child = command(&fixture.config)
        .args(["auth", "git-credential", "get"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("helper");
    write!(
        child.stdin.take().expect("stdin"),
        "protocol=http\nhost={authority}\npath=forgejo/alice/repo.git\n\n"
    )
    .expect("request");
    let output = child.wait_with_output().expect("helper result");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "username=alice\npassword=access-two\n"
    );
    assert!(output.stderr.is_empty());
    let requests = task.join().expect("server");
    assert!(requests[2].contains("Authorization: token environment-token"));
    assert!(requests[3].contains("refresh-one"));
}

#[cfg(unix)]
#[test]
fn web_login_launches_browser_executable_with_one_url_argument() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let browser = fixture.root.join("browser");
    let capture = fixture.root.join("browser-url");
    let capture_quoted = format!("'{}'", capture.to_string_lossy().replace('\'', "'\\''"));
    fs::write(
        &browser,
        format!("#!/bin/sh\n[ \"$#\" = 1 ] || exit 2\nprintf '%s' \"$1\" > {capture_quoted}\n"),
    )
    .expect("browser fixture");
    fs::set_permissions(&browser, fs::Permissions::from_mode(0o700)).expect("executable");
    let mut command = command(&fixture.config);
    let host = "http://127.0.0.1:1/forgejo";
    command
        .args(["auth", "login", "--web", "--host", host])
        .env("FJX_CLIENT_ID", "environment-client")
        .env("FJX_BROWSER", browser);
    let login = Login::spawn(command, host);
    assert_eq!(login.query["client_id"], "environment-client");
    login.callback(&format!(
        "error=access_denied&state={}",
        login.query["state"]
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !capture.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let captured = fs::read_to_string(capture).expect("browser launched");
    assert!(captured.starts_with(&format!("{host}/login/oauth/authorize?")));
    assert!(captured.ends_with(&format!("state={}", login.query["state"])));
    assert_eq!(login.finish().0.status.code(), Some(3));
}
