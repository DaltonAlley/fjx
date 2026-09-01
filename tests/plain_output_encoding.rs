use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const RAW: &str = "literal\\t\\r\\n\\u{85}\\end\tcarriage\rline\nnul\0delete\u{7f}next\u{85}";
const ENCODED: &str =
    "literal\\\\t\\\\r\\\\n\\\\u{85}\\\\end\\tcarriage\\rline\\nnul\\u{0}delete\\u{7F}next\\u{85}";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-plain-output-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn response(body: &Value) -> String {
    let body = serde_json::to_string(body).unwrap_or_else(|error| panic!("{error}"));
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn serve(responses: Vec<String>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
            read_request(&mut stream);
            stream
                .write_all(response.as_bytes())
                .unwrap_or_else(|error| panic!("{error}"));
        }
    });
    (format!("http://{address}"), handle)
}

fn read_request(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("{error}"));
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

fn invoke(root: &Path, arguments: &[&str], bodies: &[Value]) -> (Output, String) {
    let (host, server) = serve(bodies.iter().map(response).collect());
    let output = Command::new(env!("CARGO_BIN_EXE_fjx"))
        .args(arguments)
        .args(["--host", &host])
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", root.join("hosts.json"))
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    (output, host)
}

fn assert_plain(output: &Output, expected: &str, fields_per_record: &[usize]) {
    assert_eq!(output.stdout, expected.as_bytes());
    let text = std::str::from_utf8(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    let records: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("plain output lacks its final record bound"))
        .split('\n')
        .collect();
    assert_eq!(records.len(), fields_per_record.len());
    for (record, expected_fields) in records.iter().zip(fields_per_record) {
        assert_eq!(record.split('\t').count(), *expected_fields);
    }
}

fn assert_json(output: &Output, expected: &Value) {
    let actual: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(&actual, expected);
    assert_eq!(output.stdout.last(), Some(&b'\n'));
}

fn issue_response() -> Value {
    json!({
        "number": 12,
        "title": RAW,
        "body": RAW,
        "html_url": RAW,
        "user": {"login": RAW},
        "state": "open",
        "labels": [{"name": RAW}],
        "assignees": [{"login": RAW}],
        "created_at": RAW,
        "updated_at": RAW
    })
}

fn pull_response() -> Value {
    json!({
        "number": 8,
        "title": RAW,
        "body": RAW,
        "html_url": RAW,
        "user": {"login": RAW},
        "state": "open",
        "draft": false,
        "mergeable": true,
        "base": {"ref": RAW, "sha": "base"},
        "head": {"ref": RAW, "sha": RAW},
        "created_at": RAW,
        "updated_at": RAW
    })
}

fn run_response() -> Value {
    json!({
        "id": 42,
        "title": RAW,
        "event": RAW,
        "status": "success",
        "prettyref": RAW,
        "commit_sha": RAW,
        "html_url": RAW,
        "created": RAW,
        "updated": RAW
    })
}

#[test]
fn issue_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let args = ["issue", "view", "12", "-R", "owner/repo"];

    let (plain, _) = invoke(&root, &args, &[issue_response()]);
    assert_plain(
        &plain,
        &format!("12\topen\t{ENCODED}\t{ENCODED}\t{ENCODED}\n"),
        &[5],
    );

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let (json_output, _) = invoke(&root, &json_args, &[issue_response()]);
    assert_json(
        &json_output,
        &json!({
            "kind": "issue", "number": 12, "title": RAW, "body": RAW,
            "html_url": RAW, "author": RAW, "state": "open", "labels": [RAW],
            "assignees": [RAW], "created_at": RAW, "updated_at": RAW
        }),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn pull_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let args = ["pr", "view", "8", "-R", "owner/repo"];

    let (plain, _) = invoke(&root, &args, &[pull_response()]);
    assert_plain(
        &plain,
        &format!("8\topen\t{ENCODED}\t{ENCODED}\t{ENCODED}\n"),
        &[5],
    );

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let (json_output, _) = invoke(&root, &json_args, &[pull_response()]);
    assert_json(
        &json_output,
        &json!({
            "kind": "pull_request", "number": 8, "title": RAW, "body": RAW,
            "html_url": RAW, "author": RAW, "state": "open", "draft": false,
            "mergeable": true, "base": RAW, "head": RAW, "head_sha": RAW,
            "created_at": RAW, "updated_at": RAW
        }),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn run_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let args = ["run", "view", "42", "-R", "owner/repo"];

    let (plain, _) = invoke(&root, &args, &[run_response()]);
    assert_plain(
        &plain,
        &format!("42\tcompleted\tsuccess\t{ENCODED}\t{ENCODED}\n"),
        &[5],
    );

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let (json_output, _) = invoke(&root, &json_args, &[run_response()]);
    assert_json(
        &json_output,
        &json!({
            "kind": "run", "id": 42, "name": RAW, "event": RAW,
            "status": "completed", "conclusion": "success", "head_branch": RAW,
            "head_sha": RAW, "html_url": RAW, "created_at": RAW, "updated_at": RAW
        }),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn check_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let checks = json!({
        "sha": RAW,
        "state": "success",
        "statuses": [{
            "context": RAW, "status": "success", "description": RAW, "target_url": RAW
        }]
    });
    let args = ["pr", "checks", "8", "-R", "owner/repo"];

    let (plain, _) = invoke(&root, &args, &[pull_response(), checks.clone()]);
    assert_plain(
        &plain,
        &format!("{ENCODED}\tsuccess\n{ENCODED}\tsuccess\t{ENCODED}\t{ENCODED}\n"),
        &[2, 4],
    );

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let (json_output, _) = invoke(&root, &json_args, &[pull_response(), checks]);
    assert_json(
        &json_output,
        &json!({
            "kind": "checks", "sha": RAW, "state": "success",
            "statuses": [{
                "context": RAW, "state": "success", "description": RAW, "target_url": RAW
            }]
        }),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn auth_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let args = ["auth", "status"];

    let (plain, host) = invoke(&root, &args, &[json!({"login": RAW})]);
    assert_plain(&plain, &format!("{host}\t{ENCODED}\tFJX_TOKEN\n"), &[3]);

    let json_args = ["auth", "status", "--json"];
    let (json_output, json_host) = invoke(&root, &json_args, &[json!({"login": RAW})]);
    assert_json(
        &json_output,
        &json!({"kind": "auth", "host": json_host, "user": RAW, "source": "FJX_TOKEN"}),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn repo_plain_fields_are_reversible_and_json_keeps_original_values() {
    let root = temp_dir();
    let repository = json!({
        "name": RAW,
        "full_name": RAW,
        "description": RAW,
        "private": true,
        "archived": false,
        "default_branch": RAW,
        "html_url": RAW
    });
    let args = ["repo", "view", "-R", "owner/repo"];

    let (plain, _) = invoke(&root, &args, std::slice::from_ref(&repository));
    assert_plain(
        &plain,
        &format!("{ENCODED}\t{ENCODED}\t{ENCODED}\t{ENCODED}\n"),
        &[4],
    );

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let (json_output, _) = invoke(&root, &json_args, &[repository]);
    assert_json(
        &json_output,
        &json!({
            "kind": "repo", "name": RAW, "full_name": RAW, "description": RAW,
            "private": true, "archived": false, "default_branch": RAW, "html_url": RAW
        }),
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
