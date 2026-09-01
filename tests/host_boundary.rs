use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;

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

fn run_with_host(host: &str) -> Output {
    command()
        .args(["api", "version"])
        .env("FJX_HOST", host)
        .env("FJX_TOKEN", "test-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn assert_context_error(host: &str) {
    let output = run_with_host(host);
    assert_eq!(
        output.status.code(),
        Some(3),
        "host {host:?} produced stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "host {host:?} wrote stdout");
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
fn encoded_dot_segments_fail_before_http() {
    for path in [
        "prefix/../base",
        "prefix/./base",
        "prefix/%2e%2e/base",
        "prefix/%2E%2E/base",
        "prefix/%2E%2e/base",
        "prefix/.%2e/base",
        "prefix/%2e./base",
        "prefix/%2e/base",
        "prefix%2f%2e%2e/base",
        "prefix%2F.%2E/base",
        "prefix/%2e%2e%2fbase",
        "prefix%5c%2e%2e/base",
        "prefix%5C.%2e/base",
        "prefix/%2E%2e%5cbase",
    ] {
        assert_context_error(&format!("http://127.0.0.1:1/{path}"));
    }
}

#[test]
fn invalid_ports_are_context_errors() {
    for host in [
        "http://127.0.0.1:65536",
        "http://127.0.0.1:999999999999999999999",
        "http://127.0.0.1:+1",
        "http://[::1]:65536",
        "http://[::1]:not-a-port",
    ] {
        assert_context_error(host);
    }
}

#[test]
fn valid_loopback_prefix_reaches_the_named_base() {
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

    let output = run_with_host(&format!("http://{address}/forgejo/"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\n  \"version\": \"15\"\n}\n"
    );
    let request = server.join().unwrap_or_else(|_| panic!("server panicked"));
    assert!(request.starts_with("GET /forgejo/api/v1/version HTTP/1.1\r\n"));
}
