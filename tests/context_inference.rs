use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-context-inference-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn serve(body: &'static str) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
        let request = read_request(&mut stream);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .unwrap_or_else(|error| panic!("{error}"));
        request
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

fn command(root: &Path, bin: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    command
        .current_dir(root)
        .env("PATH", bin)
        .env("HOME", root)
        .env("FJX_CONFIG", root.join("hosts.json"))
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME");
    command
}

#[cfg(unix)]
fn write_tool(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explicit_host_skips_missing_vcs_tools_and_reaches_raw_api() {
    let root = temp_dir("host-flag");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    let (host, server) = serve(r#"{"version":"15.0.7"}"#);

    let output = command(&root, &bin)
        .args(["api", "version", "--json", "--host", &host])
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"version\":\"15.0.7\"}\n"
    );
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.starts_with("GET /api/v1/version HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn host_env_skips_failing_vcs_tools_and_reaches_auth_api() {
    let root = temp_dir("host-env");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    let calls = root.join("vcs-calls");
    let body = format!("printf '%s\\n' \"$0\" >> '{}'\nexit 71\n", calls.display());
    write_tool(&bin, "jj", &body);
    write_tool(&bin, "git", &body);
    let (host, server) = serve(r#"{"login":"dalton"}"#);

    let output = command(&root, &bin)
        .args(["auth", "status", "--json"])
        .env("FJX_HOST", &host)
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"user\":\"dalton\""));
    assert!(!calls.exists(), "repo-free command invoked a VCS tool");
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.starts_with("GET /api/v1/user HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn repo_required_command_uses_jj_to_infer_missing_repo() {
    let root = temp_dir("repo-required");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    let calls = root.join("vcs-calls");
    let (host, server) = serve(include_str!("fixtures/forgejo-15.0.7/repository.json"));
    write_tool(
        &bin,
        "jj",
        &format!(
            "if [ \"$*\" != \"--ignore-working-copy git remote list\" ]; then exit 70; fi\nprintf '%s\\n' jj >> '{}'\nprintf '%s\\n' 'origin {host}/dalton/monolith'\n",
            calls.display()
        ),
    );
    write_tool(
        &bin,
        "git",
        &format!("printf '%s\\n' git >> '{}'\nexit 71\n", calls.display()),
    );

    let output = command(&root, &bin)
        .args(["repo", "view", "--json", "--host", &host])
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_success(&output);
    assert_eq!(
        fs::read_to_string(&calls).unwrap_or_else(|error| panic!("{error}")),
        "jj\n"
    );
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.starts_with("GET /api/v1/repos/dalton/monolith HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn unresolved_host_checks_jj_then_git_and_uses_the_git_remote() {
    let root = temp_dir("host-inference");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    let calls = root.join("vcs-calls");
    let (host, server) = serve(r#"{"version":"15.0.7"}"#);
    write_tool(
        &bin,
        "jj",
        &format!(
            "if [ \"$*\" != \"--ignore-working-copy git remote list\" ]; then exit 70; fi\nprintf '%s\\n' jj >> '{}'\n",
            calls.display()
        ),
    );
    write_tool(
        &bin,
        "git",
        &format!(
            "if [ \"$*\" != \"remote -v\" ] || [ \"$LC_ALL\" != C ]; then exit 70; fi\nprintf '%s\\n' git >> '{}'\nprintf '%s\\n' 'origin {host}/dalton/monolith (fetch)'\n",
            calls.display()
        ),
    );

    let output = command(&root, &bin)
        .args(["api", "version", "--json"])
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_success(&output);
    assert_eq!(
        fs::read_to_string(&calls).unwrap_or_else(|error| panic!("{error}")),
        "jj\ngit\n"
    );
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.starts_with("GET /api/v1/version HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
