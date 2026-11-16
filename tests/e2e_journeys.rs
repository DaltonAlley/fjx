#![allow(clippy::expect_used, reason = "bounded local end-to-end fixtures")]

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TOKEN: &str = "journey-fixture-token";
const API: &str = "/forgejo/api/v1";
const ISSUES: &str = "/forgejo/api/v1/repos/owner/repo/issues";
const PULL: &str = "/forgejo/api/v1/repos/owner/repo/pulls/8";
const HEAD_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEAD_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct State {
    issues: Vec<Value>,
    requests: Vec<(String, String, Value)>,
    lose_create_response: bool,
    fail_page_two: bool,
    head: String,
    check: Option<&'static str>,
    merged: bool,
}

enum Reply {
    Json {
        status: u16,
        body: Value,
        total: Option<usize>,
    },
    Disconnect,
}

impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self::Json {
            status,
            body,
            total: None,
        }
    }
}

impl State {
    // Responses depend on accepted writes, rather than a canned response queue.
    // Unexpected routes fail closed; request transcripts omit credentials.
    fn respond(&mut self, method: &str, target: &str, body: &Value) -> Reply {
        self.requests
            .push((method.into(), target.into(), body.clone()));
        if method == "GET" && target == format!("{API}/user") {
            return Reply::json(200, json!({"login":"dalton"}));
        }
        if method == "POST" && target == ISSUES {
            let mut issue: Value =
                serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/issue.json"))
                    .expect("issue fixture");
            issue["number"] = json!(self.issues.len() + 1);
            issue["title"] = body["title"].clone();
            issue["body"] = body["body"].clone();
            issue["html_url"] = json!(format!(
                "https://fixture/owner/repo/issues/{}",
                self.issues.len() + 1
            ));
            self.issues.push(issue.clone());
            if std::mem::take(&mut self.lose_create_response) {
                return Reply::Disconnect; // The write happened before the connection died.
            }
            return Reply::json(201, issue);
        }
        if method == "GET" && target.starts_with(&format!("{ISSUES}?")) {
            return self.list_issues(target);
        }
        if let Some(number) = target
            .strip_prefix(&format!("{ISSUES}/"))
            .and_then(|n| n.parse::<usize>().ok())
            && let Some(issue) = number.checked_sub(1).and_then(|n| self.issues.get_mut(n))
        {
            if method == "PATCH" {
                issue["state"] = body["state"].clone();
            }
            if matches!(method, "GET" | "PATCH") {
                return Reply::json(200, issue.clone());
            }
        }
        self.respond_pull(method, target, body)
    }

    fn list_issues(&self, target: &str) -> Reply {
        let query = target.split_once('?').expect("query").1;
        let parameter = |name: &str| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        };
        let page: usize = parameter("page")
            .expect("page")
            .parse()
            .expect("page number");
        let limit: usize = parameter("limit")
            .expect("limit")
            .parse()
            .expect("page size");
        if page == 2 && self.fail_page_two {
            return Reply::json(503, json!({"message":"temporary page failure"}));
        }
        let issues: Vec<_> = self
            .issues
            .iter()
            .filter(|issue| {
                parameter("state") == Some("all") || parameter("state") == issue["state"].as_str()
            })
            .cloned()
            .collect();
        Reply::Json {
            status: 200,
            body: json!(
                issues
                    .iter()
                    .skip((page - 1) * limit)
                    .take(limit)
                    .collect::<Vec<_>>()
            ),
            total: Some(issues.len()),
        }
    }

    fn respond_pull(&mut self, method: &str, target: &str, body: &Value) -> Reply {
        if method == "GET" && target == PULL {
            let mut pull: Value =
                serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/pull-request.json"))
                    .expect("pull fixture");
            pull["head"]["sha"] = json!(self.head);
            pull["merged"] = json!(self.merged);
            pull["state"] = json!(if self.merged { "closed" } else { "open" });
            return Reply::json(200, pull);
        }
        if method == "GET"
            && target
                == format!(
                    "{API}/repos/owner/repo/commits/{}/status?page=1&limit=50",
                    self.head
                )
        {
            return Reply::json(
                200,
                self.check.map_or_else(
                    || json!({"sha":"","state":"","total_count":0,"statuses":[]}),
                    |check| {
                        json!({"sha":self.head,"state":check,"total_count":1,
                    "statuses":[{"context":"Build and E2E","status":check}]})
                    },
                ),
            );
        }
        if method == "POST" && target == format!("{PULL}/merge") {
            // Like Forgejo, an omitted precondition permits a merge. This makes
            // losing the CLI's head guard observable, not a fixture-only error.
            if body
                .get("head_commit_id")
                .is_some_and(|head| head != &json!(self.head))
            {
                return Reply::json(
                    409,
                    json!({"message":"head changed; inspect the new revision"}),
                );
            }
            self.merged = true;
            return Reply::json(204, Value::Null);
        }
        Reply::json(
            404,
            json!({"message":format!("unexpected fixture route: {method} {target}")}),
        )
    }
}

struct Fixture {
    root: PathBuf,
    host: String,
    address: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fjx-journey-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("isolated fixture directory");
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        let address = listener.local_addr().expect("fixture address");
        let state = Arc::new(Mutex::new(State {
            head: HEAD_A.into(),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let server_state = Arc::clone(&state);
        let server_stop = Arc::clone(&stop);
        let server = thread::spawn(move || {
            for connection in listener.incoming() {
                let mut stream = connection.expect("fixture connection");
                if server_stop.load(Ordering::Acquire) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read deadline");
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write deadline");
                let request = read_request(&mut stream);
                let (headers, body) = request.split_once("\r\n\r\n").expect("HTTP request");
                assert!(
                    headers
                        .lines()
                        .any(|line| line
                            .eq_ignore_ascii_case(&format!("authorization: token {TOKEN}"))),
                    "fixture authentication missing"
                );
                let mut words = headers
                    .lines()
                    .next()
                    .expect("request line")
                    .split_whitespace();
                let method = words.next().expect("method");
                let target = words.next().expect("target");
                let body = if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(body).expect("request JSON")
                };
                let reply = server_state
                    .lock()
                    .expect("fixture state")
                    .respond(method, target, &body);
                if let Reply::Json {
                    status,
                    body,
                    total,
                } = reply
                {
                    let body = if status == 204 {
                        String::new()
                    } else {
                        body.to_string()
                    };
                    let total =
                        total.map_or_else(String::new, |n| format!("X-Total-Count: {n}\r\n"));
                    write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\n{total}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("fixture response");
                }
            }
        });
        Self {
            root,
            host: format!("http://{address}/forgejo"),
            address,
            state,
            stop,
            server: Some(server),
        }
    }

    fn run(&self, args: &[&str], input: Option<&str>, token: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
        // Start from an empty environment: real tokens, proxies, Git config and
        // repository inference must never influence this server or the user.
        command
            .env_clear()
            .current_dir(&self.root)
            .env("PATH", std::env::var_os("PATH").expect("test PATH"))
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("FJX_CONFIG", self.root.join("hosts.json"))
            .args(args)
            .args(["--host", &self.host, "-R", "owner/repo"]);
        #[cfg(windows)]
        command.env(
            "SystemRoot",
            std::env::var_os("SystemRoot").expect("SystemRoot"),
        );
        if token {
            command.env("FJX_TOKEN", TOKEN);
        }
        let output = bounded_output(command, input);
        for bytes in [&output.stdout, &output.stderr] {
            assert!(
                !String::from_utf8_lossy(bytes).contains(TOKEN),
                "credential leaked by CLI"
            );
        }
        output
    }

    fn json(&self, args: &[&str], token: bool) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let output = self.run(&args, None, token);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).expect("one complete JSON result")
    }

    fn request_count(&self) -> usize {
        self.state.lock().expect("fixture state").requests.len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake blocking accept without a polling loop or readiness sleep.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(2));
        let result = self.server.take().expect("server task").join();
        let _ = fs::remove_dir_all(&self.root);
        if !thread::panicking() {
            result.expect("fixture server failed");
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).expect("bounded request read");
        assert!(count > 0, "incomplete request");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 65536, "oversized fixture request");
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if bytes.len() == end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).expect("UTF8 HTTP")
}

struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bounded_output(mut command: Command, input: Option<&str>) -> Output {
    let mut guarded = Reap(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("built CLI"),
    );
    let child = &mut guarded.0;
    let mut stdin = child.stdin.take().expect("stdin");
    if let Some(input) = input {
        stdin
            .write_all(input.as_bytes())
            .expect("small fixture input");
    }
    drop(stdin);
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let (sender, receiver) = mpsc::channel();
    let out_sender = sender.clone();
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).expect("stdout read");
        let _ = out_sender.send(());
        bytes
    });
    let err_sender = sender.clone();
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).expect("stderr read");
        let _ = err_sender.send(());
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("CLI status") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        // Pipe completion wakes this immediately; the small bounded poll also
        // handles a child that closes its pipes before exiting.
        let _ = receiver.recv_timeout(Duration::from_millis(5));
    };
    if status.is_none() {
        let _ = child.kill();
    }
    let reaped = child.wait().expect("reap CLI");
    let stdout = out.join().expect("stdout task");
    let stderr = err.join().expect("stderr task");
    assert!(
        status.is_some(),
        "CLI exceeded 10s; stdout={}, stderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    Output {
        status: reaped,
        stdout,
        stderr,
    }
}

fn assert_error(output: &Output, code: i32, diagnostic: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "failure exposed partial/success output"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(diagnostic),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn saved_login_issue_lifecycle_and_logout() {
    let fixture = Fixture::new();
    let login = fixture.run(
        &["auth", "login", "--with-token", "--json"],
        Some(&format!("{TOKEN}\n")),
        false,
    );
    assert!(
        login.status.success(),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    let auth = fixture.json(&["auth", "status"], false);
    assert_eq!(auth["user"], "dalton");
    assert_eq!(auth["source"], "config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(fixture.root.join("hosts.json"))
                .expect("saved config")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let body = "Steps:\n1. Open café\n2. See error\n";
    fs::write(fixture.root.join("body.txt"), body).expect("body file");
    let args = [
        "issue",
        "create",
        "--title",
        "Tab\tand\nline",
        "--body-file",
        "body.txt",
    ];
    let before = fixture.request_count();
    let mut preview = args.to_vec();
    preview.push("--dry-run");
    fixture.json(&preview, false);
    assert_eq!(fixture.request_count(), before, "dry-run sent a request");
    assert!(fixture.state.lock().expect("state").issues.is_empty());
    let created = fixture.json(&args, false);
    assert_eq!(created["number"], 1);
    assert_eq!(created["body"], body);
    let viewed = fixture.json(
        &["issue", "view", "1", "--fields", "number,title,body,state"],
        false,
    );
    assert_eq!(
        viewed,
        json!({"number":1,"title":"Tab\tand\nline","body":body,"state":"open"})
    );
    let plain = fixture.run(&["issue", "list"], None, false);
    assert!(plain.status.success());
    assert!(plain.stderr.is_empty());
    assert_eq!(
        String::from_utf8(plain.stdout).expect("plain output"),
        "1\topen\tTab\\tand\\nline\tdalton\thttps://fixture/owner/repo/issues/1\n"
    );
    assert_eq!(
        fixture.json(&["issue", "close", "1"], false)["state"],
        "closed"
    );
    assert_eq!(
        fixture.json(&["issue", "view", "1"], false)["state"],
        "closed"
    );
    assert_eq!(
        fixture.json(&["issue", "reopen", "1"], false)["state"],
        "open"
    );
    fixture.json(&["auth", "logout"], false);
    let before = fixture.request_count();
    assert_error(
        &fixture.run(&["issue", "view", "1"], None, false),
        3,
        "no token",
    );
    assert_eq!(
        fixture.request_count(),
        before,
        "logout still permitted an authenticated request"
    );
}

#[test]
fn lost_write_reply_and_failed_second_page_recover_without_duplicates_or_partial_stdout() {
    let fixture = Fixture::new();
    fixture.state.lock().expect("state").lose_create_response = true;
    assert_error(
        &fixture.run(
            &[
                "issue",
                "create",
                "--title",
                "Committed before disconnect",
                "--body",
                "first",
            ],
            None,
            true,
        ),
        4,
        "request failed",
    );
    {
        let state = fixture.state.lock().expect("state");
        assert_eq!(state.issues.len(), 1);
        assert_eq!(state.requests.len(), 1, "ambiguous write was retried");
    }
    // Recover by reading server state, rather than blindly repeating the write.
    let recovered = fixture.json(&["issue", "list", "--all"], true);
    assert_eq!(recovered.as_array().expect("issues").len(), 1);
    assert_eq!(recovered[0]["body"], "first");
    fixture.json(
        &["issue", "create", "--title", "Second", "--body", "second"],
        true,
    );
    fixture.state.lock().expect("state").fail_page_two = true;
    assert_error(
        &fixture.run(
            &[
                "issue",
                "list",
                "--all",
                "--limit",
                "1",
                "--json",
                "--fields",
                "number,title",
            ],
            None,
            true,
        ),
        5,
        "temporary page failure",
    );
    fixture.state.lock().expect("state").fail_page_two = false;
    let recovered = fixture.json(
        &[
            "issue",
            "list",
            "--all",
            "--limit",
            "1",
            "--fields",
            "number,title",
        ],
        true,
    );
    assert_eq!(
        recovered,
        json!([{"number":1,"title":"Committed before disconnect"},{"number":2,"title":"Second"}])
    );
    let state = fixture.state.lock().expect("state");
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|(method, _, _)| method == "POST")
            .count(),
        2,
        "unexpected write retry"
    );
    assert!(
        state
            .requests
            .iter()
            .any(|(_, target, _)| target.ends_with("page=2&limit=1")),
        "second page was never exercised"
    );
}

#[test]
fn checks_to_merge_rejects_a_stale_head_then_recovers_on_the_new_revision() {
    let fixture = Fixture::new();
    let checks = fixture.run(&["pr", "checks", "8", "--json"], None, true);
    assert_eq!(checks.status.code(), Some(1));
    let missing: Value = serde_json::from_slice(&checks.stdout).expect("missing checks JSON");
    assert_eq!(missing["sha"], HEAD_A);
    assert_eq!(missing["state"], "unknown");
    fixture.state.lock().expect("state").check = Some("pending");
    let pending = fixture.run(&["pr", "checks", "8", "--json"], None, true);
    assert_eq!(pending.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&pending.stdout).expect("pending checks")["state"],
        "pending"
    );
    fixture.state.lock().expect("state").check = Some("success");
    let passed = fixture.json(&["pr", "checks", "8"], true);
    assert_eq!(passed["sha"], HEAD_A);
    let checked = passed["sha"].as_str().expect("checked head");
    let before = fixture.request_count();
    fixture.json(
        &[
            "pr",
            "merge",
            "8",
            "--match-head",
            checked,
            "--yes",
            "--dry-run",
        ],
        true,
    );
    assert_eq!(
        fixture.request_count(),
        before,
        "merge preview wrote to the server"
    );
    {
        let mut state = fixture.state.lock().expect("state");
        state.head = HEAD_B.into();
        state.check = None;
    }
    assert_error(
        &fixture.run(
            &[
                "pr",
                "merge",
                "8",
                "--match-head",
                checked,
                "--yes",
                "--json",
            ],
            None,
            true,
        ),
        5,
        "head changed",
    );
    assert!(
        !fixture.state.lock().expect("state").merged,
        "stale checked revision was merged"
    );
    let unchecked = fixture.run(&["pr", "checks", "8", "--json"], None, true);
    assert_eq!(unchecked.status.code(), Some(1));
    let missing: Value = serde_json::from_slice(&unchecked.stdout).expect("new head checks");
    assert_eq!(missing["sha"], HEAD_B);
    assert_eq!(missing["state"], "unknown");
    fixture.state.lock().expect("state").check = Some("success");
    let refreshed = fixture.json(&["pr", "checks", "8"], true);
    assert_eq!(refreshed["sha"], HEAD_B);
    fixture.json(
        &[
            "pr",
            "merge",
            "8",
            "--match-head",
            refreshed["sha"].as_str().expect("new head"),
            "--yes",
        ],
        true,
    );
    let merged = fixture.json(&["pr", "view", "8"], true);
    assert_eq!(merged["state"], "closed");
    assert_eq!(merged["merged"], true);
    let state = fixture.state.lock().expect("state");
    let writes: Vec<_> = state
        .requests
        .iter()
        .filter(|(method, _, _)| method == "POST")
        .collect();
    assert_eq!(writes.len(), 2, "merge was retried");
    assert_eq!(writes[0].2["head_commit_id"], HEAD_A);
    assert_eq!(writes[1].2["head_commit_id"], HEAD_B);
}
