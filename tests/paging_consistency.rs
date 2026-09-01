use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    command
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME");
    command
}

fn temp_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-paging-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn serve(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
            requests.push(read_request(&mut stream));
            stream
                .write_all(response.as_bytes())
                .unwrap_or_else(|error| panic!("{error}"));
        }
        requests
    });
    (format!("http://{address}"), handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("{error}"));
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn json_response(body: &str, headers: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn run(args: &[&str], host: &str, config: &PathBuf) -> Output {
    command()
        .args(args)
        .args(["--host", host])
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn issue_page() -> String {
    format!("[{}]", include_str!("fixtures/forgejo-15.0.7/issue.json"))
}

fn run_page(total: usize) -> String {
    format!(
        "{{\"total_count\":{total},\"workflow_runs\":[{}]}}",
        include_str!("fixtures/forgejo-15.0.7/action-run.json")
    )
}

fn assert_data_error(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("total"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn raw_paging_retains_an_earlier_total() {
    let root = temp_dir("raw-retained");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![
        json_response("[1]", "X-Total-Count: 3\r\n"),
        json_response("[2]", ""),
        json_response("[3]", ""),
    ]);

    let output = run(&["api", "items", "--paginate", "--json"], &host, &config);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[1,2,3]\n");
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert_eq!(requests.len(), 3);
    assert!(requests[2].starts_with("GET /api/v1/items?page=3&limit=50 HTTP/1.1"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_paging_retains_an_earlier_total() {
    let root = temp_dir("typed-retained");
    let config = root.join("hosts.json");
    let page = issue_page();
    let (host, server) = serve(vec![
        json_response(&page, "X-Total-Count: 3\r\n"),
        json_response(&page, ""),
        json_response(&page, ""),
    ]);

    let output = run(
        &["issue", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(values.len(), 3);
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert_eq!(requests.len(), 3);
    assert!(requests[2].contains("page=3&limit=30 HTTP/1.1"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn raw_paging_rejects_a_shrinking_total() {
    let root = temp_dir("raw-shrink");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![
        json_response("[1]", "X-Total-Count: 2\r\n"),
        json_response("[2]", "X-Total-Count: 1\r\n"),
    ]);

    let output = run(&["api", "items", "--paginate", "--json"], &host, &config);

    assert_data_error(&output);
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_paging_rejects_a_shrinking_total() {
    let root = temp_dir("typed-shrink");
    let config = root.join("hosts.json");
    let page = issue_page();
    let (host, server) = serve(vec![
        json_response(&page, "X-Total-Count: 2\r\n"),
        json_response(&page, "X-Total-Count: 1\r\n"),
    ]);

    let output = run(
        &["issue", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );

    assert_data_error(&output);
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn run_paging_rejects_a_shrinking_body_total() {
    let root = temp_dir("run-shrink");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![
        json_response(&run_page(2), ""),
        json_response(&run_page(1), ""),
    ]);

    let output = run(
        &["run", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );

    assert_data_error(&output);
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn run_paging_rejects_body_and_header_total_conflict() {
    let root = temp_dir("run-conflict");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![json_response(&run_page(1), "X-Total-Count: 2\r\n")]);

    let output = run(
        &["run", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );

    assert_data_error(&output);
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
