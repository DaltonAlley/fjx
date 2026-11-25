//! A remote whose SSH host is an alias reaches the saved login that records
//! the alias, and no other host.
#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const OTHER: &str = "https://other.example";

struct Fixture {
    host: String,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<Vec<String>>>,
}

impl Fixture {
    /// A forge on loopback, saved logins for it and for another host, and a
    /// checkout whose only remote is `remote`.
    fn new(remote: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("{error}"));
        let host = format!(
            "http://{}",
            listener.local_addr().unwrap_or_else(|e| panic!("{e}"))
        );
        let root = std::env::temp_dir().join(format!(
            "fjx-ssh-alias-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap_or_else(|error| panic!("{error}"));
        // Without jj on PATH, fjx asks this git for the checkout's remotes.
        let git = bin.join("git");
        fs::write(
            &git,
            format!("#!/bin/sh\nprintf '%s\\n' 'origin {remote} (fetch)'\n"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&git, fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("{error}"));
        let fixture_host = host.clone();
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
                        let mut bytes = Vec::new();
                        let mut buffer = [0; 4096];
                        while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                            let count = stream
                                .read(&mut buffer)
                                .unwrap_or_else(|error| panic!("{error}"));
                            if count == 0 {
                                break;
                            }
                            bytes.extend_from_slice(&buffer[..count]);
                        }
                        let request = String::from_utf8_lossy(&bytes).into_owned();
                        let body = if request.starts_with("GET /api/v1/user ") {
                            r#"{"login":"user"}"#
                        } else {
                            include_str!("fixtures/forgejo-15.0.7/repository.json")
                        };
                        requests.push(request);
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
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
        let fixture = Self {
            host: fixture_host,
            root,
            stop,
            worker: Some(worker),
        };
        fixture.write_config(&json!({
            "version": 1,
            "default_host": OTHER,
            "hosts": {
                &fixture.host: {"user": "user", "token": "saved-secret"},
                OTHER: {"user": "user", "token": "other-secret"},
            },
        }));
        fixture
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("hosts.json")
    }

    fn write_config(&self, value: &Value) {
        let path = self.config_path();
        fs::write(&path, value.to_string()).unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|error| panic!("{error}"));
    }

    fn config(&self) -> Value {
        let bytes = fs::read(self.config_path()).unwrap_or_else(|error| panic!("{error}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Run in the checkout with no host, repository, or token supplied.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args(args)
            .current_dir(&self.root)
            .env("PATH", self.root.join("bin"))
            .env("HOME", &self.root)
            .env("FJX_CONFIG", self.config_path())
            .env_remove("FJX_HOST")
            .env_remove("FJX_REPO")
            .env_remove("FJX_TOKEN")
            .env_remove("FORGEJO_TOKEN")
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        self.worker
            .take()
            .unwrap_or_else(|| panic!("missing server"))
            .join()
            .unwrap_or_else(|error| panic!("{error:?}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_dir_all(&self.root);
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
    for secret in ["saved-secret", "other-secret"] {
        assert!(!stderr.contains(secret), "token disclosed");
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|error| panic!("{error}"))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn an_aliased_ssh_remote_reaches_the_saved_login_without_a_host_flag() {
    let fixture = Fixture::new("ssh://git@forge-ssh:2222/dalton/monolith.git");
    let host = fixture.host.clone();

    // Unaliased, the SSH name is only a guess and has no token to send.
    assert_failure(
        &fixture.run(&["repo", "view", "--json"]),
        3,
        "no token is available for https://forge-ssh; if forge-ssh is an SSH alias for a host you are logged in to, run fjx auth add-ssh-alias forge-ssh --host URL",
    );

    assert_eq!(
        decode(&fixture.run(&[
            "auth",
            "add-ssh-alias",
            "Forge-SSH",
            "--host",
            &host,
            "--json"
        ])),
        json!({"kind":"result","action":"auth.add-ssh-alias","ok":true,"host":host,"ssh_alias":"forge-ssh"})
    );
    let saved = fixture.config();
    assert_eq!(saved["hosts"][&host]["ssh_aliases"], json!(["forge-ssh"]));
    assert_eq!(saved["hosts"][&host]["token"], "saved-secret");
    assert_eq!(
        saved["hosts"][OTHER],
        json!({"user":"user","token":"other-secret"})
    );
    assert_eq!(saved["default_host"], OTHER);
    assert_eq!(mode(&fixture.config_path()), 0o600);

    // The remote now selects that login and the remote's own repository path.
    let view = decode(&fixture.run(&["repo", "view", "--json"]));
    assert_eq!(view["full_name"], "dalton/monolith");
    assert_eq!(
        decode(&fixture.run(&["auth", "status", "--json"])),
        json!({"kind":"auth","host":host,"user":"user","source":"config","ssh_aliases":["forge-ssh"]})
    );

    let removed = fixture.run(&["auth", "remove-ssh-alias", "forge-ssh", "--host", &host]);
    assert_eq!(
        String::from_utf8_lossy(&removed.stdout),
        format!("auth.remove-ssh-alias\tforge-ssh\t{host}\tok\n")
    );
    assert!(
        fixture.config()["hosts"][&host]
            .get("ssh_aliases")
            .is_none()
    );
    assert_failure(
        &fixture.run(&["repo", "view"]),
        3,
        "no token is available for https://forge-ssh",
    );

    let requests = fixture.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /api/v1/repos/dalton/monolith HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /api/v1/user HTTP/1.1\r\n"));
    for request in requests {
        assert!(request.contains("Authorization: token saved-secret\r\n"));
        assert!(!request.contains("other-secret"));
    }
}

#[test]
fn an_scp_style_remote_uses_the_same_alias() {
    let fixture = Fixture::new("git@forge-ssh:dalton/monolith.git");
    let host = fixture.host.clone();
    decode(&fixture.run(&[
        "auth",
        "add-ssh-alias",
        "forge-ssh",
        "--host",
        &host,
        "--json",
    ]));
    assert_eq!(
        decode(&fixture.run(&["repo", "view", "--json"]))["full_name"],
        "dalton/monolith"
    );
    assert_eq!(fixture.finish().len(), 1);
}

#[test]
fn alias_commands_change_nothing_unless_the_host_is_a_saved_login() {
    let fixture = Fixture::new("ssh://git@forge-ssh:2222/dalton/monolith.git");
    let host = fixture.host.clone();
    let mut with_alias = fixture.config();
    with_alias["hosts"][OTHER]["ssh_aliases"] = json!(["elsewhere"]);
    fixture.write_config(&with_alias);
    let before = fs::read(fixture.config_path()).unwrap_or_else(|error| panic!("{error}"));

    for (args, code, message) in [
        // Neither the remote nor the default host may choose the login.
        (
            &["auth", "add-ssh-alias", "forge-ssh"][..],
            2,
            "require --host URL or FJX_HOST",
        ),
        (
            &["auth", "remove-ssh-alias", "elsewhere"],
            2,
            "require --host URL or FJX_HOST",
        ),
        (
            &[
                "auth",
                "add-ssh-alias",
                "forge-ssh",
                "--host",
                "https://new.example",
            ],
            3,
            "no saved login for https://new.example",
        ),
        (
            &["auth", "add-ssh-alias", "elsewhere", "--host", &host],
            2,
            "SSH alias elsewhere already names https://other.example",
        ),
        (
            &["auth", "add-ssh-alias", "other.example", "--host", &host],
            2,
            "other.example is the DNS name of saved host https://other.example",
        ),
        (
            &["auth", "add-ssh-alias", "git@forge-ssh", "--host", &host],
            2,
            "SSH alias must be",
        ),
        (
            &["auth", "add-ssh-alias", "", "--host", &host],
            2,
            "SSH alias",
        ),
        (
            &["auth", "add-ssh-alias", "--host", &host],
            2,
            "fjx auth add-ssh-alias --help",
        ),
        (
            &["auth", "remove-ssh-alias", "forge-ssh", "--host", &host],
            2,
            "no SSH alias \"forge-ssh\" is recorded",
        ),
        (
            &["auth", "remove-ssh-alias", "elsewhere", "--host", &host],
            2,
            "no SSH alias",
        ),
        (
            &[
                "auth",
                "add-ssh-alias",
                "forge-ssh",
                "--host",
                &host,
                "--dry-run",
            ],
            2,
            "--dry-run is not valid for auth commands",
        ),
        (
            &[
                "auth",
                "add-ssh-alias",
                "forge-ssh",
                "--host",
                &host,
                "--yes",
            ],
            2,
            "--yes",
        ),
    ] {
        assert_failure(&fixture.run(args), code, message);
        assert_eq!(
            fs::read(fixture.config_path()).unwrap_or_else(|error| panic!("{error}")),
            before,
            "{args:?} changed the token file"
        );
    }
    assert!(fixture.finish().is_empty());
}

#[test]
fn a_token_file_whose_alias_names_two_hosts_is_refused() {
    let fixture = Fixture::new("ssh://git@forge-ssh:2222/dalton/monolith.git");
    let mut shared = fixture.config();
    for host in [fixture.host.as_str(), OTHER] {
        shared["hosts"][host]["ssh_aliases"] = json!(["forge-ssh"]);
    }
    fixture.write_config(&shared);
    assert_failure(
        &fixture.run(&["repo", "view"]),
        3,
        "invalid SSH alias in token file",
    );
    assert!(fixture.finish().is_empty());
}
