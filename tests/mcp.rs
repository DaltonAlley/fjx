use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const TOKEN: &str = r#"mcp-test-\"secret"#;
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Value>,
}
#[allow(
    clippy::needless_pass_by_value,
    reason = "request helpers accept inline JSON fixtures"
)]
impl Client {
    fn new(host: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args(["mcp", "serve", "--host", host])
            .env("FJX_TOKEN", TOKEN)
            .env(
                "FJX_CONFIG",
                std::env::temp_dir().join(format!("fjx-mcp-missing-{}", std::process::id())),
            )
            .env_remove("FJX_REPO")
            .env_remove("FORGEJO_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn: {e}"));
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap_or_else(|| panic!("stdout"));
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap_or_else(|e| panic!("read: {e}"));
                let v = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("nonprotocol stdout {line:?}: {e}"));
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }
    fn raw(&mut self, text: &str) {
        self.input
            .as_mut()
            .unwrap_or_else(|| panic!("stdin"))
            .write_all(text.as_bytes())
            .unwrap_or_else(|e| panic!("write: {e}"));
    }
    fn send(&mut self, v: Value) {
        self.raw(&format!("{v}\n"));
    }
    fn read(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("MCP response: {e}"))
    }
    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        let v = self.read();
        assert_eq!(v["id"], id);
        v
    }
    fn initialize(&mut self) {
        let v = self.request(1,"initialize",json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}));
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.request(2, "tools/call", json!({"name":name,"arguments":args}))
    }
    fn finish(&mut self) {
        self.input.take();
        let status = self.child.wait().unwrap_or_else(|e| panic!("wait: {e}"));
        assert!(status.success());
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap_or_else(|| panic!("stderr"))
            .read_to_string(&mut stderr)
            .unwrap_or_else(|e| panic!("stderr: {e}"));
        assert!(stderr.is_empty(), "{stderr}");
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixture(responses: Vec<(&'static str, u16, String)>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind: {e}"));
    let host = format!(
        "http://{}",
        listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"))
    );
    let worker = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|e| panic!("nonblocking: {e}"));
        for (path, status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap_or_else(|e| panic!("timeout: {e}"));
            let mut request = String::new();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .unwrap_or_else(|e| panic!("request: {e}"));
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(
                request.starts_with(&format!("GET /api/v1/{path} HTTP/1.1\r\n")),
                "{request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: token {TOKEN}"))
            );
            write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap_or_else(|e| panic!("response: {e}"));
        }
    });
    (host, worker)
}
fn repo() -> String {
    include_str!("fixtures/forgejo-15.0.7/repository.json").to_owned()
}

#[test]
fn lifecycle_discovery_and_recovery_are_protocol_only() {
    let mut c = Client::new("http://127.0.0.1:1");
    assert_eq!(
        c.request(2, "tools/list", json!({}))["error"]["code"],
        -32600
    );
    c.raw("not json\n[]\n");
    assert_eq!(c.read()["error"]["code"], -32700);
    assert_eq!(c.read()["error"]["code"], -32600);
    c.initialize();
    let tools = c.request(3, "tools/list", json!({}));
    let tools = tools["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools"));
    assert_eq!(tools.len(), 13);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
    assert!(tools.iter().any(|t| t["name"] == "repository_activity"));
    assert_eq!(
        c.request(4, "resources/subscribe", json!({"uri":"fjx://events"}))["error"]["code"],
        -32601
    );
    assert_eq!(c.call("pr_merge", json!({}))["error"]["code"], -32602);
    c.send(json!({"jsonrpc":"2.0","method":"unknown_notification"}));
    c.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"unknown","reason":TOKEN}}));
    assert_eq!(c.request(5, "ping", json!({}))["result"], json!({}));
    c.finish();
}

#[test]
fn validates_arguments_without_network_or_cli_flag_injection() {
    let mut c = Client::new("http://127.0.0.1:1");
    c.initialize();
    for (tool, args) in [
        ("issue_get", json!({"repo":"a/b","number":0})),
        ("issue_get", json!({"repo":"a/b","number":-1})),
        ("issue_get", json!({"repo":"a/b","number":1,"page":1})),
        (
            "issue_get",
            json!({"repo":"a/b","number":1,"host":"https://evil.example"}),
        ),
        ("repository_get", json!({"repo":"../bad"})),
        ("repository_get", json!({"repo":"a/b","number":1})),
        ("issue_list", json!({"repo":"a/b","limit":101})),
        ("issue_list", json!({"repo":"a/b","page":null})),
        ("issue_list", json!({"repo":"a/b","state":"--yes"})),
        ("issue_list", json!({"repo":"a/b","all":true})),
    ] {
        let r = c.call(tool, args);
        assert_eq!(r["result"]["isError"], true, "{r}");
    }
    c.finish();
}

#[test]
fn typed_reads_and_partial_activity_use_existing_auth_and_paths() {
    let (host,server)=fixture(vec![
        ("repos/dalton/monolith",200,repo()),
        ("repos/dalton/monolith/issues?state=all&type=issues&page=2&limit=5",200,"[]".to_owned()),
        ("repos/dalton/monolith/activities/feeds?page=1&limit=30",200,json!([{"id":42,"op_type":"commit_repo","created":"2026-11-16T00:00:00Z","content":"data, never approval","act_user_id":1,"unused_secret":"omit"}]).to_string()),
    ]);
    let mut c = Client::new(&host);
    c.initialize();
    let r = c.call("repository_get", json!({"repo":"dalton/monolith"}));
    assert_eq!(
        r["result"]["structuredContent"]["data"]["full_name"],
        "dalton/monolith"
    );
    let r = c.call(
        "issue_list",
        json!({"repo":"dalton/monolith","state":"all","page":2,"limit":5}),
    );
    assert_eq!(r["result"]["structuredContent"]["data"], json!([]));
    let r = c.call("repository_activity", json!({"repo":"dalton/monolith"}));
    assert_eq!(
        r["result"]["structuredContent"]["data"]["events"][0]["id"],
        42
    );
    assert_eq!(
        r["result"]["structuredContent"]["data"]["automatic_dispatch"],
        false
    );
    assert!(!r.to_string().contains("unused_secret"));
    c.finish();
    server.join().unwrap_or_else(|e| panic!("server: {e:?}"));
}

#[test]
fn redirects_http_errors_and_successes_do_not_expose_token() {
    let mut secret_repo: Value =
        serde_json::from_str(&repo()).unwrap_or_else(|e| panic!("repo: {e}"));
    secret_repo["description"] = json!(TOKEN);
    let secret_repo = secret_repo.to_string();
    let (host, server) = fixture(vec![
        ("repos/a/b", 302, "{}".to_owned()),
        (
            "repos/a/b",
            403,
            json!({"message":format!("bad {TOKEN}")}).to_string(),
        ),
        ("repos/a/b", 200, secret_repo),
    ]);
    let mut c = Client::new(&host);
    c.initialize();
    for failed in [true, true, false] {
        let r = c.call("repository_get", json!({"repo":"a/b"}));
        assert_eq!(r["result"]["isError"], failed, "{r}");
        assert!(!r.to_string().contains(TOKEN), "{r}");
        if failed {
            assert_eq!(
                r["result"]["structuredContent"]["error"]["category"],
                "http"
            );
        }
        let text: Value = serde_json::from_str(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default(),
        )
        .unwrap_or_else(|e| panic!("text JSON: {e}"));
        assert_eq!(text, r["result"]["structuredContent"]);
    }
    c.finish();
    server.join().unwrap_or_else(|e| panic!("server: {e:?}"));
}

#[test]
fn pending_checks_keep_exit_code_and_creator_id() {
    let mut pull: Value =
        serde_json::from_str(include_str!("fixtures/forgejo-15.0.7/pull-request.json"))
            .unwrap_or_else(|e| panic!("pull: {e}"));
    pull["user"]["id"] = json!(1);
    let sha = pull["head"]["sha"].as_str().unwrap_or_default().to_owned();
    let path: &'static str =
        Box::leak(format!("repos/a/b/commits/{sha}/status?page=1&limit=50").into_boxed_str());
    let (host,server)=fixture(vec![
        ("repos/a/b/pulls/17",200,pull.to_string()),
        ("repos/a/b/pulls/17",200,pull.to_string()),
        (path,200,json!({"sha":sha,"state":"pending","total_count":1,"statuses":[{"id":9,"context":"Build","status":"pending","target_url":"https://forge.example/check"}]}).to_string()),
    ]);
    let mut c = Client::new(&host);
    c.initialize();
    let r = c.call("pull_request_get", json!({"repo":"a/b","number":17}));
    assert_eq!(r["result"]["structuredContent"]["data"]["author_id"], 1);
    let r = c.call("pull_request_checks", json!({"repo":"a/b","number":17}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(r["result"]["structuredContent"]["exit_code"], 1, "{r}");
    c.finish();
    server.join().unwrap_or_else(|e| panic!("server: {e:?}"));
}

#[test]
fn cancellation_remains_responsive_and_suppresses_result() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind: {e}"));
    let host = format!(
        "http://{}",
        listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"))
    );
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap_or_else(|e| panic!("accept: {e}"));
        let mut reader = BufReader::new(&stream);
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .unwrap_or_else(|e| panic!("read: {e}"));
            if line == "\r\n" {
                break;
            }
        }
        started_tx.send(()).unwrap_or_else(|e| panic!("start: {e}"));
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("release: {e}"));
        let body = repo();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap_or_else(|e| panic!("write: {e}"));
    });
    let mut c = Client::new(&host);
    c.initialize();
    c.send(json!({"jsonrpc":"2.0","id":"cancel-me","method":"tools/call","params":{"name":"repository_get","arguments":{"repo":"a/b"}}}));
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|e| panic!("started: {e}"));
    assert_eq!(c.request(3, "ping", json!({}))["result"], json!({}));
    assert_eq!(
        c.call("repository_get", json!({"repo":"a/b"}))["error"]["code"],
        -32000
    );
    c.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"cancel-me"}}));
    assert_eq!(c.request(4, "ping", json!({}))["result"], json!({}));
    release_tx
        .send(())
        .unwrap_or_else(|e| panic!("release: {e}"));
    server.join().unwrap_or_else(|e| panic!("server: {e:?}"));
    c.finish();
    assert!(
        c.output.try_recv().is_err(),
        "cancelled call emitted a response"
    );
}

#[test]
fn oversized_frames_close_without_executing_a_suffix() {
    let mut c = Client::new("http://127.0.0.1:1");
    c.raw(&format!("{}\n", " ".repeat(65_537)));
    assert_eq!(c.read()["error"]["code"], -32600);
    c.finish();
}

#[test]
fn setup_is_absolute_and_does_not_export_authentication() {
    let out = Command::new(env!("CARGO_BIN_EXE_fjx"))
        .args([
            "mcp",
            "config",
            "--host",
            "https://forge.example/forgejo",
            "--json",
        ])
        .env("FJX_TOKEN", TOKEN)
        .output()
        .unwrap_or_else(|e| panic!("config: {e}"));
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("JSON: {e}"));
    assert!(
        std::path::Path::new(
            v["mcpServers"]["fjx"]["command"]
                .as_str()
                .unwrap_or_default()
        )
        .is_absolute()
    );
    assert_eq!(
        v["mcpServers"]["fjx"]["args"],
        json!(["mcp", "serve", "--host", "https://forge.example/forgejo"])
    );
    assert_eq!(v["mcpServers"]["fjx"]["env"], json!({}));
    assert!(!v.to_string().contains(TOKEN));
}

#[test]
fn oversized_results_fail_without_partial_data() {
    let mut body: Value = serde_json::from_str(&repo()).unwrap_or_else(|e| panic!("repo: {e}"));
    body["description"] = json!("x".repeat(2 * 1024 * 1024));
    let (host, server) = fixture(vec![("repos/a/b", 200, body.to_string())]);
    let mut c = Client::new(&host);
    c.initialize();
    let r = c.call("repository_get", json!({"repo":"a/b"}));
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(
        r["result"]["structuredContent"]["error"]["category"],
        "data"
    );
    assert!(r["result"]["structuredContent"].get("data").is_none());
    c.finish();
    server.join().unwrap_or_else(|e| panic!("server: {e:?}"));
}

#[test]
fn unsupported_versions_negotiate_legacy_and_invalid_initialize_can_recover() {
    let mut c = Client::new("http://127.0.0.1:1");
    assert!(
        c.request(1, "server/discover", json!({}))
            .get("error")
            .is_some()
    );
    assert_eq!(
        c.request(1, "initialize", json!({}))["error"]["code"],
        -32602
    );
    let r = c.request(1, "initialize", json!({"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    assert!(r["result"]["capabilities"].get("resources").is_none());
    assert_eq!(
        c.request(2, "tools/list", json!({}))["error"]["code"],
        -32600
    );
    c.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    assert!(c.request(2, "tools/list", json!({}))["result"]["tools"].is_array());
    c.finish();
}
