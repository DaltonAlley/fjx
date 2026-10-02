use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::thread;

use serde_json::{Value, json};

fn run(args: &[&str], responses: Vec<(u16, Value, Option<usize>)>) -> (Output, Vec<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error:?}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error:?}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|error| panic!("{error}"));
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, value, total) in responses {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return requests;
                        }
                        thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap_or_else(|error| panic!("{error:?}"));
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = stream
                    .read(&mut buffer)
                    .unwrap_or_else(|error| panic!("{error:?}"));
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
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap_or_else(|error| panic!("{error:?}")));
            let body = value.to_string();
            let total =
                total.map_or_else(String::new, |total| format!("X-Total-Count: {total}\r\n"));
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{total}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap_or_else(|error| panic!("{error:?}"));
        }
        requests
    });
    let output = Command::new(env!("CARGO_BIN_EXE_fjx"))
        .args(args)
        .args(["--host", &format!("http://{address}"), "-R", "owner/repo"])
        .env("FJX_TOKEN", "test-token")
        .env("FJX_CONFIG", "/nonexistent/fjx-issue-workflows")
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN")
        .output()
        .unwrap_or_else(|error| panic!("{error:?}"));
    (
        output,
        server.join().unwrap_or_else(|error| panic!("{error:?}")),
    )
}

fn issue() -> Value {
    serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/issue.json"))
        .unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn filters_resolve_self_once_and_encode_query() {
    let (output, requests) = run(
        &[
            "issue",
            "list",
            "--author",
            "@me",
            "--assignee",
            "@me",
            "--label",
            "needs work",
            "--search",
            "a&b",
        ],
        vec![
            (200, json!({"login":"alice"}), None),
            (200, json!([]), None),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /api/v1/user "));
    assert!(requests[1].contains("created_by=alice&assigned_by=alice&q=a%26b"));
    assert!(requests[1].contains("labels=needs%20work"));
}

#[test]
fn human_view_preserves_lines_but_escapes_terminal_controls() {
    let mut value = issue();
    value["body"] = json!("first\nsecond\u{1b}[31m");
    let (output, _) = run(
        &["issue", "view", "12", "--human"],
        vec![(200, value, None)],
    );
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(text.contains("first\nsecond"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn comments_paginate_and_normalize_author() {
    let comment = json!({"id":4,"body":"hello","user":{"login":"alice"},"html_url":"https://example.test/comment/4","created_at":"2026-10-02T00:00:00Z","updated_at":"2026-10-02T00:00:00Z"});
    let (output, requests) = run(
        &["issue", "comments", "12", "--all", "--limit", "1", "--json"],
        vec![
            (200, json!([comment]), Some(2)),
            (200, json!([comment]), Some(2)),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(values[0]["author"], "alice");
    assert_eq!(
        values
            .as_array()
            .unwrap_or_else(|| panic!("expected array"))
            .len(),
        2
    );
    assert!(requests[1].contains("page=2&limit=1"));
}

#[test]
fn dry_run_resolves_metadata_without_writing() {
    let (output, requests) = run(
        &[
            "issue",
            "create",
            "--title",
            "test",
            "--label",
            "bug",
            "--milestone",
            "v1",
            "--assignee",
            "@me",
            "--dry-run",
            "--json",
        ],
        vec![
            (200, json!([{"id":7,"name":"bug"}]), None),
            (200, json!([{"id":8,"title":"v1"}]), None),
            (200, json!({"login":"alice"}), None),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(requests.iter().all(|request| request.starts_with("GET ")));
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(value["body"]["labels"], json!([7]));
    assert_eq!(value["body"]["milestone"], 8);
    assert_eq!(value["body"]["assignees"], json!(["alice"]));
}

#[test]
fn edit_dry_run_reports_each_delta_request() {
    let (output, requests) = run(
        &[
            "issue",
            "edit",
            "12",
            "--title",
            "changed",
            "--add-label-id",
            "7",
            "--remove-label-id",
            "8",
            "--dry-run",
            "--json",
        ],
        vec![],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(requests.is_empty());
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error:?}"));
    assert_eq!(
        value
            .as_array()
            .unwrap_or_else(|| panic!("expected array"))
            .len(),
        3
    );
    assert_eq!(value[1]["method"], "POST");
    assert_eq!(value[2]["method"], "DELETE");
}

#[test]
fn later_write_failure_reports_successes_and_keeps_stdout_empty() {
    let (output, requests) = run(
        &[
            "issue",
            "edit",
            "12",
            "--title",
            "changed",
            "--add-label-id",
            "7",
        ],
        vec![
            (200, issue(), None),
            (500, json!({"message":"failure"}), None),
        ],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(requests.len(), 2);
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|error| panic!("{error:?}"));
    assert!(stderr.contains("partial update"));
    assert!(stderr.contains("succeeded: PATCH"));
}

#[test]
fn assignee_edit_preserves_unrelated_assignments_and_resolves_self_once() {
    let (output, requests) = run(
        &[
            "issue",
            "edit",
            "12",
            "--add-assignee",
            "@me",
            "--remove-assignee",
            "bob",
            "--dry-run",
            "--json",
        ],
        vec![
            (200, json!({"login":"alice"}), None),
            (
                200,
                json!({"assignees":[{"login":"bob"},{"login":"carol"}]}),
                None,
            ),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value[0]["body"]["assignees"], json!(["carol", "alice"]));
}

#[test]
fn label_lookup_follows_all_pages_before_planning() {
    let (output, requests) = run(
        &[
            "issue",
            "edit",
            "12",
            "--add-label",
            "bug",
            "--dry-run",
            "--json",
        ],
        vec![
            (200, json!([{"id":1,"name":"other"}]), Some(2)),
            (200, json!([{"id":7,"name":"bug"}]), Some(2)),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("page=2"));
    let value: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value[0]["body"]["labels"], json!([7]));
}

#[test]
fn ambiguous_label_prevents_all_writes() {
    let (output, requests) = run(
        &[
            "issue",
            "edit",
            "12",
            "--title",
            "changed",
            "--add-label",
            "bug",
        ],
        vec![(
            200,
            json!([{"id":1,"name":"bug"},{"id":2,"name":"bug"}]),
            None,
        )],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(requests.len(), 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("ambiguous"));
}
