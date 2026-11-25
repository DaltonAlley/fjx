//! Board commands against responses recorded from Gitea 28.2.0, and against
//! a forge that serves no project routes, as Forgejo 17 does.
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
const BASE: &str = "/api/v1/repos/o/r";

struct Mock {
    host: String,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}

impl Mock {
    fn serve(mut handler: impl FnMut(&str, &str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("{error}"));
        let host = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
        );
        let root = std::env::temp_dir().join(format!(
            "fjx-board-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap_or_else(|error| panic!("{error}"));
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap_or_else(|error| panic!("{error}"));
                        let request = read_request(&mut stream);
                        let line = request.lines().next().unwrap_or_default().to_owned();
                        let mut words = line.split(' ');
                        let method = words.next().unwrap_or_default();
                        let target = words.next().unwrap_or_default();
                        let response = handler(method, target);
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

    /// A Gitea with the recorded board, optionally overridden per request.
    fn gitea(overrides: impl Fn(&str, &str) -> Option<String> + Send + 'static) -> Self {
        Self::serve(move |method, target| {
            overrides(method, target).unwrap_or_else(|| gitea(method, target))
        })
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

    /// The request lines the forge received, in order.
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

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn ok(value: &Value) -> String {
    response("200 OK", &value.to_string())
}

fn not_found() -> String {
    response("404 Not Found", r#"{"message":"not found"}"#)
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}"))
}

fn boards() -> Value {
    fixture(include_str!("fixtures/gitea-28.2.0/boards.json"))
}

fn columns() -> Value {
    fixture(include_str!("fixtures/gitea-28.2.0/board-columns.json"))
}

/// The recorded "In Progress" column: issue number 16, forge-wide ID 756.
fn cards() -> Value {
    fixture(include_str!("fixtures/gitea-28.2.0/board-cards.json"))
}

fn closed_board() -> Value {
    let mut board = boards()[0].clone();
    board["id"] = json!(3);
    board["title"] = json!("Archive");
    board["state"] = json!("closed");
    board
}

fn gitea(method: &str, target: &str) -> String {
    let Some(route) = target.strip_prefix(BASE) else {
        return not_found();
    };
    let (path, query) = route.split_once('?').unwrap_or((route, ""));
    match (method, path) {
        ("GET", "") => ok(&json!({"full_name":"o/r","has_projects":true})),
        ("GET", "/projects") if query.starts_with("state=open&") => ok(&boards()),
        ("GET", "/projects") if query.starts_with("state=all&") => {
            ok(&json!([boards()[0], closed_board()]))
        }
        ("GET", "/projects/7") => ok(&boards()[0]),
        ("GET", "/projects/3") => ok(&closed_board()),
        ("GET", "/projects/7/columns" | "/projects/3/columns") => ok(&columns()),
        ("GET", "/projects/7/columns/23/issues") => ok(&cards()),
        ("GET", path) if path.starts_with("/projects/") && path.ends_with("/issues") => {
            ok(&json!([]))
        }
        ("GET", "/issues/16") => ok(&cards()[0]),
        ("POST", path) if path.starts_with("/projects/7/columns/") => response("201 Created", ""),
        ("DELETE", path) if path.starts_with("/projects/7/columns/") => {
            response("204 No Content", "")
        }
        ("POST", "/issues") => response(
            "201 Created",
            include_str!("fixtures/forgejo-15.0.7/issue.json"),
        ),
        _ => not_found(),
    }
}

fn decode(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"))
}

fn assert_failure(output: &Output, code: i32, message: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(code), "{stderr}");
    assert!(output.stdout.is_empty(), "partial stdout on failure");
    assert!(stderr.contains(message), "{stderr}");
}

/// `METHOD /path?query` for each request, without the API prefix.
fn lines(requests: &[String]) -> Vec<String> {
    requests
        .iter()
        .map(|request| {
            let line = request.lines().next().unwrap_or_default();
            line.trim_end_matches(" HTTP/1.1").replacen(BASE, "", 1)
        })
        .collect()
}

fn writes(requests: &[String]) -> Vec<String> {
    lines(requests)
        .into_iter()
        .filter(|line| !line.starts_with("GET "))
        .collect()
}

fn body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |parts| parts.1)
}

const PAGE: &str = "page=1&limit=50";

#[test]
fn view_reports_columns_in_board_order_with_their_cards() {
    let mock = Mock::gitea(|_, _| None);
    let view = decode(&mock.run(&["board", "view", "--json"]));
    let titles: Vec<_> = view
        .as_array()
        .unwrap_or_else(|| panic!("columns"))
        .iter()
        .map(|column| column["title"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        titles,
        ["Backlog", "To Do", "In Progress", "Blocked", "Done"]
    );
    assert_eq!(
        view[2],
        json!({"kind":"column","board":7,"id":23,"title":"In Progress","default":false,"cards":[{
            "number":16,"state":"open","title":"Add the protocol skeleton","pull_request":false,
            "html_url":"https://gitea.example/owner/project/issues/16"}]})
    );
    assert_eq!(view[0]["default"], true);
    assert_eq!(view[4]["cards"], json!([]));

    let plain = mock.run(&["board", "view"]);
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout),
        "column\t21\tBacklog\t0\ncolumn\t22\tTo Do\t0\ncolumn\t23\tIn Progress\t1\n\
         card\tIn Progress\t16\topen\tAdd the protocol skeleton\n\
         column\t31\tBlocked\t0\ncolumn\t24\tDone\t0\n"
    );
    assert_eq!(
        decode(&mock.run(&["board", "view", "--json", "--fields", "title,default"]))[3],
        json!({"title":"Blocked","default":false})
    );
    let requests = lines(&mock.finish());
    assert_eq!(
        requests[..7],
        [
            format!("GET /projects?state=open&{PAGE}"),
            format!("GET /projects/7/columns?{PAGE}"),
            format!("GET /projects/7/columns/21/issues?{PAGE}"),
            format!("GET /projects/7/columns/22/issues?{PAGE}"),
            format!("GET /projects/7/columns/23/issues?{PAGE}"),
            format!("GET /projects/7/columns/31/issues?{PAGE}"),
            format!("GET /projects/7/columns/24/issues?{PAGE}"),
        ]
    );
    assert_eq!(requests.len(), 21);
}

#[test]
fn list_normalizes_boards_and_forwards_state_and_paging() {
    let mock = Mock::gitea(|_, _| None);
    assert_eq!(
        decode(&mock.run(&["board", "list", "--json"])),
        json!([{"kind":"board","id":7,"title":"Development","description":"","state":"open",
            "open_issues":22,"closed_issues":0,
            "html_url":"https://gitea.example/owner/project/projects/7",
            "created_at":"2026-10-30T11:45:01-04:00","updated_at":"2026-11-25T16:04:46-05:00"}])
    );
    let all = mock.run(&["board", "list", "--state", "all", "--limit", "5"]);
    assert_eq!(
        String::from_utf8_lossy(&all.stdout),
        "7\topen\tDevelopment\t22\t0\thttps://gitea.example/owner/project/projects/7\n\
         3\tclosed\tArchive\t22\t0\thttps://gitea.example/owner/project/projects/7\n"
    );
    assert_eq!(
        lines(&mock.finish()),
        [
            "GET /projects?state=open&page=1&limit=30",
            "GET /projects?state=all&page=1&limit=5",
        ]
    );
}

#[test]
fn move_addresses_the_card_by_internal_issue_id_and_column_id() {
    let mock = Mock::gitea(|_, _| None);
    // The title matches ignoring case; the issue number becomes its global ID.
    assert_eq!(
        decode(&mock.run(&["board", "move", "16", "to do", "--json"])),
        json!({"kind":"result","action":"board.move","ok":true,"number":16,"board":7,
            "column":"To Do","column_id":22})
    );
    let plain = mock.run(&["board", "move", "16", "Done"]);
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout),
        "board.move\t16\tDone\tok\n"
    );
    let requests = mock.finish();
    assert_eq!(
        lines(&requests)[..4],
        [
            format!("GET /projects?state=open&{PAGE}"),
            format!("GET /projects/7/columns?{PAGE}"),
            "GET /issues/16".to_owned(),
            "POST /projects/7/columns/22/issues/756".to_owned(),
        ]
    );
    assert_eq!(body(&requests[3]), "");
    assert_eq!(
        writes(&requests),
        [
            "POST /projects/7/columns/22/issues/756",
            "POST /projects/7/columns/24/issues/756"
        ]
    );
}

#[test]
fn remove_finds_the_column_holding_the_card() {
    let mock = Mock::gitea(|_, _| None);
    assert_eq!(
        decode(&mock.run(&["board", "remove", "16", "--json"])),
        json!({"kind":"result","action":"board.remove","ok":true,"number":16,"board":7,
            "column":"In Progress","column_id":23})
    );
    // An issue that is not on the board is reported; nothing is deleted for it.
    assert_failure(
        &mock.run(&["board", "remove", "17"]),
        2,
        "issue #17 has no card on board 7",
    );
    let requests = mock.finish();
    assert_eq!(
        lines(&requests)[..6],
        [
            format!("GET /projects?state=open&{PAGE}"),
            format!("GET /projects/7/columns?{PAGE}"),
            format!("GET /projects/7/columns/21/issues?{PAGE}"),
            format!("GET /projects/7/columns/22/issues?{PAGE}"),
            format!("GET /projects/7/columns/23/issues?{PAGE}"),
            "DELETE /projects/7/columns/23/issues/756".to_owned(),
        ]
    );
    assert_eq!(
        writes(&requests),
        ["DELETE /projects/7/columns/23/issues/756"]
    );
}

#[test]
fn dry_runs_resolve_the_request_without_writing() {
    let mock = Mock::gitea(|_, _| None);
    let host = mock.host.clone();
    assert_eq!(
        decode(&mock.run(&["board", "move", "16", "DONE", "--dry-run", "--json"])),
        json!({"kind":"request","method":"POST","body":null,
            "url":format!("{host}{BASE}/projects/7/columns/24/issues/756")})
    );
    let remove = mock.run(&["board", "remove", "16", "--dry-run"]);
    assert_eq!(
        String::from_utf8_lossy(&remove.stdout),
        format!("DELETE\t{host}{BASE}/projects/7/columns/23/issues/756\tnull\n")
    );
    assert_eq!(
        decode(&mock.run(&[
            "issue",
            "create",
            "--title",
            "New",
            "--board",
            "development",
            "--dry-run",
            "--json"
        ]))["body"],
        json!({"projects":[7],"title":"New"})
    );
    assert!(writes(&mock.finish()).is_empty());
}

#[test]
fn a_board_is_implied_only_when_exactly_one_is_open() {
    let none = Mock::gitea(|_, target| target.contains("state=open").then(|| ok(&json!([]))));
    assert_failure(
        &none.run(&["board", "move", "16", "Done"]),
        2,
        "o/r has no open board",
    );
    assert!(writes(&none.finish()).is_empty());

    let two = Mock::gitea(|_, target| {
        target.contains("state=open").then(|| {
            let mut second = boards()[0].clone();
            second["id"] = json!(9);
            second["title"] = json!("Releases");
            ok(&json!([boards()[0], second]))
        })
    });
    assert_failure(
        &two.run(&["board", "view"]),
        2,
        "o/r has 2 open boards; pass --board TITLE or --board-id ID: 7 (\"Development\", open), 9 (\"Releases\", open)",
    );
    // Naming the board settles it, by ID or by title in any letter case.
    assert_eq!(
        decode(&two.run(&["board", "move", "16", "Done", "--board-id", "7", "--json"]))["board"],
        7
    );
    assert_eq!(
        decode(&two.run(&["board", "view", "--board", "ARCHIVE", "--json"]))[0]["board"],
        3
    );
    let requests = lines(&two.finish());
    assert!(requests.contains(&"GET /projects/7".to_owned()));
    assert!(requests.contains(&format!("GET /projects?state=all&{PAGE}")));
    assert!(requests.contains(&format!("GET /projects/3/columns?{PAGE}")));
}

#[test]
fn unknown_boards_and_columns_fail_before_any_write() {
    let mock = Mock::gitea(|_, _| None);
    assert_failure(
        &mock.run(&["board", "move", "16", "Doing"]),
        2,
        "board 7 has no column titled \"Doing\"; columns: \"Backlog\", \"To Do\", \"In Progress\", \"Blocked\", \"Done\"",
    );
    assert_failure(
        &mock.run(&["board", "view", "--board", "Roadmap"]),
        2,
        "o/r has no board titled \"Roadmap\"; boards: 7 (\"Development\", open), 3 (\"Archive\", closed)",
    );
    // A missing board on a forge that has boards stays an HTTP error.
    assert_failure(
        &mock.run(&["board", "view", "--board-id", "99"]),
        5,
        "o/r has no board with ID 99 (HTTP 404)",
    );
    // The forge rejects an unknown issue; the card write is never attempted.
    assert_failure(
        &mock.run(&["board", "move", "404", "Done"]),
        5,
        "HTTP 404: not found",
    );
    // A numeric-looking title is still a title, never a guessed ID.
    assert_failure(
        &mock.run(&["board", "view", "--board", "7"]),
        2,
        "o/r has no board titled \"7\"",
    );
    assert!(writes(&mock.finish()).is_empty());

    let duplicate = Mock::gitea(|_, target| {
        target.contains("/columns?").then(|| {
            let mut twin = columns()[4].clone();
            twin["id"] = json!(40);
            twin["title"] = json!("DONE");
            ok(&json!([columns()[4], twin]))
        })
    });
    // An exact title still selects one of two that differ only by case.
    assert_eq!(
        decode(&duplicate.run(&["board", "move", "16", "DONE", "--json"]))["column_id"],
        40
    );
    assert_failure(
        &duplicate.run(&["board", "move", "16", "done"]),
        2,
        "more than one column titled \"done\"",
    );
    assert_eq!(
        writes(&duplicate.finish()),
        ["POST /projects/7/columns/40/issues/756"]
    );
}

#[test]
fn a_forge_without_project_routes_is_incompatible_and_receives_no_write() {
    // Forgejo 17: the repository exists and every project route is 404.
    let forgejo = Mock::serve(|method, target| match (method, target) {
        ("GET", BASE) => ok(&json!({"full_name":"o/r","has_projects":true})),
        _ => not_found(),
    });
    for command in [
        &["board", "list"][..],
        &["board", "view"],
        &["board", "view", "--board-id", "7"],
        &["board", "move", "16", "Done"],
        &["board", "move", "16", "Done", "--board", "Development"],
        &["board", "remove", "16", "--dry-run"],
        &["issue", "create", "--title", "New", "--board-id", "7"],
        &[
            "issue",
            "create",
            "--title",
            "New",
            "--board",
            "Development",
        ],
    ] {
        let output = forgejo.run(command);
        assert_failure(&output, 7, "has no project-board API for o/r");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&forgejo.host),
            "{command:?}"
        );
    }
    assert!(writes(&forgejo.finish()).is_empty());

    // A repository that is missing or hidden keeps the forge's own answer.
    let missing = Mock::serve(|_, _| not_found());
    assert_failure(&missing.run(&["board", "list"]), 5, "HTTP 404: not found");
    assert_eq!(
        lines(&missing.finish()),
        [
            "GET /projects?state=open&page=1&limit=30".to_owned(),
            "GET ".to_owned()
        ]
    );
}

#[test]
fn issue_create_sends_the_resolved_board_with_the_issue() {
    let mock = Mock::gitea(|_, _| None);
    let created = decode(&mock.run(&[
        "issue",
        "create",
        "--title",
        "New",
        "--board-id",
        "7",
        "--json",
    ]));
    assert_eq!(created["kind"], "issue");
    // Without --board the request is the one fjx always sent.
    decode(&mock.run(&["issue", "create", "--title", "Plain", "--json"]));
    let requests = mock.finish();
    assert_eq!(
        lines(&requests),
        ["GET /projects/7", "POST /issues", "POST /issues"]
    );
    assert_eq!(
        serde_json::from_str::<Value>(body(&requests[1])).unwrap_or_else(|e| panic!("{e}")),
        json!({"projects":[7],"title":"New"})
    );
    assert_eq!(
        serde_json::from_str::<Value>(body(&requests[2])).unwrap_or_else(|e| panic!("{e}")),
        json!({"title":"Plain"})
    );
}

#[test]
fn failed_card_writes_are_reported_and_never_retried() {
    let mock = Mock::gitea(|method, _| {
        (method == "POST").then(|| response("403 Forbidden", r#"{"message":"no write access"}"#))
    });
    assert_failure(
        &mock.run(&["board", "move", "16", "Done"]),
        5,
        "HTTP 403: no write access",
    );
    assert_eq!(
        writes(&mock.finish()),
        ["POST /projects/7/columns/24/issues/756"]
    );
}

#[test]
fn incompatible_board_data_and_late_failures_leave_stdout_empty() {
    let odd = Mock::gitea(|_, target| {
        target.contains("/columns/23/issues").then(|| {
            let mut card = cards()[0].clone();
            card["state"] = json!("merged");
            ok(&json!([card]))
        })
    });
    assert_failure(&odd.run(&["board", "view"]), 7, "unknown issue state");
    drop(odd);

    let late = Mock::gitea(|_, target| {
        target
            .contains("/columns/24/issues")
            .then(|| response("500 Internal Server Error", "{}"))
    });
    assert_failure(&late.run(&["board", "view", "--json"]), 5, "HTTP 500");
}

#[test]
fn safety_flags_and_malformed_arguments_make_no_requests() {
    let mock = Mock::gitea(|_, _| None);
    for (command, message) in [
        (&["board", "list", "--dry-run"][..], "write safety flags"),
        (&["board", "view", "--yes"], "write safety flags"),
        (
            &["board", "move", "16", "Done", "--yes"],
            "--yes is not valid",
        ),
        (&["board", "remove", "16", "--yes"], "--yes is not valid"),
        (&["board", "move", "16"], "fjx board move --help"),
        (&["board", "move", "0", "Done"], "issue number"),
        (&["board", "move", "16", ""], "column title"),
        (&["board", "remove", "sixteen"], "issue number"),
        (&["board", "view", "7"], "fjx board view --help"),
        (&["board", "view", "--board", ""], "--board"),
        (&["board", "view", "--board-id", "0"], "board ID"),
        (&["board", "view", "--board-id", "seven"], "board ID"),
        (
            &["board", "view", "--board", "Development", "--board-id", "7"],
            "--board conflicts with --board-id",
        ),
        (
            &["board", "list", "--board-id", "7"],
            "--board-id is not valid",
        ),
        (&["board", "list", "--board", "7"], "--board is not valid"),
        (&["board", "view", "--state", "all"], "--state is not valid"),
        (
            &["board", "view", "--json", "--fields", "number"],
            "unknown output field",
        ),
        (&["pr", "list", "--board", "7"], "--board is not valid"),
    ] {
        assert_failure(&mock.run(command), 2, message);
    }
    assert!(mock.finish().is_empty());
}

#[test]
fn focused_help_and_schema_describe_board_commands_without_context() {
    let mock = Mock::gitea(|_, _| None);
    let schema = decode(&mock.run(&["schema", "board", "--json"]));
    let commands = schema["commands"]
        .as_array()
        .unwrap_or_else(|| panic!("commands"));
    let names: Vec<_> = commands
        .iter()
        .map(|command| command["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        ["board list", "board view", "board move", "board remove"]
    );
    for command in commands {
        let words: Vec<_> = command["name"]
            .as_str()
            .unwrap_or_default()
            .split(' ')
            .collect();
        let output = Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args([words[0], words[1], "--help"])
            .env("FJX_HOST", "invalid")
            .env("FJX_TOKEN", "invalid\ntoken")
            .env("FJX_CONFIG", "/does/not/exist")
            .output()
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(output.status.success(), "{words:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains(command["usage"].as_str().unwrap_or_default()));
        assert!(
            text.contains("Forgejo through version 17 has no project-board API"),
            "{text}"
        );
    }
    let moving = &commands[2];
    assert_eq!(
        moving["positionals"],
        json!([{"name":"number","required":true},{"name":"column","required":true}])
    );
    assert_eq!(moving["write"], true);
    assert_eq!(moving["destructive"], false);
    assert!(
        moving["flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|flag| flag
                .as_str()
                .is_some_and(|flag| flag.starts_with("--dry-run"))))
    );
    assert_eq!(
        commands[0]["enum_values"]["--state"],
        json!(["open", "closed", "all"])
    );
    // Help keeps accepting the command's own arguments and flags.
    let output = mock.run(&[
        "board",
        "move",
        "16",
        "In Progress",
        "--board-id",
        "7",
        "--help",
    ]);
    assert!(output.status.success());
    let create = mock.run(&["issue", "create", "--board", "7", "--help"]);
    assert!(String::from_utf8_lossy(&create.stdout).contains("--board TITLE | --board-id ID"));
    assert_eq!(
        commands[1]["mutually_exclusive"],
        json!([["--human", "--json"], ["--board", "--board-id"]])
    );
    assert!(mock.finish().is_empty());
}
