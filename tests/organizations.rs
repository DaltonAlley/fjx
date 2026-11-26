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
        let mut replies = replies.into_iter();
        Self::serve(move |_| {
            replies
                .next()
                .unwrap_or_else(|| response("500 Error", "{}", ""))
        })
    }

    fn serve(mut handler: impl FnMut(&str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("{error}"));
        let host = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
        );
        let root = std::env::temp_dir().join(format!(
            "fjx-organization-{}-{}",
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
                        // Accepted sockets can inherit nonblocking mode on Windows.
                        stream
                            .set_nonblocking(false)
                            .unwrap_or_else(|error| panic!("{error}"));
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap_or_else(|error| panic!("{error}"));
                        let request = read_request(&mut stream);
                        let response = handler(&request);
                        requests.push(request);
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
            .args(["--host", &format!("{}/forgejo", self.host)])
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

fn org() -> Value {
    json!({"id":7,"name":"acme","full_name":"Acme Labs","description":"Original",
        "email":"team@example.com","website":"https://example.com","location":"Here",
        "visibility":"private","repo_admin_change_team_access":true})
}

fn team() -> Value {
    json!({"id":11,"name":"Developers","description":"Team description","permission":"read",
        "units_map":{"repo.code":"read","repo.issues":"write"},
        "includes_all_repositories":false,"can_create_org_repo":true})
}

fn member() -> Value {
    json!({"id":19,"login":"alice","full_name":"Alice"})
}

fn repository() -> Value {
    json!({"id":21,"full_name":"acme/project","private":true,"html_url":"https://example.com/acme/project"})
}

fn input(mock: &Mock, value: &Value) -> String {
    let path = mock.root.join("input.json");
    fs::write(&path, value.to_string()).unwrap_or_else(|e| panic!("{e}"));
    path.to_string_lossy().into_owned()
}

fn request_body(request: &str) -> Value {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("request headers"));
    if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}"))
    }
}

fn assert_failure(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "partial stdout on failure");
}

fn json_response(value: &Value) -> String {
    let (status, body) = if value.is_null() {
        ("204 No Content", String::new())
    } else {
        ("200 OK", value.to_string())
    };
    response(status, &body, "")
}

fn lifecycle_mock() -> Mock {
    let mut organization: Option<Value> = None;
    let mut group: Option<Value> = None;
    let mut joined = false;
    let mut granted = false;
    Mock::serve(move |request| {
        let mut words = request.split_whitespace();
        let method = words.next().unwrap_or_default();
        let path = words
            .next()
            .unwrap_or_default()
            .strip_prefix("/forgejo/api/v1/")
            .unwrap_or_default();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: token fake-test-token")
        );
        let body = request_body(request);
        let result = match (method, path) {
            ("POST", "orgs") => {
                assert_eq!(body["username"], "acme");
                let mut value = org();
                value["visibility"] = body["visibility"].clone();
                organization = Some(value.clone());
                value
            }
            ("GET", "user/orgs?page=1&limit=30") => json!(organization.iter().collect::<Vec<_>>()),
            ("GET", "orgs/acme") => organization
                .clone()
                .unwrap_or_else(|| panic!("missing org")),
            ("PATCH", "orgs/acme") => {
                let value = organization
                    .as_mut()
                    .unwrap_or_else(|| panic!("missing org"));
                for (key, new) in body.as_object().unwrap_or_else(|| panic!("body")) {
                    value[key] = new.clone();
                }
                value.clone()
            }
            ("POST", "orgs/acme/teams") => {
                assert_eq!(body["units_map"]["repo.code"], "read");
                let mut value = team();
                value["name"] = body["name"].clone();
                group = Some(value.clone());
                value
            }
            ("GET", "orgs/acme/teams?page=1&limit=30") => json!(group.iter().collect::<Vec<_>>()),
            ("GET", "teams/11") => group.clone().unwrap_or_else(|| panic!("missing team")),
            ("PATCH", "teams/11") => {
                let value = group.as_mut().unwrap_or_else(|| panic!("missing team"));
                assert_eq!(body["name"], value["name"]);
                for (key, new) in body.as_object().unwrap_or_else(|| panic!("body")) {
                    value[key] = new.clone();
                }
                value.clone()
            }
            ("PUT", "teams/11/members/alice") => {
                joined = true;
                Value::Null
            }
            ("GET", "orgs/acme/members?page=1&limit=30" | "teams/11/members?page=1&limit=30") => {
                if joined {
                    json!([member()])
                } else {
                    json!([])
                }
            }
            ("PUT", "teams/11/repos/acme/project") => {
                granted = true;
                Value::Null
            }
            ("GET", "teams/11/repos?page=1&limit=30" | "orgs/acme/repos?page=1&limit=30") => {
                if granted {
                    json!([repository()])
                } else {
                    json!([])
                }
            }
            ("DELETE", "teams/11/repos/acme/project") => {
                granted = false;
                Value::Null
            }
            ("DELETE", "teams/11/members/alice" | "orgs/acme/members/alice") => {
                joined = false;
                Value::Null
            }
            ("DELETE", "teams/11") => {
                group = None;
                Value::Null
            }
            ("DELETE", "orgs/acme") => {
                organization = None;
                Value::Null
            }
            _ => panic!("unexpected {method} {path}"),
        };
        json_response(&result)
    })
}

#[test]
fn organization_and_team_lifecycle_uses_accepted_writes() {
    let mock = lifecycle_mock();
    let file = input(&mock, &json!({"username":"acme","visibility":"private"}));
    assert_eq!(
        decode(&mock.run(&["org", "create", "--input", &file, "--json"]))["name"],
        "acme"
    );
    assert_eq!(
        decode(&mock.run(&["org", "list", "--json"]))[0]["name"],
        "acme"
    );
    let file = input(&mock, &json!({"description":"Updated"}));
    let edited = decode(&mock.run(&["org", "edit", "acme", "--input", &file, "--json"]));
    assert_eq!(edited["description"], "Updated");
    assert_eq!(edited["full_name"], "Acme Labs");
    assert_eq!(edited["website"], "https://example.com");
    assert_eq!(
        decode(&mock.run(&["org", "view", "acme", "--json"]))["visibility"],
        "private"
    );
    let file = input(
        &mock,
        &json!({"name":"Developers","units_map":{"repo.code":"read","repo.issues":"write"}}),
    );
    assert_eq!(
        decode(&mock.run(&["team", "create", "acme", "--input", &file, "--json"]))["id"],
        11
    );
    assert_eq!(
        decode(&mock.run(&["team", "list", "acme", "--json"]))[0]["id"],
        11
    );
    let file = input(&mock, &json!({"includes_all_repositories":true}));
    assert_eq!(
        decode(&mock.run(&["team", "edit", "11", "--input", &file, "--json"]))["includes_all_repositories"],
        true
    );
    assert_eq!(
        decode(&mock.run(&["team", "view", "11", "--json"]))["units_map"]["repo.issues"],
        "write"
    );
    decode(&mock.run(&["team", "add-member", "11", "alice", "--json"]));
    for args in [
        &["team", "members", "11", "--json"][..],
        &["org", "members", "acme", "--json"],
    ] {
        assert_eq!(decode(&mock.run(args))[0]["login"], "alice");
    }
    decode(&mock.run(&["team", "add-repo", "11", "acme/project", "--json"]));
    for args in [
        &["team", "repos", "11", "--json"][..],
        &["org", "repos", "acme", "--json"],
    ] {
        assert_eq!(decode(&mock.run(args))[0]["full_name"], "acme/project");
    }
    decode(&mock.run(&[
        "team",
        "remove-repo",
        "11",
        "acme/project",
        "--yes",
        "--json",
    ]));
    assert_eq!(
        decode(&mock.run(&["team", "repos", "11", "--json"])),
        json!([])
    );
    decode(&mock.run(&["team", "remove-member", "11", "alice", "--yes", "--json"]));
    assert_eq!(
        decode(&mock.run(&["team", "members", "11", "--json"])),
        json!([])
    );
    decode(&mock.run(&["team", "add-member", "11", "alice", "--json"]));
    decode(&mock.run(&["org", "remove-member", "acme", "alice", "--yes", "--json"]));
    assert_eq!(
        decode(&mock.run(&["org", "members", "acme", "--json"])),
        json!([])
    );
    decode(&mock.run(&["team", "delete", "11", "--yes", "--json"]));
    assert_eq!(
        decode(&mock.run(&["team", "list", "acme", "--json"])),
        json!([])
    );
    decode(&mock.run(&["org", "delete", "acme", "--yes", "--json"]));
    assert_eq!(decode(&mock.run(&["org", "list", "--json"])), json!([]));
    let requests = mock.finish();
    assert_eq!(
        requests.iter().filter(|r| r.starts_with("POST ")).count(),
        2
    );
}

#[test]
fn dry_runs_validate_and_preserve_without_writes() {
    let mock = Mock::new(vec![
        response("200 OK", &org().to_string(), ""),
        response("200 OK", &team().to_string(), ""),
    ]);
    let file = input(&mock, &json!({"description":""}));
    let plan = decode(&mock.run(&[
        "org",
        "edit",
        "acme",
        "--input",
        &file,
        "--dry-run",
        "--json",
    ]));
    assert_eq!(plan["method"], "PATCH");
    assert_eq!(plan["body"]["full_name"], "Acme Labs");
    assert_eq!(plan["body"]["description"], "");
    assert_eq!(plan["body"]["website"], "https://example.com");
    let file = input(&mock, &json!({"includes_all_repositories":true}));
    let plan = decode(&mock.run(&[
        "team",
        "edit",
        "11",
        "--input",
        &file,
        "--dry-run",
        "--json",
    ]));
    assert_eq!(plan["body"]["permission"], "read");
    assert_eq!(plan["body"]["name"], "Developers");
    assert!(plan["body"].get("units_map").is_none());
    assert!(mock.finish().iter().all(|r| r.starts_with("GET ")));
    let mock = Mock::new(vec![]);
    for args in [
        &["org", "delete", "acme", "--yes"][..],
        &["org", "remove-member", "acme", "alice", "--yes"],
        &["team", "delete", "11", "--yes"],
        &["team", "add-member", "11", "alice"],
        &["team", "remove-member", "11", "alice", "--yes"],
        &["team", "add-repo", "11", "acme/project"],
        &["team", "remove-repo", "11", "acme/project", "--yes"],
    ] {
        let mut args = args.to_vec();
        args.extend(["--dry-run", "--json"]);
        let plan = decode(&mock.run(&args));
        assert!(plan["body"].is_null());
        assert!(
            plan["url"]
                .as_str()
                .unwrap_or_default()
                .contains("/forgejo/api/v1/")
        );
    }
    assert!(mock.finish().is_empty());
}

#[test]
fn invalid_inputs_and_safety_flags_make_no_requests() {
    let mock = Mock::new(vec![]);
    for args in [
        &["org", "delete", "acme"][..],
        &["org", "remove-member", "acme", "alice"],
        &["team", "delete", "11"],
        &["team", "remove-member", "11", "alice"],
        &["team", "remove-repo", "11", "acme/project"],
    ] {
        assert_failure(&mock.run(args), 6);
        let mut dry = args.to_vec();
        dry.push("--dry-run");
        assert_failure(&mock.run(&dry), 6);
    }
    for args in [
        &["org", "list", "--dry-run"][..],
        &["org", "view", "acme", "--yes"],
        &["org", "members", "acme", "--yes"],
        &["org", "repos", "acme", "--dry-run"],
        &["team", "list", "acme", "--yes"],
        &["team", "view", "11", "--dry-run"],
        &["team", "members", "11", "--yes"],
        &["team", "repos", "11", "--dry-run"],
        &["team", "add-member", "11", "alice", "--yes"],
        &["team", "add-repo", "11", "acme/project", "--yes"],
        &["team", "view", "0"],
        &["team", "add-repo", "11", "acme/../project"],
        &["org", "view", ".."],
        &["org", "view", "a/b"],
        &["org", "view", "acme?x=y"],
        &["org", "list", "--permission", "admin"],
        &["org", "members", "acme", "--state", "all"],
    ] {
        assert_failure(&mock.run(args), 2);
    }
    for value in [
        json!({}),
        json!([]),
        json!({"username":".."}),
        json!({"username":"acme","visibility":"secret"}),
        json!({"username":"acme","unknown":true}),
        json!({"username":null}),
    ] {
        let file = input(&mock, &value);
        assert_failure(&mock.run(&["org", "create", "--input", &file]), 2);
    }
    let file = input(&mock, &json!({"username":"new-name"}));
    assert_failure(&mock.run(&["org", "edit", "acme", "--input", &file]), 2);
    for value in [
        json!({"name":""}),
        json!({"name":"Dev","permission":"owner"}),
        json!({"name":"Dev"}),
        json!({"name":"Dev","units_map":{}}),
        json!({"name":"Dev","units_map":{"repo.fake":"read"}}),
        json!({"name":"Dev","units_map":{"repo.code":"owner"}}),
        json!({"name":"Dev","permission":"admin","units_map":{"repo.code":"read"}}),
    ] {
        let file = input(&mock, &value);
        assert_failure(&mock.run(&["team", "create", "acme", "--input", &file]), 2);
    }
    let file = input(&mock, &json!({"permission":"read"}));
    assert_failure(&mock.run(&["team", "edit", "11", "--input", &file]), 2);
    // Projection validation must happen before a PUT or input read.
    assert_failure(
        &mock.run(&[
            "team",
            "add-member",
            "11",
            "alice",
            "--json",
            "--fields",
            "missing",
        ]),
        2,
    );
    assert!(mock.finish().is_empty());
}

#[test]
fn all_organization_lists_follow_pages_and_project_fields() {
    let mock = Mock::new(vec![
        response(
            "200 OK",
            &json!([org()]).to_string(),
            "x-total-count: 2\r\n",
        ),
        response(
            "200 OK",
            &json!([org()]).to_string(),
            "x-total-count: 2\r\n",
        ),
    ]);
    let value = decode(&mock.run(&[
        "org",
        "list",
        "alice",
        "--all",
        "--limit",
        "1",
        "--json",
        "--fields",
        "name,visibility",
    ]));
    assert_eq!(
        value,
        json!([{"name":"acme","visibility":"private"},{"name":"acme","visibility":"private"}])
    );
    let requests = mock.finish();
    assert!(requests[0].starts_with("GET /forgejo/api/v1/users/alice/orgs?page=1&limit=1 "));
    assert!(requests[1].starts_with("GET /forgejo/api/v1/users/alice/orgs?page=2&limit=1 "));
}

#[test]
fn later_page_failure_and_incompatible_responses_leave_stdout_empty() {
    let mock = Mock::new(vec![
        response(
            "200 OK",
            &json!([team()]).to_string(),
            "x-total-count: 2\r\n",
        ),
        response("503 Unavailable", "{}", ""),
    ]);
    assert_failure(&mock.run(&["team", "list", "acme", "--all", "--json"]), 5);
    assert_eq!(mock.finish().len(), 2);
    let mock = Mock::new(vec![response("200 OK", "{\"id\":7}", "")]);
    let file = input(&mock, &json!({"description":"updated"}));
    assert_failure(&mock.run(&["org", "edit", "acme", "--input", &file]), 7);
    assert_eq!(mock.finish().len(), 1); // No PATCH from an incomplete preservation read.
}

#[test]
fn access_edits_preserve_unrelated_permissions_and_refuse_owner_changes() {
    let mock = Mock::new(vec![response("200 OK", &team().to_string(), "")]);
    let file = input(&mock, &json!({"description":"New"}));
    let plan = decode(&mock.run(&[
        "team",
        "edit",
        "11",
        "--input",
        &file,
        "--dry-run",
        "--json",
    ]));
    assert!(plan["body"].get("permission").is_none());
    assert!(plan["body"].get("units_map").is_none());
    assert_eq!(mock.finish().len(), 1);
    let mock = Mock::new(vec![response("200 OK", &team().to_string(), "")]);
    let file = input(&mock, &json!({"units_map":{"repo.code":"write"}}));
    let plan = decode(&mock.run(&[
        "team",
        "edit",
        "11",
        "--input",
        &file,
        "--dry-run",
        "--json",
    ]));
    assert_eq!(plan["body"]["permission"], "read");
    assert_eq!(plan["body"]["units_map"], json!({"repo.code":"write"}));
    mock.finish();
    let mut owners = team();
    owners["permission"] = json!("owner");
    let mock = Mock::new(vec![response("200 OK", &owners.to_string(), "")]);
    let file = input(&mock, &json!({"permission":"admin"}));
    assert_failure(&mock.run(&["team", "edit", "11", "--input", &file]), 6);
    assert_eq!(mock.finish().len(), 1);
}

#[test]
fn failed_writes_and_redirects_are_never_retried() {
    for status in ["503 Unavailable", "302 Found"] {
        let mock = Mock::new(vec![response(
            status,
            "{}",
            "Location: /forgejo/api/v1/user\r\n",
        )]);
        assert_failure(&mock.run(&["team", "add-member", "11", "alice"]), 5);
        assert_eq!(mock.finish().len(), 1);
    }
}

#[test]
fn plain_fields_escape_controls_and_json_preserves_them() {
    let mut value = org();
    value["full_name"] = json!("A\tB\nC\\D");
    let mock = Mock::new(vec![
        response("200 OK", &value.to_string(), ""),
        response("200 OK", &value.to_string(), ""),
    ]);
    let output = mock.run(&["org", "view", "acme"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "7\tacme\tprivate\tA\\tB\\nC\\\\D\n"
    );
    assert_eq!(
        decode(&mock.run(&["org", "view", "acme", "--json"]))["full_name"],
        "A\tB\nC\\D"
    );
    mock.finish();
}

#[test]
fn focused_help_and_schema_need_no_auth_and_match_new_commands() {
    let mock = Mock::new(vec![]);
    for group in ["org", "team"] {
        let schema = decode(&mock.run(&["schema", group, "--json"]));
        for command in schema["commands"]
            .as_array()
            .unwrap_or_else(|| panic!("commands"))
        {
            let name = command["name"].as_str().unwrap_or_default();
            let words: Vec<_> = name.split_whitespace().collect();
            let output = Command::new(env!("CARGO_BIN_EXE_fjx"))
                .args([words[0], words[1], "--help"])
                .env("FJX_HOST", "invalid")
                .env("FJX_TOKEN", "invalid\ntoken")
                .env("FJX_CONFIG", "/does/not/exist")
                .output()
                .unwrap_or_else(|e| panic!("{e}"));
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains(command["usage"].as_str().unwrap_or_default())
            );
        }
    }
    assert!(mock.finish().is_empty());
}

#[test]
fn ambiguous_create_disconnect_is_not_retried() {
    let mock = Mock::new(vec![String::new()]);
    let file = input(&mock, &json!({"username":"acme"}));
    assert_failure(&mock.run(&["org", "create", "--input", &file]), 4);
    let requests = mock.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /forgejo/api/v1/orgs "));
}
