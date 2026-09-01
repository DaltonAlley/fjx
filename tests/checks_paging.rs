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
        "fjx-checks-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn serve(responses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
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

fn json_response(body: &str, headers: &str) -> &'static str {
    Box::leak(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_boxed_str(),
    )
}

fn pull_response(head_sha: &str) -> &'static str {
    let body = format!(
        "{{\"number\":8,\"title\":\"change\",\"body\":\"\",\"html_url\":\"https://forge.example/dalton/monolith/pulls/8\",\"user\":{{\"login\":\"dalton\"}},\"state\":\"open\",\"draft\":false,\"mergeable\":true,\"base\":{{\"ref\":\"main\",\"sha\":\"base\"}},\"head\":{{\"ref\":\"feature\",\"sha\":{}}},\"created_at\":\"2026-08-27T00:00:00Z\",\"updated_at\":\"2026-08-27T00:00:00Z\"}}",
        serde_json::to_string(head_sha).unwrap_or_else(|error| panic!("{error}"))
    );
    json_response(&body, "")
}

fn run_checks(host: &str, config: &PathBuf) -> Output {
    command()
        .args([
            "pr",
            "checks",
            "8",
            "--json",
            "--host",
            host,
            "-R",
            "dalton/monolith",
        ])
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn status(context: &str) -> String {
    format!(
        "{{\"context\":\"{context}\",\"status\":\"success\",\"description\":null,\"target_url\":null}}"
    )
}

#[test]
fn checks_collects_all_pages_and_encodes_the_ref_path() {
    let root = temp_dir("all-pages");
    let config = root.join("hosts.json");
    let first = format!(
        "{{\"sha\":\"feature/a b\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":2}}",
        status("build")
    );
    let second = format!(
        "{{\"sha\":\"feature/a b\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":2}}",
        status("test")
    );
    let (host, server) = serve(vec![
        pull_response("feature/a b"),
        json_response(&first, ""),
        json_response(&second, ""),
    ]);

    let output = run_checks(&host, &config);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"checks\",\"sha\":\"feature/a b\",\"state\":\"success\",\"statuses\":[{\"context\":\"build\",\"state\":\"success\",\"description\":null,\"target_url\":null},{\"context\":\"test\",\"state\":\"success\",\"description\":null,\"target_url\":null}]}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/dalton/monolith/commits/feature%2Fa%20b/status?page=1&limit=50 HTTP/1.1"
    ));
    assert!(requests[2].starts_with(
        "GET /api/v1/repos/dalton/monolith/commits/feature%2Fa%20b/status?page=2&limit=50 HTTP/1.1"
    ));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn checks_retains_the_first_pages_total_count() {
    let root = temp_dir("retained-total");
    let config = root.join("hosts.json");
    let first = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":3}}",
        status("build")
    );
    let second = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}]}}",
        status("test")
    );
    let third = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}]}}",
        status("lint")
    );
    let (host, server) = serve(vec![
        pull_response("abc123"),
        json_response(&first, ""),
        json_response(&second, ""),
        json_response(&third, ""),
    ]);

    let output = run_checks(&host, &config);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"checks\",\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{\"context\":\"build\",\"state\":\"success\",\"description\":null,\"target_url\":null},{\"context\":\"test\",\"state\":\"success\",\"description\":null,\"target_url\":null},{\"context\":\"lint\",\"state\":\"success\",\"description\":null,\"target_url\":null}]}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[3].starts_with(
        "GET /api/v1/repos/dalton/monolith/commits/abc123/status?page=3&limit=50 HTTP/1.1"
    ));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn checks_rejects_an_empty_page_that_claims_another_page() {
    let root = temp_dir("empty-next");
    let config = root.join("hosts.json");
    let body = "{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[],\"total_count\":1}";
    let (host, server) = serve(vec![
        pull_response("abc123"),
        json_response(body, "Link: </next>; rel=\"next\"\r\nX-Total-Count: 1\r\n"),
    ]);

    let output = run_checks(&host, &config);

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("another page"));
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn checks_rejects_metadata_that_changes_between_pages() {
    for (label, second_sha, second_state) in
        [("sha", "other", "success"), ("state", "abc123", "failure")]
    {
        let root = temp_dir(label);
        let config = root.join("hosts.json");
        let first = format!(
            "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":2}}",
            status("build")
        );
        let second = format!(
            "{{\"sha\":\"{second_sha}\",\"state\":\"{second_state}\",\"statuses\":[{}],\"total_count\":2}}",
            status("test")
        );
        let (host, server) = serve(vec![
            pull_response("abc123"),
            json_response(&first, ""),
            json_response(&second, ""),
        ]);

        let output = run_checks(&host, &config);

        assert_eq!(output.status.code(), Some(7));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains(label)
        );
        server.join().unwrap_or_else(|_| panic!("server panicked"));
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn checks_rejects_conflicting_changed_or_exceeded_totals() {
    let root = temp_dir("conflicting-total");
    let config = root.join("hosts.json");
    let body = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":2}}",
        status("build")
    );
    let (host, server) = serve(vec![
        pull_response("abc123"),
        json_response(&body, "X-Total-Count: 3\r\n"),
    ]);
    let output = run_checks(&host, &config);
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("disagree"));
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let first = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":2}}",
        status("build")
    );
    let second = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":3}}",
        status("test")
    );
    let (host, server) = serve(vec![
        pull_response("abc123"),
        json_response(&first, ""),
        json_response(&second, ""),
    ]);
    let output = run_checks(&host, &config);
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed"));
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let body = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{}],\"total_count\":0}}",
        status("build")
    );
    let (host, server) = serve(vec![pull_response("abc123"), json_response(&body, "")]);
    let output = run_checks(&host, &config);
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds"));
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn checks_refuses_a_partial_result_at_one_thousand_statuses() {
    let root = temp_dir("cap");
    let config = root.join("hosts.json");
    let statuses = (0..50)
        .map(|index| status(&format!("check-{index}")))
        .collect::<Vec<_>>()
        .join(",");
    let body = format!(
        "{{\"sha\":\"abc123\",\"state\":\"success\",\"statuses\":[{statuses}],\"total_count\":1001}}"
    );
    let page = json_response(&body, "");
    let mut responses = vec![pull_response("abc123")];
    responses.extend(vec![page; 20]);
    let (host, server) = serve(responses);

    let output = run_checks(&host, &config);

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("1,000"));
    assert_eq!(
        server
            .join()
            .unwrap_or_else(|_| panic!("server panicked"))
            .len(),
        21
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
