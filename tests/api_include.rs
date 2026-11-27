//! `fjx api --include` writes each response's status line and headers to
//! stderr and keeps stdout as the body.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;

fn serve(responses: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("{error}"));
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
            // Accepted sockets can inherit nonblocking mode on Windows.
            stream
                .set_nonblocking(false)
                .unwrap_or_else(|error| panic!("{error}"));
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
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream
            .read(&mut buffer)
            .unwrap_or_else(|error| panic!("{error}"));
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn response(status: &str, headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn run(args: &[&str], host: &str) -> Output {
    let missing_config = std::env::temp_dir().join(format!(
        "fjx-api-include-missing-config-{}",
        std::process::id()
    ));
    Command::new(env!("CARGO_BIN_EXE_fjx"))
        .args(args)
        .args(["--host", host])
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN")
        .env("FJX_TOKEN", "test-secret")
        .env("FJX_CONFIG", missing_config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn include_writes_escaped_headers_to_stderr_and_keeps_the_body_on_stdout() {
    let (host, server) = serve(vec![response(
        "200 Fine\tReason",
        "Link: <https://x/api/v1/version?page=2>; rel=\"next\"\r\nX-Total-Count: 2\r\nSet-Cookie: session=secret-cookie\r\nSET-COOKIE: other=secret-cookie\r\nX-Odd: a\tb\\c\r\nVary: Accept\r\nVary: Origin\r\n",
        r#"{"version":"17.0.2"}"#,
    )]);
    let output = run(&["api", "version", "--include", "--json"], &host);
    server.join().unwrap_or_else(|_| panic!("server"));
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "{\"version\":\"17.0.2\"}\n");
    assert_eq!(
        text(&output.stderr),
        concat!(
            "HTTP/1.1 200 Fine\\tReason\n",
            "content-type: application/json\n",
            "link: <https://x/api/v1/version?page=2>; rel=\"next\"\n",
            "x-total-count: 2\n",
            "x-odd: a\\tb\\\\c\n",
            "vary: Accept\n",
            "vary: Origin\n",
            "content-length: 20\n",
            "connection: close\n",
            "\n",
        )
    );
}

#[test]
fn without_include_stderr_stays_empty() {
    let (host, server) = serve(vec![response("200 OK", "X-Total-Count: 2\r\n", "{}")]);
    let output = run(&["api", "version", "--json"], &host);
    server.join().unwrap_or_else(|_| panic!("server"));
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
}

#[test]
fn include_with_paginate_writes_one_block_per_page() {
    let (host, server) = serve(vec![
        response(
            "200 OK",
            "Link: <https://x/api/v1/items?page=2>; rel=\"next\"\r\nX-Total-Count: 2\r\n",
            "[1]",
        ),
        response("200 OK", "X-Total-Count: 2\r\n", "[2]"),
    ]);
    let output = run(
        &["api", "items", "--paginate", "--include", "--json"],
        &host,
    );
    let requests = server.join().unwrap_or_else(|_| panic!("server"));
    assert_eq!(requests.len(), 2);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "[1,2]\n");
    let stderr = text(&output.stderr);
    let blocks: Vec<&str> = stderr.split_terminator("\n\n").collect();
    assert_eq!(blocks.len(), 2, "{stderr}");
    assert!(blocks[0].starts_with("HTTP/1.1 200 OK\n") && blocks[0].contains("rel=\"next\""));
    assert!(blocks[1].starts_with("HTTP/1.1 200 OK\n") && !blocks[1].contains("link:"));
}

#[test]
fn include_shows_headers_of_an_http_error() {
    let (host, server) = serve(vec![response(
        "429 Too Many Requests",
        "Retry-After: 30\r\n",
        r#"{"message":"slow down"}"#,
    )]);
    let output = run(&["api", "version", "--include"], &host);
    server.join().unwrap_or_else(|_| panic!("server"));
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("HTTP/1.1 429 Too Many Requests\n"),
        "{stderr}"
    );
    assert!(stderr.contains("\nretry-after: 30\n"), "{stderr}");
    assert!(stderr.contains("\n\nfjx: "), "{stderr}");
    assert!(stderr.ends_with("fjx: Forgejo returned HTTP 429: slow down\n"));
}

#[test]
fn include_is_a_usage_error_without_a_request() {
    for args in [
        &["api", "user", "-X", "POST", "--dry-run", "--include"][..],
        &["issue", "list", "-R", "o/r", "--include"],
        &["auth", "list", "--include"],
    ] {
        let output = run(args, "http://127.0.0.1:1");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
}
