#[cfg(unix)]
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Stdio;
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
        "fjx-{label}-{}-{stamp}-{sequence}",
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
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn run_with_context(arguments: &[&str], host: &str, config: &PathBuf) -> Output {
    command()
        .args(arguments)
        .args(["--host", host])
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

#[cfg(unix)]
fn fake_jj(root: &Path, remotes: &[(&str, &str)]) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap_or_else(|error| panic!("{error}"));
    let script = bin.join("jj");
    let mut body = String::from("#!/bin/sh\n");
    for (name, url) in remotes {
        writeln!(&mut body, "printf '%s\\n' '{name} {url}'")
            .unwrap_or_else(|error| panic!("{error}"));
    }
    fs::write(&script, body).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|error| panic!("{error}"));
    bin
}

#[cfg(unix)]
fn path_with_first(first: PathBuf) -> std::ffi::OsString {
    let mut paths = vec![first];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths).unwrap_or_else(|error| panic!("{error}"))
}

#[cfg(unix)]
fn repository_response() -> &'static str {
    let body = include_str!("fixtures/forgejo-15.0.7/repository.json");
    Box::leak(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_boxed_str(),
    )
}

fn json_response(status: &str, body: &str, headers: &str) -> &'static str {
    Box::leak(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_boxed_str(),
    )
}

#[test]
fn help_version_and_usage_keep_the_output_contract() {
    let package_version = env!("CARGO_PKG_VERSION");
    let help = command()
        .arg("--help")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(help.status.success());
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(help_stdout.starts_with(&format!("fjx {package_version} - a small Forgejo client\n")));
    assert!(help_stdout.contains("fjx api PATH"));
    assert!(help.stderr.is_empty());

    let version = command()
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("fjx {package_version}\n")
    );
    assert!(version.stderr.is_empty());

    let bad = command()
        .arg("unknown")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(bad.status.code(), Some(2));
    assert!(bad.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bad.stderr).starts_with("fjx: "));
}

#[test]
#[cfg(windows)]
fn windows_environment_token_needs_no_home_or_persisted_config() {
    let root = temp_dir("windows-env-only");
    let (host, server) = serve(vec![json_response(
        "200 OK",
        "{\"version\":\"15.0.7\"}",
        "",
    )]);
    let output = command()
        .args(["api", "version", "--json", "--host", &host])
        .env("FJX_TOKEN", "test-secret")
        .env_remove("FJX_CONFIG")
        .env_remove("APPDATA")
        .env_remove("USERPROFILE")
        .env_remove("HOME")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"version\":\"15.0.7\"}\n"
    );
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let persistence = command()
        .args(["auth", "logout", "--host", "https://forge.example"])
        .env("FJX_TOKEN", "test-secret")
        .env_remove("FJX_CONFIG")
        .env_remove("APPDATA")
        .env_remove("USERPROFILE")
        .env_remove("HOME")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(persistence.status.code(), Some(3));
    assert!(persistence.stdout.is_empty());
    assert!(String::from_utf8_lossy(&persistence.stderr).contains("saved tokens are unavailable"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn setup_git_installs_a_host_scoped_fjx_helper() {
    let root = temp_dir("setup-git");
    let config = root.join("hosts.json");
    let git_config = root.join("gitconfig");
    fs::write(
        &config,
        "{\"version\":1,\"default_host\":\"https://forge.example/code\",\"hosts\":{\"https://forge.example/code\":{\"user\":\"agent\",\"token\":\"saved-secret\"}}}",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));

    let output = command()
        .args([
            "auth",
            "setup-git",
            "--host",
            "https://forge.example/code",
            "--json",
        ])
        .env("FJX_CONFIG", &config)
        .env("GIT_CONFIG_GLOBAL", &git_config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"result\",\"action\":\"auth.setup-git\",\"ok\":true,\"number\":null,\"html_url\":null}\n"
    );
    let helpers = Command::new("git")
        .args([
            "config",
            "--file",
            git_config
                .to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 path")),
            "--get-all",
            "credential.https://forge.example/code.helper",
        ])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(helpers.status.success());
    let expected_helper = format!("\n!'{}' auth git-credential\n", env!("CARGO_BIN_EXE_fjx"));
    assert_eq!(String::from_utf8_lossy(&helpers.stdout), expected_helper);
    let use_http_path = Command::new("git")
        .args([
            "config",
            "--file",
            git_config
                .to_str()
                .unwrap_or_else(|| panic!("non-UTF-8 path")),
            "--get",
            "credential.https://forge.example/code.useHttpPath",
        ])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(use_http_path.status.success());
    assert_eq!(String::from_utf8_lossy(&use_http_path.stdout), "true\n");
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn setup_git_upgrades_a_saved_token_with_its_forgejo_user() {
    let root = temp_dir("setup-git-upgrade");
    let config = root.join("hosts.json");
    let git_config = root.join("gitconfig");
    let user = include_str!("fixtures/forgejo-15.0.7/user.json");
    let response = json_response("200 OK", user, "");
    let (host, server) = serve(vec![response]);
    fs::write(
        &config,
        format!(
            "{{\"version\":1,\"default_host\":\"{host}\",\"hosts\":{{\"{host}\":{{\"token\":\"saved-secret\"}}}}}}"
        ),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));

    let output = command()
        .args(["auth", "setup-git", "--host", &host])
        .env("FJX_CONFIG", &config)
        .env("GIT_CONFIG_GLOBAL", &git_config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "auth.setup-git\tok\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with("GET /api/v1/user HTTP/1.1\r\n"));
    assert!(requests[0].contains("Authorization: token saved-secret\r\n"));
    let saved = fs::read_to_string(&config).unwrap_or_else(|error| panic!("{error}"));
    assert!(saved.contains("\"user\":\"dalton\""));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn git_credential_serves_only_the_matching_saved_login() {
    let root = temp_dir("git-credential");
    let config = root.join("hosts.json");
    fs::write(
        &config,
        "{\"version\":1,\"default_host\":\"https://forge.example/code\",\"hosts\":{\"https://forge.example/code\":{\"user\":\"agent\",\"token\":\"saved-secret\"}}}",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));

    let mut child = command()
        .args(["auth", "git-credential", "get"])
        .env("FJX_CONFIG", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{error}"));
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin is missing"))
        .write_all(b"protocol=https\nhost=forge.example\npath=code/owner/repo.git\n\n")
        .unwrap_or_else(|error| panic!("{error}"));
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "username=agent\npassword=saved-secret\n"
    );
    assert!(output.stderr.is_empty());

    let mut child = command()
        .args(["auth", "git-credential", "get"])
        .env("FJX_CONFIG", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{error}"));
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin is missing"))
        .write_all(b"protocol=https\nhost=forge.example\npath=other/repo.git\n\n")
        .unwrap_or_else(|error| panic!("{error}"));
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn raw_api_keeps_server_json_and_sends_token_only_to_selected_host() {
    let root = temp_dir("raw");
    let config = root.join("hosts.json");
    let body = "{\"version\":\"15.0.7\",\"extra\":1}";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response: &'static str = Box::leak(response.into_boxed_str());
    let (host, server) = serve(vec![response]);
    let output = run_with_context(&["api", "/version", "--json"], &host, &config);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"extra\":1,\"version\":\"15.0.7\"}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with("GET /api/v1/version HTTP/1.1\r\n"));
    assert!(requests[0].contains("Authorization: token test-secret\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn repo_view_emits_the_stable_record_and_ignores_new_server_fields() {
    let root = temp_dir("repo");
    let config = root.join("hosts.json");
    let body = include_str!("fixtures/forgejo-15.0.7/repository.json");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let response: &'static str = Box::leak(response.into_boxed_str());
    let (host, server) = serve(vec![response]);
    let output = run_with_context(
        &["repo", "view", "-R", "dalton/monolith", "--json"],
        &host,
        &config,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"repo\",\"name\":\"monolith\",\"full_name\":\"dalton/monolith\",\"description\":null,\"private\":true,\"archived\":false,\"default_branch\":\"main\",\"html_url\":\"https://forge.example/dalton/monolith\"}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with("GET /api/v1/repos/dalton/monolith HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn explicit_repo_selects_only_a_matching_remote_host() {
    let root = temp_dir("matching-remote");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![repository_response()]);
    let bin = fake_jj(
        &root,
        &[
            ("origin", "https://wrong.example/other/repo"),
            ("selected", &format!("{host}/dalton/monolith")),
        ],
    );

    let output = command()
        .args(["repo", "view", "-R", "dalton/monolith", "--json"])
        .current_dir(&root)
        .env("PATH", path_with_first(bin))
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with("GET /api/v1/repos/dalton/monolith HTTP/1.1\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn unrelated_remote_falls_back_to_the_saved_default() {
    let root = temp_dir("default-after-unrelated");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![repository_response()]);
    fs::write(
        &config,
        format!(
            "{{\"version\":1,\"default_host\":\"{host}\",\"hosts\":{{\"{host}\":{{\"token\":\"saved\"}}}}}}"
        ),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));
    let bin = fake_jj(&root, &[("origin", "https://wrong.example/other/repo")]);

    let output = command()
        .args(["repo", "view", "-R", "dalton/monolith", "--json"])
        .current_dir(&root)
        .env("PATH", path_with_first(bin))
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].contains("Authorization: token saved\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn explicit_host_ignores_a_stale_saved_default() {
    let root = temp_dir("explicit-host-stale-default");
    let config = root.join("hosts.json");
    fs::write(
        &config,
        "{\"version\":1,\"default_host\":\"https://stale.example\",\"hosts\":{}}",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));
    let (host, server) = serve(vec![json_response(
        "200 OK",
        "{\"version\":\"15.0.7\"}",
        "",
    )]);

    let output = run_with_context(&["api", "version", "--json"], &host, &config);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"version\":\"15.0.7\"}\n"
    );
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn unrelated_remote_cannot_supply_the_host_for_an_explicit_repo() {
    let root = temp_dir("unrelated-remote");
    let config = root.join("hosts.json");
    let bin = fake_jj(&root, &[("origin", "https://wrong.example/other/repo")]);

    let output = command()
        .args(["repo", "view", "-R", "dalton/monolith", "--json"])
        .current_dir(&root)
        .env("PATH", path_with_first(bin))
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("host context is required"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn login_verifies_then_saves_a_mode_0600_token_without_echoing_it() {
    let root = temp_dir("login");
    let config = root.join("config/hosts.json");
    let (host, server) = serve(vec![
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"login\":\"dalton\"}",
    ]);
    let mut child = command()
        .args(["auth", "login", "--with-token", "--json", "--host", &host])
        .env("FJX_CONFIG", &config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{error}"));
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("missing stdin"))
        .write_all(b"login-secret\n")
        .unwrap_or_else(|error| panic!("{error}"));
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"result\",\"action\":\"auth.login\",\"ok\":true,\"number\":null,\"html_url\":null}\n"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("login-secret"));
    let saved = fs::read_to_string(&config).unwrap_or_else(|error| panic!("{error}"));
    assert!(saved.contains("login-secret"));
    assert_eq!(
        fs::metadata(&config)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].contains("Authorization: token login-secret\r\n"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn redirect_and_unsafe_path_fail_with_empty_stdout() {
    let root = temp_dir("failures");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    ]);
    let redirect = run_with_context(&["api", "user"], &host, &config);
    assert_eq!(redirect.status.code(), Some(5));
    assert!(redirect.stdout.is_empty());
    assert_eq!(
        server
            .join()
            .unwrap_or_else(|_| panic!("server panicked"))
            .len(),
        1
    );

    let unsafe_path = command()
        .args(["api", "%2e%2e/user", "--host", "http://127.0.0.1:1"])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(unsafe_path.status.code(), Some(2));
    assert!(unsafe_path.stdout.is_empty());
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn paginate_joins_arrays_and_dry_run_sends_no_request() {
    let root = temp_dir("paging");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nLink: </api/v1/items?page=2>; rel=\"next\"\r\nx-total-count: 3\r\nContent-Length: 5\r\nConnection: close\r\n\r\n[1,2]",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nx-total-count: 3\r\nContent-Length: 3\r\nConnection: close\r\n\r\n[3]",
    ]);
    let output = run_with_context(&["api", "items", "--paginate", "--json"], &host, &config);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[1,2,3]\n");
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with("GET /api/v1/items?page=1&limit=50 HTTP/1.1"));
    assert!(requests[1].starts_with("GET /api/v1/items?page=2&limit=50 HTTP/1.1"));

    let input = root.join("input.json");
    fs::write(&input, "{\"title\":\"safe\"}").unwrap_or_else(|error| panic!("{error}"));
    let dry = command()
        .args(["api", "items", "--input"])
        .arg(&input)
        .args(["--dry-run", "--json", "--host", "http://127.0.0.1:1"])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&dry.stdout),
        "{\"kind\":\"request\",\"method\":\"POST\",\"url\":\"http://127.0.0.1:1/api/v1/items\",\"body\":{\"title\":\"safe\"}}\n"
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn paging_rejects_an_empty_page_that_claims_a_next_page() {
    let root = temp_dir("empty-next-page");
    let config = root.join("hosts.json");
    let next = "Link: </api/v1/next>; rel=\"next\"\r\nx-total-count: 1\r\n";

    let (host, server) = serve(vec![json_response("200 OK", "[]", next)]);
    let raw = run_with_context(&["api", "items", "--paginate", "--json"], &host, &config);
    assert_eq!(raw.status.code(), Some(7));
    assert!(raw.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let (host, server) = serve(vec![json_response("200 OK", "[]", next)]);
    let issues = run_with_context(
        &["issue", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );
    assert_eq!(issues.status.code(), Some(7));
    assert!(issues.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let empty_runs = "{\"total_count\":1,\"workflow_runs\":[]}";
    let (host, server) = serve(vec![json_response("200 OK", empty_runs, next)]);
    let runs = run_with_context(
        &["run", "list", "--all", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );
    assert_eq!(runs.status.code(), Some(7));
    assert!(runs.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn delete_requires_yes_before_context_or_network_work() {
    let output = command()
        .args([
            "api",
            "repos/o/r",
            "-X",
            "DELETE",
            "--host",
            "http://127.0.0.1:1",
        ])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
}

#[test]
fn typed_issue_commands_keep_routes_payloads_and_stable_json() {
    let root = temp_dir("typed-issue");
    let config = root.join("hosts.json");
    let issue = include_str!("fixtures/forgejo-15.0.7/issue.json");
    let list = format!("[{issue}]");
    let (host, server) = serve(vec![
        json_response("200 OK", &list, "x-total-count: 1\r\n"),
        json_response("201 Created", issue, ""),
    ]);

    let listed = run_with_context(
        &[
            "issue",
            "list",
            "--state",
            "all",
            "--limit",
            "10",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&listed.stdout).starts_with("[{\"kind\":\"issue\",\"number\":12")
    );

    let created = run_with_context(
        &[
            "issue",
            "create",
            "--title",
            "new",
            "--body",
            "body",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with(
        "GET /api/v1/repos/dalton/monolith/issues?state=all&type=issues&page=1&limit=10 HTTP/1.1"
    ));
    assert!(requests[1].starts_with("POST /api/v1/repos/dalton/monolith/issues HTTP/1.1"));
    assert!(requests[1].ends_with("{\"title\":\"new\",\"body\":\"body\"}"));

    let dry = command()
        .args([
            "issue",
            "close",
            "12",
            "--dry-run",
            "--json",
            "--host",
            "http://127.0.0.1:1",
            "-R",
            "dalton/monolith",
        ])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(dry.status.success());
    assert_eq!(
        String::from_utf8_lossy(&dry.stdout),
        "{\"kind\":\"request\",\"method\":\"PATCH\",\"url\":\"http://127.0.0.1:1/api/v1/repos/dalton/monolith/issues/12\",\"body\":{\"state\":\"closed\"}}\n"
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_bad_input_and_api_errors_leave_stdout_empty() {
    for arguments in [
        vec!["issue", "list", "--page", "0"],
        vec!["issue", "comment", "1", "--body", "a", "--body-file", "-"],
        vec!["pr", "review", "1", "--event", "request-changes"],
    ] {
        let output = command()
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }

    let root = temp_dir("typed-api-error");
    let config = root.join("hosts.json");
    let (host, server) = serve(vec![json_response("500 Internal Server Error", "{}", "")]);
    let output = run_with_context(
        &["issue", "view", "1", "-R", "dalton/monolith"],
        &host,
        &config,
    );
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one contract test compares all Forgejo 15 pull request payload spellings"
)]
fn pull_draft_review_merge_and_checks_use_forgejo_15_payloads() {
    let root = temp_dir("typed-pr");
    let config = root.join("hosts.json");
    let dry = command()
        .args([
            "pr",
            "create",
            "--head",
            "feature",
            "--title",
            "change",
            "--draft",
            "--dry-run",
            "--json",
            "--host",
            "http://127.0.0.1:1",
            "-R",
            "dalton/monolith",
        ])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(
        String::from_utf8_lossy(&dry.stdout)
            .ends_with("\"body\":{\"head\":\"feature\",\"title\":\"WIP: change\"}}\n")
    );

    let review = command()
        .args([
            "pr",
            "review",
            "8",
            "--event",
            "request-changes",
            "--body",
            "fix it",
            "--dry-run",
            "--json",
            "--host",
            "http://127.0.0.1:1",
            "-R",
            "dalton/monolith",
        ])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(review.status.success());
    let review_output = String::from_utf8_lossy(&review.stdout);
    assert!(review_output.contains("\"body\":\"fix it\""));
    assert!(review_output.contains("\"event\":\"REQUEST_CHANGES\""));

    let merge = command()
        .args([
            "pr",
            "merge",
            "8",
            "--style",
            "squash",
            "--title",
            "done",
            "--message",
            "merged",
            "--delete-branch",
            "--yes",
            "--dry-run",
            "--json",
            "--host",
            "http://127.0.0.1:1",
            "-R",
            "dalton/monolith",
        ])
        .env("FJX_TOKEN", "secret")
        .env("FJX_CONFIG", &config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(merge.status.success());
    let merge_output = String::from_utf8_lossy(&merge.stdout);
    for part in [
        "\"Do\":\"squash\"",
        "\"MergeTitleField\":\"done\"",
        "\"MergeMessageField\":\"merged\"",
        "\"delete_branch_after_merge\":true",
    ] {
        assert!(
            merge_output.contains(part),
            "missing {part}: {merge_output}"
        );
    }

    let refused = command()
        .args(["pr", "merge", "8"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(refused.status.code(), Some(6));
    assert!(refused.stdout.is_empty());

    let pull = include_str!("fixtures/forgejo-15.0.7/pull-request.json");
    let statuses = "{\"sha\":\"abc123\",\"state\":\"failure\",\"statuses\":[{\"context\":\"ci\",\"status\":\"error\",\"description\":null,\"target_url\":null}]}";
    let (host, server) = serve(vec![
        json_response("200 OK", pull, ""),
        json_response("200 OK", statuses, ""),
    ]);
    let checks = run_with_context(
        &["pr", "checks", "8", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );
    assert_eq!(checks.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&checks.stdout),
        "{\"kind\":\"checks\",\"sha\":\"abc123\",\"state\":\"failure\",\"statuses\":[{\"context\":\"ci\",\"state\":\"failure\",\"description\":null,\"target_url\":null}]}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[1].starts_with(
        "GET /api/v1/repos/dalton/monolith/commits/abc123/status?page=1&limit=50 HTTP/1.1"
    ));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn action_run_list_and_watch_emit_only_final_records() {
    let root = temp_dir("typed-runs");
    let config = root.join("hosts.json");
    let success = include_str!("fixtures/forgejo-15.0.7/action-run.json");
    let list = format!("{{\"total_count\":1,\"workflow_runs\":[{success}]}}");
    let (host, server) = serve(vec![json_response("200 OK", &list, "")]);
    let listed = run_with_context(
        &["run", "list", "--json", "-R", "dalton/monolith"],
        &host,
        &config,
    );
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("\"conclusion\":\"success\""));
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    for (status, code) in [("success", 0), ("failure", 1)] {
        let body = success.replace(
            "\"status\": \"success\"",
            &format!("\"status\": \"{status}\""),
        );
        let (host, server) = serve(vec![json_response("200 OK", &body, "")]);
        let watched = run_with_context(
            &["run", "watch", "42", "--json", "-R", "dalton/monolith"],
            &host,
            &config,
        );
        assert_eq!(watched.status.code().unwrap_or(0), code);
        assert_eq!(
            String::from_utf8_lossy(&watched.stdout)
                .matches("\"kind\":\"run\"")
                .count(),
            1
        );
        server.join().unwrap_or_else(|_| panic!("server panicked"));
    }
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn action_watch_timeout_and_poll_error_keep_stdout_empty() {
    let root = temp_dir("typed-watch-fail");
    let config = root.join("hosts.json");
    let success = include_str!("fixtures/forgejo-15.0.7/action-run.json");
    let running = success.replace("\"status\": \"success\"", "\"status\": \"running\"");
    let (host, server) = serve(vec![json_response("200 OK", &running, "")]);
    let timeout = run_with_context(
        &[
            "run",
            "watch",
            "42",
            "--poll",
            "1",
            "--wait",
            "1",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert_eq!(timeout.status.code(), Some(4));
    assert!(timeout.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));

    let (host, server) = serve(vec![
        json_response("200 OK", &running, ""),
        json_response("500 Internal Server Error", "{}", ""),
    ]);
    let failed = run_with_context(
        &[
            "run",
            "watch",
            "42",
            "--poll",
            "1",
            "--wait",
            "3",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert_eq!(failed.status.code(), Some(5));
    assert!(failed.stdout.is_empty());
    server.join().unwrap_or_else(|_| panic!("server panicked"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_all_refuses_a_partial_result_at_one_thousand_items() {
    let root = temp_dir("typed-cap");
    let config = root.join("hosts.json");
    let issue = include_str!("fixtures/forgejo-15.0.7/issue.json");
    let body = format!("[{}]", vec![issue; 50].join(","));
    let response = json_response(
        "200 OK",
        &body,
        "Link: </api/v1/next>; rel=\"next\"\r\nx-total-count: 1001\r\n",
    );
    let (host, server) = serve(vec![response; 20]);
    let output = run_with_context(
        &[
            "issue",
            "list",
            "--all",
            "--limit",
            "50",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    assert_eq!(
        server
            .join()
            .unwrap_or_else(|_| panic!("server panicked"))
            .len(),
        20
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one table-driven public contract test covers all related dry-run command families"
)]
fn workflow_family_dry_runs_use_typed_forgejo_15_payloads() {
    let root = temp_dir("workflow-family-dry-run");
    let config = root.join("hosts.json");
    let asset = root.join("asset.bin");
    fs::write(&asset, b"asset").unwrap_or_else(|error| panic!("{error}"));
    let host = "http://127.0.0.1:1";
    let cases: Vec<(Vec<&str>, &[&str])> = vec![
        (
            vec![
                "release",
                "create",
                "--tag",
                "v1",
                "--title",
                "one",
                "--target",
                "main",
                "--draft",
                "--prerelease",
            ],
            &[
                "/releases",
                "\"tag_name\":\"v1\"",
                "\"target_commitish\":\"main\"",
            ],
        ),
        (
            vec!["label", "create", "--name", "bug", "--color", "#d73a4a"],
            &["/labels", "\"color\":\"#d73a4a\""],
        ),
        (
            vec![
                "milestone",
                "create",
                "--title",
                "1.2",
                "--due",
                "2026-09-30T23:59:59Z",
            ],
            &["/milestones", "\"due_on\":\"2026-09-30T23:59:59Z\""],
        ),
        (
            vec![
                "milestone",
                "create",
                "--title",
                "leap",
                "--due",
                "2028-02-29T00:00:00.123+14:00",
            ],
            &[
                "/milestones",
                "\"due_on\":\"2028-02-29T00:00:00.123+14:00\"",
            ],
        ),
        (
            vec![
                "milestone",
                "create",
                "--title",
                "offset",
                "--due",
                "2026-12-31T23:59:59.000001-23:59",
            ],
            &[
                "/milestones",
                "\"due_on\":\"2026-12-31T23:59:59.000001-23:59\"",
            ],
        ),
        (
            vec!["branch", "delete", "feature/a", "--yes"],
            &["/branches/feature%2Fa", "\"body\":null"],
        ),
        (
            vec![
                "workflow",
                "dispatch",
                ".forgejo/workflows/release.yml",
                "--ref",
                "main",
                "--field",
                "channel=stable",
            ],
            &[
                "/actions/workflows/.forgejo%2Fworkflows%2Frelease.yml/dispatches",
                "\"inputs\":{\"channel\":\"stable\"}",
                "\"return_run_info\":true",
            ],
        ),
    ];
    for (mut arguments, expected) in cases {
        arguments.extend(["--dry-run", "--json", "-R", "dalton/monolith"]);
        let output = run_with_context(&arguments, host, &config);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        for part in expected {
            assert!(stdout.contains(part), "missing {part}: {stdout}");
        }
    }

    let asset_text = asset.to_string_lossy().into_owned();
    let uploaded = run_with_context(
        &[
            "release",
            "upload",
            "7",
            &asset_text,
            "--name",
            "asset one.bin",
            "--dry-run",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        host,
        &config,
    );
    assert!(uploaded.status.success());
    let stdout = String::from_utf8_lossy(&uploaded.stdout);
    assert!(stdout.contains("/releases/7/assets?name=asset%20one.bin"));
    assert!(stdout.contains("\"size\":5"));

    let refused = command()
        .args(["branch", "delete", "main"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(refused.status.code(), Some(6));
    assert!(refused.stdout.is_empty());
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_workflow_lists_keep_routes_paging_and_domain_json() {
    let root = temp_dir("workflow-family-lists");
    let config = root.join("hosts.json");
    let release = include_str!("fixtures/forgejo-15.0.7/release.json");
    let label = include_str!("fixtures/forgejo-15.0.7/label.json");
    let milestone = include_str!("fixtures/forgejo-15.0.7/milestone.json");
    let branch = include_str!("fixtures/forgejo-15.0.7/branch.json");
    let responses = [release, label, milestone, branch]
        .map(|value| json_response("200 OK", &format!("[{value}]"), "x-total-count: 1\r\n"));
    let (host, server) = serve(responses.into_iter().collect());
    let commands = [
        ["release", "list"],
        ["label", "list"],
        ["milestone", "list"],
        ["branch", "list"],
    ];
    for arguments in commands {
        let output = run_with_context(
            &[
                arguments[0],
                arguments[1],
                "--json",
                "-R",
                "dalton/monolith",
            ],
            &host,
            &config,
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("\"kind\":"));
    }
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    for (request, path) in
        requests
            .iter()
            .zip(["releases", "labels", "milestones?state=open", "branches"])
    {
        assert!(request.starts_with(&format!(
            "GET /api/v1/repos/dalton/monolith/{path}{}page=1&limit=30 HTTP/1.1",
            if path.contains('?') { "&" } else { "?" }
        )));
    }
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn release_upload_and_workflow_dispatch_emit_only_final_typed_results() {
    let root = temp_dir("workflow-family-writes");
    let config = root.join("hosts.json");
    let asset = root.join("artifact.bin");
    fs::write(&asset, b"binary-asset").unwrap_or_else(|error| panic!("{error}"));
    let attachment = "{\"id\":9,\"name\":\"artifact.bin\",\"size\":12,\"browser_download_url\":\"https://forgejo.example/assets/9\",\"created_at\":\"2026-08-29T12:00:00Z\"}";
    let dispatch = include_str!("fixtures/forgejo-15.0.7/workflow-dispatch.json");
    let (host, server) = serve(vec![
        json_response("201 Created", attachment, ""),
        json_response("201 Created", dispatch, ""),
    ]);
    let asset_text = asset.to_string_lossy().into_owned();
    let uploaded = run_with_context(
        &[
            "release",
            "upload",
            "7",
            &asset_text,
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert!(uploaded.status.success());
    assert!(String::from_utf8_lossy(&uploaded.stdout).starts_with("{\"kind\":\"release_asset\""));

    let dispatched = run_with_context(
        &[
            "workflow",
            "dispatch",
            "release.yml",
            "--ref",
            "main",
            "--field",
            "publish=yes",
            "--json",
            "-R",
            "dalton/monolith",
        ],
        &host,
        &config,
    );
    assert_eq!(
        String::from_utf8_lossy(&dispatched.stdout),
        "{\"kind\":\"workflow_dispatch\",\"id\":42,\"run_number\":9,\"jobs\":[\"check\",\"release\"]}\n"
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(requests[0].starts_with(
        "POST /api/v1/repos/dalton/monolith/releases/7/assets?name=artifact.bin HTTP/1.1"
    ));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("content-type: application/octet-stream\r\n")
    );
    assert!(requests[0].ends_with("binary-asset"));
    assert!(requests[1].starts_with(
        "POST /api/v1/repos/dalton/monolith/actions/workflows/release.yml/dispatches HTTP/1.1"
    ));
    assert!(
        requests[1].ends_with(
            "{\"ref\":\"main\",\"inputs\":{\"publish\":\"yes\"},\"return_run_info\":true}"
        )
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn typed_workflow_input_errors_fail_before_context_or_network() {
    for arguments in [
        vec!["release", "create", "--title", "missing tag"],
        vec!["label", "create", "--name", "bad", "--color", "xyz"],
        vec!["milestone", "create", "--title", "x", "--due", ""],
        vec![
            "workflow",
            "dispatch",
            "release.yml",
            "--ref",
            "main",
            "--field",
            "bad",
        ],
        vec![
            "workflow",
            "dispatch",
            "release.yml",
            "--ref",
            "main",
            "--field",
            "x=one",
            "--field",
            "x=two",
        ],
    ] {
        let output = command()
            .args(arguments)
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn invalid_milestone_due_dates_fail_before_context_or_network() {
    for due in [
        "next Friday",
        "2025-02-29T12:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T23:59:60Z",
        "2026-01-01T12:00:00+24:00",
        "2026-01-01T12:00:00+02:60",
        "2026-01-01T12:00:00.Z",
    ] {
        let output = command()
            .args(["milestone", "create", "--title", "x", "--due", due])
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(output.status.code(), Some(2), "{due}");
        assert!(output.stdout.is_empty(), "{due}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "fjx: --due must be an RFC3339 date-time\n",
            "{due}"
        );
    }
}
