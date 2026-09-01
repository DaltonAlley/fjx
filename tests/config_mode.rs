#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-config-mode-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn strict_umask_command(arguments: &[&str], host: &str, config: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "umask 0777; exec \"$@\"", "sh"])
        .arg(env!("CARGO_BIN_EXE_fjx"))
        .args(arguments)
        .args(["--host", host])
        .env("FJX_CONFIG", config)
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME");
    command
}

fn assert_success_without_token(output: &Output, token: &str) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(token));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(token));
}

fn assert_exact_mode_0600(path: &Path) {
    let metadata = fs::metadata(path).unwrap_or_else(|error| panic!("{error}"));
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn login_and_logout_write_readable_mode_0600_config_under_strict_umask() {
    let root = temp_dir();
    let config = root.join("config/hosts.json");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let host = format!(
        "http://{}",
        listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"))
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"login\":\"dalton\"}",
            )
            .unwrap_or_else(|error| panic!("{error}"));
    });
    let token = "strict-umask-secret";
    let mut login =
        strict_umask_command(&["auth", "login", "--with-token", "--json"], &host, &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("{error}"));
    writeln!(
        login
            .stdin
            .take()
            .unwrap_or_else(|| panic!("missing stdin")),
        "{token}"
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let login_output = login
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_success_without_token(&login_output, token);
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert_exact_mode_0600(&config);
    assert!(
        fs::read_to_string(&config)
            .unwrap_or_else(|error| panic!("{error}"))
            .contains(token)
    );

    let logout_output = strict_umask_command(&["auth", "logout", "--json"], &host, &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_success_without_token(&logout_output, token);
    assert_exact_mode_0600(&config);
    assert!(
        !fs::read_to_string(&config)
            .unwrap_or_else(|error| panic!("{error}"))
            .contains(token)
    );

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
