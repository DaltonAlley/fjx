use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
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
        "fjx-token-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn assert_rejected_without_token(output: &Output, token: &str, fragments: &[&str]) {
    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(token), "token leaked in stderr: {stderr}");
    for fragment in fragments {
        assert!(
            !stderr.contains(fragment),
            "token fragment leaked in stderr: {stderr}"
        );
    }
}

#[cfg(unix)]
fn write_config(path: &Path, host: &str, token_json: &str) {
    fs::write(
        path,
        format!(
            "{{\"version\":1,\"default_host\":\"{host}\",\"hosts\":{{\"{host}\":{{\"token\":{token_json}}}}}}}"
        ),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));
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

#[test]
fn unsafe_environment_token_is_rejected_without_disclosure() {
    let root = temp_dir("env");
    let config = root.join("hosts.json");
    let token = "env-secret\nforged-header";
    let output = command()
        .args(["api", "version", "--host", "http://127.0.0.1:1"])
        .env("FJX_TOKEN", token)
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_rejected_without_token(&output, token, &["env-secret", "forged-header"]);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn unsafe_config_token_is_rejected_without_disclosure() {
    let root = temp_dir("config");
    let config = root.join("hosts.json");
    let host = "http://127.0.0.1:1";
    let token = "config-secret-\u{00e9}";
    write_config(&config, host, "\"config-secret-\\u00e9\"");

    let output = command()
        .args(["auth", "status", "--host", host])
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_rejected_without_token(&output, token, &["config-secret"]);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn unsafe_login_token_is_rejected_before_http_without_disclosure() {
    let root = temp_dir("login");
    let config = root.join("hosts.json");
    let token = "login-secret\tvalue";
    let mut child = command()
        .args([
            "auth",
            "login",
            "--with-token",
            "--host",
            "http://127.0.0.1:1",
        ])
        .env("FJX_CONFIG", config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{error}"));
    writeln!(
        child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("missing stdin")),
        "{token}"
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_rejected_without_token(&output, token, &["login-secret", "value"]);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn transport_errors_do_not_disclose_a_valid_token() {
    let root = temp_dir("transport");
    let config = root.join("hosts.json");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    drop(listener);
    let host = format!("http://{address}");
    let token = "transport-secret";

    let output = command()
        .args(["api", "version", "--host", &host])
        .env("FJX_TOKEN", token)
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "fjx: Forgejo request failed\n"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains(token));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn valid_environment_token_still_reaches_the_selected_host() {
    let root = temp_dir("valid");
    let config = root.join("hosts.json");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
        let request = read_request(&mut stream);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"version\":\"15\"}",
            )
            .unwrap_or_else(|error| panic!("{error}"));
        request
    });
    let host = format!("http://{address}");

    let output = command()
        .args(["api", "version", "--json", "--host", &host])
        .env("FJX_TOKEN", "valid-test-token")
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"version\":\"15\"}\n"
    );
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.contains("Authorization: token valid-test-token\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
