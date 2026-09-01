use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
    command
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .env(
            "FJX_CONFIG",
            std::env::temp_dir().join(format!("fjx-repo-parts-{}.json", std::process::id())),
        );
    command
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

struct Server {
    host: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| panic!("{error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_request(&mut stream);
                        thread_requests
                            .lock()
                            .unwrap_or_else(|error| panic!("{error}"))
                            .push(request);
                        let body = include_str!("fixtures/forgejo-15.0.7/repository.json");
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream
                            .write_all(response.as_bytes())
                            .unwrap_or_else(|error| panic!("{error}"));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        Self {
            host: format!("http://{address}"),
            requests,
            stop,
            thread,
        }
    }

    fn finish(self) -> Vec<String> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .join()
            .unwrap_or_else(|_| panic!("server panicked"));
        Arc::try_unwrap(self.requests)
            .unwrap_or_else(|_| panic!("server requests still shared"))
            .into_inner()
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

fn run_with_flag(host: &str, repo: &str) -> Output {
    command()
        .args(["repo", "view", "--json", "--host", host, "-R", repo])
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn run_with_env(host: &str, repo: &str) -> Output {
    command()
        .args(["repo", "view", "--json", "--host", host])
        .env("FJX_REPO", repo)
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn assert_context_errors(outputs: &[(&str, Output)]) {
    for (repo, output) in outputs {
        assert_eq!(
            output.status.code(),
            Some(3),
            "repo {repo:?} produced stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "repo {repo:?} wrote stdout");
    }
}

#[test]
fn repo_flag_rejects_dot_parts_before_http() {
    let server = Server::start();
    let outputs: Vec<_> = ["./repo", "../repo", "owner/.", "owner/.."]
        .into_iter()
        .map(|repo| (repo, run_with_flag(&server.host, repo)))
        .collect();
    let requests = server.finish();

    assert_context_errors(&outputs);
    assert!(requests.is_empty(), "invalid -R values sent HTTP requests");
}

#[test]
fn repo_env_rejects_dot_parts_before_http() {
    let server = Server::start();
    let outputs: Vec<_> = ["./repo", "../repo", "owner/.", "owner/.."]
        .into_iter()
        .map(|repo| (repo, run_with_env(&server.host, repo)))
        .collect();
    let requests = server.finish();

    assert_context_errors(&outputs);
    assert!(
        requests.is_empty(),
        "invalid FJX_REPO values sent HTTP requests"
    );
}

#[test]
fn dotted_repo_parts_reach_http() {
    let server = Server::start();
    let flag = run_with_flag(&server.host, "dalton.dev/monolith.rs");
    let env = run_with_env(&server.host, ".dalton/monolith.");
    let requests = server.finish();

    for output in [&flag, &env] {
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /api/v1/repos/dalton.dev/monolith.rs HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /api/v1/repos/.dalton/monolith. HTTP/1.1\r\n"));
}
