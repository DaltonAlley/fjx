use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
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

fn temp_dir(label: &str) -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-paging-header-{label}-{}-{stamp}-{sequence}",
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

fn json_response(body: &str, total_header: Option<&str>) -> String {
    let total_header =
        total_header.map_or_else(String::new, |value| format!("X-Total-Count: {value}\r\n"));
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{total_header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn run(args: &[&str], host: &str, config: &Path) -> Output {
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

fn run_page() -> String {
    format!(
        "{{\"total_count\":1,\"workflow_runs\":[{}]}}",
        include_str!("fixtures/forgejo-15.0.7/action-run.json")
    )
}

fn pull_response() -> String {
    "{\"number\":8,\"title\":\"change\",\"body\":\"\",\"html_url\":\"https://forge.example/dalton/monolith/pulls/8\",\"user\":{\"login\":\"dalton\"},\"state\":\"open\",\"draft\":false,\"mergeable\":true,\"base\":{\"ref\":\"main\",\"sha\":\"base\"},\"head\":{\"ref\":\"feature\",\"sha\":\"abc123\"},\"created_at\":\"2026-08-27T00:00:00Z\",\"updated_at\":\"2026-08-27T00:00:00Z\"}".to_owned()
}

fn checks_response() -> String {
    "{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{\"context\":\"build\",\"status\":\"success\",\"description\":null,\"target_url\":null}],\"total_count\":1}".to_owned()
}

fn assert_invalid_total(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(7),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("X-Total-Count"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn invalid_totals() -> [(&'static str, &'static str); 2] {
    [
        ("malformed", "not-a-number"),
        ("overflow", "999999999999999999999999999999999999999999"),
    ]
}

#[test]
fn raw_paging_rejects_present_invalid_total_headers() {
    for (label, total) in invalid_totals() {
        let root = temp_dir(&format!("raw-{label}"));
        let config = root.join("hosts.json");
        let (host, server) = serve(vec![json_response("[1]", Some(total))]);

        let output = run(&["api", "items", "--paginate", "--json"], &host, &config);

        assert_invalid_total(&output);
        assert_eq!(
            server
                .join()
                .unwrap_or_else(|_| panic!("server panicked"))
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn typed_paging_rejects_present_invalid_total_headers() {
    for (label, total) in invalid_totals() {
        let root = temp_dir(&format!("typed-{label}"));
        let config = root.join("hosts.json");
        let (host, server) = serve(vec![json_response(&issue_page(), Some(total))]);

        let output = run(
            &["issue", "list", "--all", "--json", "-R", "dalton/monolith"],
            &host,
            &config,
        );

        assert_invalid_total(&output);
        assert_eq!(
            server
                .join()
                .unwrap_or_else(|_| panic!("server panicked"))
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn run_paging_rejects_present_invalid_total_headers() {
    for (label, total) in invalid_totals() {
        let root = temp_dir(&format!("run-{label}"));
        let config = root.join("hosts.json");
        let (host, server) = serve(vec![json_response(&run_page(), Some(total))]);

        let output = run(
            &["run", "list", "--all", "--json", "-R", "dalton/monolith"],
            &host,
            &config,
        );

        assert_invalid_total(&output);
        assert_eq!(
            server
                .join()
                .unwrap_or_else(|_| panic!("server panicked"))
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn checks_paging_rejects_present_invalid_total_headers() {
    for (label, total) in invalid_totals() {
        let root = temp_dir(&format!("checks-{label}"));
        let config = root.join("hosts.json");
        let (host, server) = serve(vec![
            json_response(&pull_response(), None),
            json_response(&checks_response(), Some(total)),
        ]);

        let output = run(
            &["pr", "checks", "8", "--json", "-R", "dalton/monolith"],
            &host,
            &config,
        );

        assert_invalid_total(&output);
        assert_eq!(
            server
                .join()
                .unwrap_or_else(|_| panic!("server panicked"))
                .len(),
            2
        );
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn absent_total_headers_remain_valid() {
    let root = temp_dir("absent");
    let config = root.join("hosts.json");

    let cases = [
        (
            vec!["api", "items", "--paginate", "--json"],
            vec![json_response("[1]", None)],
            1,
        ),
        (
            vec!["issue", "list", "--all", "--json", "-R", "dalton/monolith"],
            vec![json_response(&issue_page(), None)],
            1,
        ),
        (
            vec!["run", "list", "--all", "--json", "-R", "dalton/monolith"],
            vec![json_response(&run_page(), None)],
            1,
        ),
        (
            vec!["pr", "checks", "8", "--json", "-R", "dalton/monolith"],
            vec![
                json_response(&pull_response(), None),
                json_response(&checks_response(), None),
            ],
            2,
        ),
    ];

    for (args, responses, expected_requests) in cases {
        let (host, server) = serve(responses);
        let output = run(&args, &host, &config);

        assert!(
            output.status.success(),
            "args: {args:?}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
        assert_eq!(
            server
                .join()
                .unwrap_or_else(|_| panic!("server panicked"))
                .len(),
            expected_requests
        );
    }

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
