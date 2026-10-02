//! Opt-in, actual-binary A/B measurements. No latency assertions.
// Fixture failures deliberately panic rather than becoming benchmark measurements.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Traffic {
    requests: usize,
    response_body_bytes: usize,
    mutations: Vec<Value>,
}
struct Fixture {
    host: String,
    stop: Arc<AtomicBool>,
    traffic: Arc<Mutex<Traffic>>,
    worker: Option<thread::JoinHandle<()>>,
}
fn issues() -> Vec<Value> {
    (1..=120).map(|n| json!({"number":n,"title":if n % 20 == 0 {format!("needle issue {n}")} else {format!("Representative issue {n}")},"body":"Representative issue description with reproduction steps and expected behavior.\n".repeat(54),"html_url":format!("https://example.invalid/owner/repo/issues/{n}"),"user":{"login":"author"},"state":"open","labels":[{"id":7,"name":"bug","color":"ff0000","description":"Defect"}],"assignees":[],"created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:00:00Z"})).collect()
}
impl Fixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let traffic = Arc::new(Mutex::new(Traffic::default()));
        let stopped = stop.clone();
        let captured = traffic.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_mins(1);
            while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_micros(100));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 8192];
                let (end, length) = loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert_ne!(count, 0, "incomplete request");
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break (end, length);
                        }
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap();
                let target = first.next().unwrap();
                let labels = json!([{"id":7,"name":"bug","color":"ff0000","description":"Defect"}]);
                let (value, total) = if method != "GET" {
                    let body: Value =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    captured
                        .lock()
                        .unwrap()
                        .mutations
                        .push(json!({"method":method,"path":target,"body":body}));
                    if target.ends_with("/labels") {
                        (labels, None)
                    } else {
                        let mut issue = issues()[41].clone();
                        issue["title"] = json!("Revised");
                        (issue, None)
                    }
                } else if target.split('?').next().unwrap().ends_with("/labels") {
                    (labels, Some(1))
                } else {
                    assert!(
                        target.starts_with("/api/v1/repos/owner/repo/issues?"),
                        "unexpected {target}"
                    );
                    let query = target.split_once('?').unwrap().1;
                    let parameter = |name: &str| {
                        query.split('&').find_map(|p| {
                            p.split_once('=')
                                .filter(|(k, _)| *k == name)
                                .map(|(_, v)| v)
                        })
                    };
                    let page = parameter("page").unwrap_or("1").parse::<usize>().unwrap();
                    let limit = parameter("limit").unwrap_or("30").parse::<usize>().unwrap();
                    let values: Vec<_> = issues()
                        .into_iter()
                        .filter(|v| {
                            parameter("q").is_none()
                                || v["title"].as_str().unwrap().contains("needle")
                        })
                        .collect();
                    let total = values.len();
                    (
                        json!(
                            values
                                .into_iter()
                                .skip((page - 1) * limit)
                                .take(limit)
                                .collect::<Vec<_>>()
                        ),
                        Some(total),
                    )
                };
                let body = serde_json::to_vec(&value).unwrap();
                {
                    let mut t = captured.lock().unwrap();
                    t.requests += 1;
                    t.response_body_bytes += body.len();
                }
                let total = total
                    .map(|n| format!("X-Total-Count: {n}\r\n"))
                    .unwrap_or_default();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{total}Content-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            host,
            stop,
            traffic,
            worker: Some(worker),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn invoke(binary: &Path, args: &[String], host: &str, root: &Path) -> Vec<u8> {
    // Files rather than pipes avoid child stdout blocking while enforcing a deadline.
    let stdout = fs::File::create(root.join("stdout")).unwrap();
    let stderr = fs::File::create(root.join("stderr")).unwrap();
    let mut child = Command::new(binary)
        .args(args)
        .args(["--host", host, "-R", "owner/repo"])
        .env("FJX_TOKEN", "benchmark-fixture-token")
        .env("FJX_CONFIG", root.join("absent-config"))
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FORGEJO_TOKEN")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{:?}: {}",
                args,
                fs::read_to_string(root.join("stderr")).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("CLI deadline exceeded")
        }
        thread::sleep(Duration::from_micros(100));
    }
    fs::read(root.join("stdout")).unwrap()
}
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}
fn sample(binary: &Path, scenario: &str, new: bool, root: &Path) -> Value {
    let fixture = Fixture::new();
    let mut commands = match scenario {
        "help" => vec![strings(&["issue", "list", "--help"])],
        "projection" | "search" => {
            let mut args = strings(&["issue", "list", "--all", "--limit", "30", "--json"]);
            if new {
                args.extend(strings(&["--fields", "number,title,state"]));
                if scenario == "search" {
                    args.extend(strings(&["--search", "needle"]));
                }
            }
            vec![args]
        }
        "triage" if new => vec![strings(&[
            "issue",
            "edit",
            "42",
            "--title",
            "Revised",
            "--add-label",
            "bug",
            "--json",
        ])],
        "triage" => vec![
            strings(&["label", "list", "--json"]),
            strings(&[
                "api",
                "repos/owner/repo/issues/42",
                "-X",
                "PATCH",
                "--input",
            ]),
            strings(&[
                "api",
                "repos/owner/repo/issues/42/labels",
                "-X",
                "POST",
                "--input",
            ]),
        ],
        _ => panic!("unknown scenario"),
    };
    if scenario == "triage" && !new {
        for (i, body) in [(1, json!({"title":"Revised"})), (2, json!({"labels":[7]}))] {
            let path = root.join(format!("input{i}.json"));
            fs::write(&path, body.to_string()).unwrap();
            commands[i].push(path.to_str().unwrap().to_owned());
        }
    }
    let start = Instant::now();
    let outputs: Vec<_> = commands
        .iter()
        .map(|args| invoke(binary, args, &fixture.host, root))
        .collect();
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    if scenario == "projection" || scenario == "search" {
        let value: Value = serde_json::from_slice(&outputs[0]).unwrap();
        let records = value.as_array().unwrap();
        assert_eq!(
            records.len(),
            if scenario == "search" && new { 6 } else { 120 }
        );
        let ids: Vec<_> = records
            .iter()
            .filter(|v| scenario != "search" || v["title"].as_str().unwrap().contains("needle"))
            .map(|v| v["number"].as_u64().unwrap())
            .collect();
        assert_eq!(
            ids,
            if scenario == "search" {
                vec![20, 40, 60, 80, 100, 120]
            } else {
                (1..=120).collect()
            }
        );
        let expected = issues();
        for record in records {
            let number = record["number"].as_u64().unwrap();
            let original = &expected[usize::try_from(number - 1).unwrap()];
            assert_eq!(record["title"], original["title"]);
            assert_eq!(record["state"], original["state"]);
        }
        if new {
            assert!(records.iter().all(|v| v.as_object().unwrap().len() == 3));
        }
    }
    let t = fixture.traffic.lock().unwrap();
    if scenario == "help" {
        assert_eq!(t.requests, 0);
        assert!(!outputs[0].is_empty());
    }
    if scenario == "triage" {
        assert_eq!(
            t.mutations,
            vec![
                json!({"method":"PATCH","path":"/api/v1/repos/owner/repo/issues/42","body":{"title":"Revised"}}),
                json!({"method":"POST","path":"/api/v1/repos/owner/repo/issues/42/labels","body":{"labels":[7]}})
            ]
        );
        assert_eq!(t.requests, 3);
    }
    if scenario == "projection" || scenario == "search" {
        assert_eq!(t.requests, if scenario == "search" && new { 1 } else { 4 });
    }
    json!({"wall_ms":elapsed,"stdout_bytes":outputs.iter().map(Vec::len).sum::<usize>(),"requests":t.requests,"response_body_bytes":t.response_body_bytes,"http_writes":t.mutations.len(),"cli_invocations":commands.len()})
}
#[test]
fn fixture_shape_smoke() {
    let values = issues();
    assert_eq!(values.len(), 120);
    assert_eq!(
        values
            .iter()
            .filter(|v| v["title"].as_str().unwrap().contains("needle"))
            .count(),
        6
    );
    assert!(values[0]["body"].as_str().unwrap().len() > 4000);
    let fixture = Fixture::new();
    assert!(fixture.host.starts_with("http://127.0.0.1:"));
    drop(fixture);
    let root = std::env::temp_dir().join(format!("fjx-benchmark-smoke-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let result = sample(
        Path::new(env!("CARGO_BIN_EXE_fjx")),
        "projection",
        false,
        &root,
    );
    assert_eq!(result["requests"], 4);
    let result = sample(Path::new(env!("CARGO_BIN_EXE_fjx")), "triage", true, &root);
    assert_eq!(result["http_writes"], 2);
    fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "requires explicit release binaries, runs 15 timed A/B samples"]
fn benchmark_ab() {
    let before = PathBuf::from(std::env::var_os("FJX_BENCH_BEFORE").expect("set FJX_BENCH_BEFORE"));
    let after = std::env::var_os("FJX_BENCH_AFTER").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/fjx"),
        PathBuf::from,
    );
    let root = std::env::temp_dir().join(format!("fjx-benchmark-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let mut results = Vec::new();
    for scenario in ["help", "projection", "search", "triage"] {
        for _ in 0..2 {
            sample(&before, scenario, false, &root);
            sample(&after, scenario, true, &root);
        }
        let mut old = Vec::new();
        let mut new = Vec::new();
        for i in 0..15 {
            if i % 2 == 0 {
                old.push(sample(&before, scenario, false, &root));
                new.push(sample(&after, scenario, true, &root));
            } else {
                new.push(sample(&after, scenario, true, &root));
                old.push(sample(&before, scenario, false, &root));
            }
        }
        let summary = |samples: Vec<Value>| {
            let mut times: Vec<_> = samples
                .iter()
                .map(|v| v["wall_ms"].as_f64().unwrap())
                .collect();
            times.sort_by(f64::total_cmp);
            let mut value = samples[0].clone();
            value["wall_ms"] = json!(times[7]);
            value["samples"] = json!(samples);
            value
        };
        results.push(json!({"scenario":scenario,"before":summary(old),"after":summary(new)}));
    }
    let report = json!({"fixture":"loopback synthetic Forgejo schema","samples":15,"warmups":2,"before":before,"after":after,"results":results});
    let text = serde_json::to_string_pretty(&report).unwrap();
    println!("{text}");
    if let Some(path) = std::env::var_os("FJX_BENCH_REPORT") {
        fs::write(path, text).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}
