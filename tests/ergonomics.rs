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
            "fjx-ergonomics-{}-{}",
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

#[test]
fn field_projection_preserves_list_shape_and_empty_lists() {
    let issue: Value = serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/issue.json"))
        .unwrap_or_else(|error| panic!("{error}"));
    let mock = Mock::new(vec![
        response("200 OK", &json!([issue]).to_string(), ""),
        response("200 OK", "[]", ""),
    ]);
    let first = mock.run(&["issue", "list", "--json", "--fields", "number,title,state"]);
    assert_eq!(
        decode(&first),
        json!([{ "number": 12, "title": "Typed issue", "state": "open" }])
    );
    let second = mock.run(&["issue", "list", "--json", "--fields", "number"]);
    assert_eq!(decode(&second), json!([]));
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn unknown_projection_fails_before_network_or_body_file_access() {
    let mock = Mock::new(vec![]);
    let output = mock.run(&[
        "issue",
        "create",
        "--title",
        "test",
        "--body-file",
        "/nonexistent-fjx-body",
        "--json",
        "--fields",
        "typo",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown output field"));
    assert!(mock.finish().is_empty());
}

#[test]
fn projection_does_not_hide_a_later_page_failure() {
    let issue: Value = serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/issue.json"))
        .unwrap_or_else(|error| panic!("{error}"));
    let mock = Mock::new(vec![
        response(
            "200 OK",
            &json!([issue]).to_string(),
            "X-Total-Count: 2\r\n",
        ),
        response("500 Error", "{}", ""),
    ]);
    let output = mock.run(&["issue", "list", "--all", "--json", "--fields", "number"]);
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn raw_empty_success_is_json_null_without_a_retry() {
    for args in [
        vec!["api", "user/example", "-X", "POST", "--json"],
        vec!["api", "user/example", "-X", "DELETE", "--yes", "--json"],
    ] {
        let mock = Mock::new(vec![response("204 No Content", "", "")]);
        let output = mock.run(&args);
        assert_eq!(decode(&output), Value::Null);
        assert_eq!(output.stdout, b"null\n");
        assert_eq!(mock.finish().len(), 1);
    }
}

#[test]
fn run_filters_are_encoded_and_preserved_on_all_pages() {
    let run: Value = serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/action-run.json"))
        .unwrap_or_else(|error| panic!("{error}"));
    let body = json!({"total_count": 2, "workflow_runs": [run]}).to_string();
    let mock = Mock::new(vec![
        response("200 OK", &body, ""),
        response("200 OK", &body, ""),
    ]);
    let output = mock.run(&[
        "run",
        "list",
        "--all",
        "--status",
        "failure",
        "--event",
        "pull_request",
        "--ref",
        "refs/heads/a &b",
        "--head-sha",
        "abc123",
        "--workflow",
        "7",
        "--json",
        "--fields",
        "id,status",
    ]);
    let value = decode(&output);
    assert_eq!(value.as_array().map(Vec::len), Some(2));
    let requests = mock.finish();
    assert_eq!(requests.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        let line = request.lines().next().unwrap_or_default();
        assert!(line.contains(&format!("page={}", index + 1)));
        for filter in [
            "status=failure",
            "event=pull_request",
            "ref=refs%2Fheads%2Fa%20%26b",
            "head_sha=abc123",
            "workflow_id=7",
        ] {
            assert!(line.contains(filter), "{line} missing {filter}");
        }
    }
}

#[test]
fn projection_retains_unsuccessful_check_exit_code() {
    let pull = include_str!("fixtures/forgejo-15.0.7/pull-request.json");
    let checks = json!({"sha":"abc123", "state":"failure", "statuses":[{"context":"tests", "status":"failure"}]}).to_string();
    let mock = Mock::new(vec![
        response("200 OK", pull, ""),
        response("200 OK", &checks, ""),
    ]);
    let output = mock.run(&["pr", "checks", "8", "--json", "--fields", "state"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value, json!({"state":"failure"}));
    assert_eq!(mock.finish().len(), 2);
}

#[test]
fn dry_run_projection_does_not_send_requests() {
    let mock = Mock::new(vec![]);
    let output = mock.run(&[
        "issue",
        "create",
        "--title",
        "test",
        "--dry-run",
        "--json",
        "--fields",
        "method,body",
    ]);
    assert_eq!(
        decode(&output),
        json!({"method":"POST", "body":{"title":"test"}})
    );
    assert!(mock.finish().is_empty());
}

#[test]
fn help_with_projection_options_does_not_send_requests() {
    let mock = Mock::new(vec![]);
    let output = mock.run(&["issue", "list", "--json", "--fields", "number", "--help"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("issue list"));
    assert!(mock.finish().is_empty());
}

#[test]
fn duplicate_projection_fields_fail_before_writes() {
    let mock = Mock::new(vec![]);
    let output = mock.run(&[
        "issue",
        "create",
        "--title",
        "test",
        "--json",
        "--fields",
        "number,number",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate"));
    assert!(mock.finish().is_empty());
}
