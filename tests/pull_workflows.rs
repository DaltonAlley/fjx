#![allow(
    clippy::unwrap_used,
    reason = "mock integration fixtures fail immediately on setup errors"
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::thread;

fn request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}

fn server(bodies: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        bodies.into_iter().map(|body| {
        let (mut stream, _) = listener.accept().unwrap();
        let request = request(&mut stream);
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        request
    }).collect()
    });
    (format!("http://{address}"), handle)
}

fn run(host: &str, args: &[&str], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    command
        .args(args)
        .args(["--host", host, "-R", "owner/repo"])
        .env("FJX_TOKEN", "test-token")
        .env("FJX_CONFIG", "/nonexistent/fjx-pull-workflows.json")
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
fn inline_review_is_commit_bound_and_returns_id() {
    let (host, handle) = server(vec![
        r#"{"id":31,"html_url":"https://example/review/31"}"#.into(),
    ]);
    let sha = "a".repeat(40);
    let output = run(
        &host,
        &[
            "pr",
            "review",
            "8",
            "--event",
            "comment",
            "--comments-file",
            "-",
            "--commit",
            &sha,
            "--json",
        ],
        Some(r#"[{"body":"fix this","path":"src/main.rs","new_position":7}]"#),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], 31);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /api/v1/repos/owner/repo/pulls/8/reviews "));
    let payload: serde_json::Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(payload["commit_id"], sha);
    assert_eq!(payload["comments"][0]["new_position"], 7);
}

#[test]
fn malformed_inline_comments_are_rejected_before_network() {
    let sha = "b".repeat(40);
    for input in [
        "[]",
        r#"[{"body":"x","path":"../a","new_position":1}]"#,
        r#"[{"body":"x","path":"a","new_position":1,"old_position":2}]"#,
        r#"[{"body":"\u001b[31m","path":"a","new_position":1}]"#,
        r#"[{"body":"x","path":"a","new_position":-1}]"#,
        r#"[{"body":"x","path":"a","new_position":1,"extra":true}]"#,
    ] {
        let output = run(
            "http://127.0.0.1:1",
            &[
                "pr",
                "review",
                "8",
                "--event",
                "comment",
                "--comments-file",
                "-",
                "--commit",
                &sha,
            ],
            Some(input),
        );
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("HTTP request failed"));
    }
}

#[test]
fn inline_review_requires_commit_and_single_stdin() {
    for args in [
        vec![
            "pr",
            "review",
            "8",
            "--event",
            "comment",
            "--comments-file",
            "-",
        ],
        vec![
            "pr",
            "review",
            "8",
            "--event",
            "comment",
            "--comments-file",
            "-",
            "--body-file",
            "-",
            "--commit",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
    ] {
        let output = run("http://127.0.0.1:1", &args, Some("[]"));
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn merge_dry_run_contains_head_guard_and_auto_without_writes() {
    let output = run(
        "http://127.0.0.1:1",
        &[
            "pr",
            "merge",
            "8",
            "--yes",
            "--dry-run",
            "--json",
            "--match-head",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--auto",
        ],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["body"]["merge_when_checks_succeed"], true);
    assert_eq!(
        value["body"]["head_commit_id"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn collection_routes_and_stable_ids() {
    for (action, extra, route, body, expected) in [
        (
            "comments",
            None,
            "issues/8/comments",
            r#"[{"id":4,"body":"a\u001b","user":{"login":"u"}}]"#,
            serde_json::json!(4),
        ),
        (
            "reviews",
            None,
            "pulls/8/reviews",
            r#"[{"id":5,"body":"review"}]"#,
            serde_json::json!(5),
        ),
        (
            "review-comments",
            Some("5"),
            "pulls/8/reviews/5/comments",
            r#"[{"id":6,"path":"a","body":"inline"}]"#,
            serde_json::json!(6),
        ),
        (
            "files",
            None,
            "pulls/8/files",
            r#"[{"filename":"src/main.rs","status":"modified","additions":2}]"#,
            serde_json::json!("src/main.rs"),
        ),
    ] {
        let (host, handle) = server(vec![body.into()]);
        let mut args = vec!["pr", action, "8"];
        if let Some(extra) = extra {
            args.push(extra);
        }
        args.push("--json");
        let output = run(&host, &args, None);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value[0]["id"], expected);
        assert!(
            handle.join().unwrap()[0].contains(&format!("/repos/owner/repo/{route}?page=1&limit="))
        );
    }
}

#[test]
fn request_review_posts_users_and_teams() {
    let (host, handle) = server(vec!["[]".into()]);
    let output = run(
        &host,
        &[
            "pr",
            "request-review",
            "8",
            "--reviewer",
            "alice",
            "--team",
            "maintainers",
        ],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = &handle.join().unwrap()[0];
    assert!(request.contains("/pulls/8/requested_reviewers "));
    let payload: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(payload["reviewers"], serde_json::json!(["alice"]));
    assert_eq!(
        payload["team_reviewers"],
        serde_json::json!(["maintainers"])
    );
}

fn pull() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/pull-request.json")).unwrap()
}

#[test]
fn nullable_metadata_current_author_and_review_summaries() {
    let mut value = pull();
    value["labels"] = serde_json::Value::Null;
    value["assignees"] = serde_json::Value::Null;
    let (host, handle) = server(vec![value.to_string()]);
    let output = run(&host, &["pr", "view", "8", "--json"], None);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["labels"], serde_json::json!([]));
    assert_eq!(value["assignees"], serde_json::json!([]));
    handle.join().unwrap();
    let (host, handle) = server(vec![r#"{"login":"alice"}"#.into(), "[]".into()]);
    let output = run(&host, &["pr", "list", "--author", "@me"], None);
    assert!(output.status.success());
    let requests = handle.join().unwrap();
    assert!(requests[0].starts_with("GET /api/v1/user "));
    assert!(requests[1].contains("poster=alice"));
    let (host, handle) = server(vec![r#"[{"id":5,"submitted_at":"2026-10-02T00:00:00Z","dismissed":false,"stale":true,"comments_count":3}]"#.into()]);
    let output = run(&host, &["pr", "reviews", "8", "--json"], None);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["submitted_at"], "2026-10-02T00:00:00Z");
    assert_eq!(value[0]["stale"], true);
    assert_eq!(value[0]["comments_count"], 3);
    assert!(value[0].get("created_at").is_none());
    handle.join().unwrap();
}

#[test]
fn human_view_preserves_lines_and_escapes_terminal_controls() {
    let mut value = pull();
    value["body"] = serde_json::json!("first\nsecond\u{1b}[31m");
    value["labels"] = serde_json::json!([{"name":"bug"}]);
    value["milestone"] = serde_json::json!({"title":"v1"});
    let (host, handle) = server(vec![value.to_string()]);
    let output = run(&host, &["pr", "view", "8", "--human"], None);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("first\nsecond\\u{1B}[31m"));
    assert!(text.contains("Labels: bug\nMilestone: v1"));
    assert!(!text.contains('\u{1b}'));
    handle.join().unwrap();
}

#[test]
fn checks_rejects_wrong_head_and_success_child_contradictions() {
    for status in [
        serde_json::json!({"sha":"different","state":"success","statuses":[{"context":"ci","status":"success"}]}),
        serde_json::json!({"sha":"abc123","state":"success","statuses":[{"context":"ci","status":"pending"}]}),
        serde_json::json!({"sha":"abc123","state":"success","statuses":[]}),
    ] {
        let (host, handle) = server(vec![pull().to_string(), status.to_string()]);
        let output = run(&host, &["pr", "checks", "8", "--json"], None);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        handle.join().unwrap();
    }
}

#[test]
fn create_metadata_and_edit_base_use_pull_endpoints() {
    let (host, handle) = server(vec![pull().to_string()]);
    let output = run(
        &host,
        &[
            "pr",
            "create",
            "--head",
            "feature",
            "--title",
            "change",
            "--label-id",
            "4",
            "--milestone-id",
            "2",
            "--assignee",
            "alice",
            "--json",
        ],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = handle.join().unwrap();
    let body: serde_json::Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["labels"], serde_json::json!([4]));
    assert_eq!(body["milestone"], 2);
    assert_eq!(body["assignees"], serde_json::json!(["alice"]));
    let (host, handle) = server(vec!["{}".into()]);
    let output = run(
        &host,
        &["pr", "edit", "8", "--base", "stable", "--title", "new"],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(handle.join().unwrap()[0].starts_with("PATCH /api/v1/repos/owner/repo/pulls/8 "));
}

#[test]
fn list_resolves_labels_milestone_and_sends_official_filters() {
    let (host, handle) = server(vec![
        r#"[{"id":4,"name":"bug"}]"#.into(),
        r#"[{"id":2,"title":"v1"}]"#.into(),
        "[]".into(),
    ]);
    let output = run(
        &host,
        &[
            "pr",
            "list",
            "--label",
            "bug",
            "--milestone",
            "v1",
            "--author",
            "alice",
            "--sort",
            "oldest",
        ],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = handle.join().unwrap();
    assert!(
        requests[2]
            .contains("pulls?state=open&labels=4&poster=alice&milestone=2&sort=oldest&page=1")
    );
}

#[test]
fn collections_paginate_without_partial_output_and_escape_plain_body() {
    let (host, handle) = server(vec![
        r#"[{"id":4,"body":"a\u001b\n","user":{"login":"u"}}]"#.into(),
    ]);
    let output = run(&host, &["pr", "comments", "8"], None);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "4\tu\ta\\u{1B}\\n\n"
    );
    handle.join().unwrap();
    let (host, handle) = server(vec![r#"[{"body":"missing id"}]"#.into()]);
    let output = run(&host, &["pr", "comments", "8", "--json"], None);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    handle.join().unwrap();
}

#[test]
fn comments_all_collects_pages_and_later_failure_emits_nothing() {
    for fail in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for page in 1..=2 {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(request(&mut stream));
                let body = format!("[{{\"id\":{page},\"body\":\"page {page}\"}}]");
                let status = if fail && page == 2 {
                    "500 Error"
                } else {
                    "200 OK"
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nX-Total-Count: 2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        let output = run(
            &host,
            &["pr", "comments", "8", "--all", "--limit", "1", "--json"],
            None,
        );
        if fail {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        } else {
            assert!(output.status.success());
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value[1]["id"], 2);
        }
        let requests = worker.join().unwrap();
        assert!(requests[0].contains("page=1&limit=1"));
        assert!(requests[1].contains("page=2&limit=1"));
    }
}
