use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Mock {
    host: String,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}

impl Mock {
    fn new(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("{error}"));
        let host = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
        );
        let root = std::env::temp_dir().join(format!(
            "fjx-webhook-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap_or_else(|error| panic!("{error}"));
                        let request = read_request(&mut stream);
                        let index = requests.len();
                        requests.push(request);
                        let response = replies.get(index).map_or(
                            "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            String::as_str,
                        );
                        stream
                            .write_all(response.as_bytes())
                            .unwrap_or_else(|error| panic!("{error}"));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
            requests
        });
        Self {
            host,
            root,
            stop,
            worker: Some(worker),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args(args)
            .args(["--host", &self.host, "-R", "o/r"])
            .env_remove("FJX_HOST")
            .env_remove("FJX_REPO")
            .env_remove("FORGEJO_TOKEN")
            .env("FJX_TOKEN", "fake-test-token")
            .env("FJX_CONFIG", self.root.join("config.json"))
            .current_dir(&self.root)
            .output()
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        self.worker
            .take()
            .unwrap_or_else(|| panic!("missing mock worker"))
            .join()
            .unwrap_or_else(|error| panic!("{error:?}"))
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("{error}"));
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
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
    String::from_utf8(bytes).unwrap_or_else(|error| panic!("{error}"))
}

fn response(status: &str, body: &str, headers: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn decode(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"))
}

fn hook() -> Value {
    json!({"id":7,"type":"forgejo","active":true,"events":["push"],"branch_filter":"main",
        "url":"https://example.com/receiver-secret", "authorization_header":"Bearer receiver-secret",
        "config":{"url":"https://example.com/receiver-secret","secret":"signing-secret"},
        "metadata":{"secret":"metadata-secret"}})
}

fn input(mock: &Mock, body: &Value) -> String {
    let path = mock.root.join("hook.json");
    fs::write(&path, body.to_string()).unwrap_or_else(|e| panic!("{e}"));
    path.to_string_lossy().into_owned()
}

fn assert_private(output: &Output) {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for secret in ["receiver-secret", "signing-secret", "metadata-secret"] {
        assert!(!combined.contains(secret), "credential leaked");
    }
}

#[test]
fn reads_page_hooks_without_disclosing_credentials() {
    let mock = Mock::new(vec![
        response(
            "200 OK",
            &json!([hook()]).to_string(),
            "Link: <http://localhost/ignored>; rel=\"next\"\r\n",
        ),
        response("200 OK", "[]", ""),
        response("200 OK", &hook().to_string(), ""),
    ]);
    let list = mock.run(&["webhook", "list", "--all", "--json"]);
    assert_eq!(
        decode(&list),
        json!([{"id":7,"type":"forgejo","active":true,"events":["push"],"branch_filter":"main"}])
    );
    assert_private(&list);
    let view = mock.run(&["webhook", "view", "7"]);
    assert!(view.status.success());
    assert_private(&view);
    let requests = mock.finish();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].starts_with("GET /api/v1/repos/o/r/hooks?page=2&limit="));
    assert!(requests[2].starts_with("GET /api/v1/repos/o/r/hooks/7 "));
}

#[test]
fn create_sends_secrets_only_to_forgejo_and_dry_run_redacts_them() {
    let mock = Mock::new(vec![response("201 Created", &hook().to_string(), "")]);
    let path = input(
        &mock,
        &json!({"config":{"url":"https://example.com/receiver-secret?token=receiver-secret","secret":"signing-secret"},"events":["push"],"authorization_header":"Bearer receiver-secret"}),
    );
    let preview = mock.run(&["webhook", "create", "--input", &path, "--dry-run", "--json"]);
    let plan = decode(&preview);
    assert_private(&preview);
    assert_eq!(plan["body"]["config"]["url"], "[redacted]");
    assert_eq!(plan["body"]["authorization_header"], "[redacted]");
    let created = mock.run(&["webhook", "create", "--input", &path, "--json"]);
    assert_eq!(decode(&created)["id"], 7);
    assert_private(&created);
    let requests = mock.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /api/v1/repos/o/r/hooks "));
    let body: Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(body["config"]["secret"], "signing-secret");
    assert_eq!(body["type"], "forgejo");
    assert_eq!(body["active"], true);
    assert_eq!(body["config"]["http_method"], "POST");
}

#[test]
fn edits_preserve_omitted_fields_and_test_only_reports_queued() {
    let mock = Mock::new(vec![
        response("200 OK", &hook().to_string(), ""),
        response("200 OK", &hook().to_string(), ""),
        response("204 No Content", "", ""),
        response("204 No Content", "", ""),
    ]);
    let path = input(&mock, &json!({"active":false}));
    assert_eq!(
        decode(&mock.run(&["webhook", "edit", "7", "--input", &path, "--json"]))["id"],
        7
    );
    let queued = mock.run(&[
        "webhook",
        "test",
        "7",
        "--ref",
        "feature/x&evil=1",
        "--json",
    ]);
    assert_eq!(decode(&queued)["action"], "webhook.test.queued");
    assert_eq!(
        decode(&mock.run(&["webhook", "delete", "7", "--yes", "--json"]))["action"],
        "webhook.delete"
    );
    let requests = mock.finish();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("GET /api/v1/repos/o/r/hooks/7 "));
    assert!(requests[1].starts_with("PATCH /api/v1/repos/o/r/hooks/7 "));
    let body: Value =
        serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        body,
        json!({"active":false,"events":["push"],"branch_filter":"main","authorization_header":"Bearer receiver-secret"})
    );
    assert!(
        requests[2].starts_with("POST /api/v1/repos/o/r/hooks/7/tests?ref=feature%2Fx%26evil%3D1 ")
    );
    assert!(requests[3].starts_with("DELETE /api/v1/repos/o/r/hooks/7 "));
}

#[test]
fn invalid_inputs_and_safety_flags_fail_without_network_or_secret_output() {
    let mock = Mock::new(vec![]);
    for body in [
        json!({"config":{"url":"http://example.com/receiver-secret"},"events":["push"]}),
        json!({"config":{"url":"https://receiver-secret@example.com/"},"events":["push"]}),
        json!({"config":{"url":"https://example.com"},"events":["push","push"]}),
        json!({"config":{"url":"https://example.com","secret":5},"events":["push"]}),
        json!({"config":{"url":"https://example.com"},"events":["push"],"authorization_header":"receiver-secret\r\nx: y"}),
        json!({"receiver-secret":true}),
        json!({}),
    ] {
        let path = input(&mock, &body);
        let output = mock.run(&["webhook", "create", "--input", &path, "--json"]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_private(&output);
    }
    for args in [
        vec!["webhook", "delete", "7"],
        vec!["webhook", "test", "7", "--yes"],
        vec!["webhook", "view", "7", "--dry-run"],
        vec!["webhook", "list", "--yes"],
        vec![
            "webhook", "create", "--input", "missing", "--json", "--fields", "url",
        ],
        vec!["webhook", "view", "../7"],
    ] {
        let output = mock.run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(mock.finish().is_empty());
}

#[test]
fn failed_write_is_not_retried_and_does_not_disclose_response() {
    let mock = Mock::new(vec![
        response("200 OK", &hook().to_string(), ""),
        response("500 Error", "{\"message\":\"receiver-secret\"}", ""),
    ]);
    let path = input(&mock, &json!({"active":false}));
    let output = mock.run(&["webhook", "edit", "7", "--input", &path]);
    assert!(!output.status.success());
    assert_private(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn edit_preview_preserves_narrow_event_flags_and_redacts_stored_auth() {
    let mut current = hook();
    current["events"] = json!([
        "issues",
        "pull_request",
        "pull_request_comment",
        "pull_request_review_approved",
        "pull_request_review_comment",
        "pull_request_review_rejected",
        "workflow_run_success"
    ]);
    let mock = Mock::new(vec![response("200 OK", &current.to_string(), "")]);
    let path = input(&mock, &json!({"active":false}));
    let output = mock.run(&[
        "webhook",
        "edit",
        "7",
        "--input",
        &path,
        "--dry-run",
        "--json",
    ]);
    let plan = decode(&output);
    assert_private(&output);
    assert_eq!(
        plan["body"]["events"],
        json!([
            "issues_only",
            "pull_request_comment",
            "pull_request_only",
            "pull_request_review",
            "workflow_run_success"
        ])
    );
    assert_eq!(plan["body"]["authorization_header"], "[redacted]");
    assert_eq!(plan["body"]["branch_filter"], "main");
    assert_eq!(mock.finish().len(), 1);
}

#[test]
fn unsupported_event_changes_and_secret_rotation_fail_before_writes() {
    let mut current = hook();
    current["events"] = json!(["push", "workflow_run_success"]);
    let mock = Mock::new(vec![response("200 OK", &current.to_string(), "")]);
    let path = input(&mock, &json!({"events":["push"]}));
    let output = mock.run(&["webhook", "edit", "7", "--input", &path]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("replacement hook"));
    let path = input(
        &mock,
        &json!({"config":{"url":"https://example.com","secret":"signing-secret"}}),
    );
    let output = mock.run(&["webhook", "edit", "7", "--input", &path]);
    assert!(!output.status.success());
    assert_private(&output);
    assert_eq!(mock.finish().len(), 1);
}

#[test]
fn edits_round_trip_comment_and_review_subscriptions_independently() {
    // Forgejo 17 reports review_comment through its PR-comment flag, while
    // approved/rejected share a separate review flag (models/webhook/webhook.go).
    for (stored, expected) in [
        (
            vec!["pull_request_comment", "pull_request_review_comment"],
            vec!["pull_request_comment"],
        ),
        (
            vec![
                "pull_request_review_approved",
                "pull_request_review_rejected",
            ],
            vec!["pull_request_review"],
        ),
        (
            vec![
                "pull_request_comment",
                "pull_request_review_comment",
                "pull_request_review_approved",
                "pull_request_review_rejected",
            ],
            vec!["pull_request_comment", "pull_request_review"],
        ),
    ] {
        let mut current = hook();
        current["events"] = json!(stored);
        let mock = Mock::new(vec![
            response("200 OK", &current.to_string(), ""),
            response("200 OK", &current.to_string(), ""),
        ]);
        let path = input(&mock, &json!({"active":false}));
        let output = mock.run(&["webhook", "edit", "7", "--input", &path, "--json"]);
        assert_eq!(decode(&output)["events"], json!(stored));
        assert_private(&output);
        let requests = mock.finish();
        assert_eq!(requests.len(), 2);
        let body: Value =
            serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap_or_default())
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(body["events"], json!(expected));

        // Apply the server's two flags to the actual PATCH, then compare the
        // effective read response, rather than trusting our canned PATCH reply.
        let mut after = Vec::new();
        let sent = body["events"]
            .as_array()
            .unwrap_or_else(|| panic!("missing events"));
        if sent.contains(&json!("pull_request_comment")) {
            after.extend(["pull_request_comment", "pull_request_review_comment"]);
        }
        if sent.contains(&json!("pull_request_review")) {
            after.extend([
                "pull_request_review_approved",
                "pull_request_review_rejected",
            ]);
        }
        after.sort_unstable();
        let mut before = stored;
        before.sort_unstable();
        assert_eq!(after, before);
    }
}

#[test]
fn edits_refuse_ambiguous_send_everything_before_patch() {
    // This is EventsArray() for SendEverything=true, including flags that
    // PATCH never updates. The API omits the underlying SendEverything bit.
    let all = all_reported_events();
    let mut current = hook();
    current["events"] = json!(all);
    let mut requested = all
        .iter()
        .filter(|e| {
            !e.starts_with("pull_request_review_") || e.as_str() == "pull_request_review_request"
        })
        .cloned()
        .collect::<Vec<_>>();
    requested.push("pull_request_review".to_owned());
    for body in [json!({"active":false}), json!({"events":requested})] {
        for preview in [false, true] {
            let mock = Mock::new(vec![response("200 OK", &current.to_string(), "")]);
            let path = input(&mock, &body);
            let mut args = vec!["webhook", "edit", "7", "--input", &path, "--json"];
            if preview {
                args.push("--dry-run");
            }
            let output = mock.run(&args);
            assert_eq!(output.status.code(), Some(2));
            assert!(output.stdout.is_empty());
            assert_private(&output);
            assert!(String::from_utf8_lossy(&output.stderr).contains("Send everything"));
            assert_eq!(mock.finish().len(), 1, "must stop after GET, before PATCH");
        }
    }
}

#[test]
fn edits_preserve_all_ci_and_package_events_on_unambiguous_hooks() {
    let mut stored = all_reported_events();
    stored.retain(|e| {
        e == "push" || e == "package" || e.starts_with("workflow_") || e.starts_with("action_run_")
    });
    let mut current = hook();
    current["events"] = json!(stored);
    let mock = Mock::new(vec![
        response("200 OK", &current.to_string(), ""),
        response("200 OK", &current.to_string(), ""),
    ]);
    let path = input(&mock, &json!({"active":false}));
    let output = mock.run(&["webhook", "edit", "7", "--input", &path, "--json"]);
    assert_eq!(decode(&output)["events"], json!(stored));
    assert_private(&output);
    let requests = mock.finish();
    assert_eq!(requests.len(), 2);
    let body: Value =
        serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap_or_default())
            .unwrap_or_else(|e| panic!("{e}"));
    stored.sort();
    assert_eq!(body["events"], json!(stored));
    assert_eq!(body["active"], false);
    assert_eq!(body["branch_filter"], "main");
    assert_eq!(body["authorization_header"], "Bearer receiver-secret");
}

#[test]
fn edits_refuse_review_events_that_cannot_round_trip() {
    for stored in [
        vec!["pull_request_review_approved"],
        vec!["pull_request_review_comment"],
        vec!["pull_request_comment"],
    ] {
        let mut current = hook();
        current["events"] = json!(stored);
        let mock = Mock::new(vec![response("200 OK", &current.to_string(), "")]);
        let path = input(&mock, &json!({"active":false}));
        let output = mock.run(&["webhook", "edit", "7", "--input", &path]);
        assert_eq!(output.status.code(), Some(7));
        assert!(output.stdout.is_empty());
        assert_private(&output);
        assert_eq!(mock.finish().len(), 1);
    }
}

fn all_reported_events() -> Vec<String> {
    let mut all = vec![
        "create",
        "delete",
        "fork",
        "push",
        "issues",
        "issue_assign",
        "issue_label",
        "issue_milestone",
        "issue_comment",
        "pull_request",
        "pull_request_assign",
        "pull_request_label",
        "pull_request_milestone",
        "pull_request_comment",
        "pull_request_review_approved",
        "pull_request_review_rejected",
        "pull_request_review_comment",
        "pull_request_sync",
        "wiki",
        "repository",
        "release",
        "package",
        "pull_request_review_request",
        "action_run_failure",
        "action_run_success",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for prefix in ["workflow_run_", "workflow_job_"] {
        for state in [
            "blocked",
            "cancelled",
            "failure",
            "running",
            "skipped",
            "success",
            "waiting",
        ] {
            all.push(format!("{prefix}{state}"));
        }
    }
    all
}
