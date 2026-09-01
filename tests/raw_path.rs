use std::process::{Command, Output};

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

fn run_without_context(path: &str) -> Output {
    let missing_config = std::env::temp_dir().join(format!(
        "fjx-raw-path-missing-config-{}",
        std::process::id()
    ));
    command()
        .args(["api", path, "-X", "POST", "--dry-run"])
        .current_dir(std::env::temp_dir())
        .env("FJX_CONFIG", missing_config)
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn run_dry_run(path: &str) -> Output {
    command()
        .args([
            "--host",
            "https://forgejo.example",
            "api",
            path,
            "-X",
            "POST",
            "--dry-run",
        ])
        .env("FJX_TOKEN", "test-token")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn absolute_scheme_paths_are_usage_errors_before_context_or_network() {
    for path in [
        "alpha:opaque",
        "https:attacker.example/steal",
        "alpha://attacker.example/steal",
        "a0+-.Z:opaque",
        "MiXeD:opaque",
        "MiXeD://attacker.example/steal",
        "/Custom+V1.2-value:opaque",
    ] {
        let output = run_without_context(path);
        assert_eq!(
            output.status.code(),
            Some(2),
            "path {path:?} produced stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "path {path:?} wrote stdout");
    }
}

#[test]
fn url_shaped_query_values_reach_the_request() {
    let path = "repos/o/r?callback=https://example.com/x";
    let output = run_dry_run(path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "POST\thttps://forgejo.example/api/v1/repos/o/r?callback=https://example.com/x\tnull\n"
    );
}

#[test]
fn control_bytes_and_uri_whitespace_are_usage_errors_before_effects() {
    for path in [
        "repos/o/r\nnext",
        "repos/o/r?value=line\nbreak",
        "repos/o/r\tbad",
        "repos/o/r?value=bad space",
        "repos/o/r\u{1f}bad",
        "repos/o/r\u{7f}bad",
    ] {
        for output in [run_without_context(path), run_dry_run(path)] {
            assert_eq!(
                output.status.code(),
                Some(2),
                "path {path:?} produced stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty(), "path {path:?} wrote stdout");
        }
    }
}
