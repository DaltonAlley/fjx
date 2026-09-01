#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const HOST: &str = "https://forge.example/code";
const HELPER_KEY: &str = "credential.https://forge.example/code.helper";
const PATH_KEY: &str = "credential.https://forge.example/code.useHttpPath";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    git_config: PathBuf,
    bin: PathBuf,
    real_git: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = temp_dir();
        let config = root.join("hosts.json");
        let git_config = root.join("gitconfig");
        fs::write(
            &config,
            "{\"version\":1,\"default_host\":\"https://forge.example/code\",\"hosts\":{\"https://forge.example/code\":{\"user\":\"agent\",\"token\":\"saved-secret\"}}}",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|error| panic!("{error}"));

        let real_git = find_git();
        add_value(&real_git, &git_config, HELPER_KEY, "first-secret-helper");
        add_value(&real_git, &git_config, HELPER_KEY, "");
        add_value(&real_git, &git_config, HELPER_KEY, "last-helper");
        add_value(&real_git, &git_config, PATH_KEY, "old-secret-path");
        add_value(&real_git, &git_config, PATH_KEY, "false");

        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
        let wrapper = bin.join("git");
        fs::write(
            &wrapper,
            r#"#!/bin/sh
set -eu
mutating=0
for argument in "$@"; do
    case "$argument" in
        --replace-all|--add|--unset-all) mutating=1 ;;
    esac
done
if [ "$mutating" -eq 0 ]; then
    exec "$FJX_TEST_REAL_GIT" "$@"
fi
count=0
if [ -f "$FJX_TEST_ROOT/count" ]; then
    count=$(sed -n '1p' "$FJX_TEST_ROOT/count")
fi
count=$((count + 1))
printf '%s\n' "$count" > "$FJX_TEST_ROOT/count"
if [ -f "$FJX_TEST_ROOT/primary-failed" ] && [ "${FJX_TEST_FAIL_ROLLBACK:-0}" = 1 ] && [ ! -f "$FJX_TEST_ROOT/rollback-failed" ]; then
    : > "$FJX_TEST_ROOT/rollback-failed"
    printf '%s\n' 'child stderr old-secret-path saved-secret' >&2
    exit 43
fi
if [ "$count" = "${FJX_TEST_FAIL_AFTER:-0}" ]; then
    "$FJX_TEST_REAL_GIT" "$@"
    : > "$FJX_TEST_ROOT/primary-failed"
    printf '%s\n' 'child stderr first-secret-helper saved-secret' >&2
    exit 42
fi
exec "$FJX_TEST_REAL_GIT" "$@"
"#,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|error| panic!("{error}"));

        Self {
            root,
            config,
            git_config,
            bin,
            real_git,
        }
    }

    fn run(&self, fail_after: Option<u8>, fail_rollback: bool) -> Output {
        let mut paths = vec![self.bin.clone()];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_fjx"));
        command
            .args(["auth", "setup-git", "--host", HOST])
            .env("FJX_CONFIG", &self.config)
            .env("GIT_CONFIG_GLOBAL", &self.git_config)
            .env("FJX_TEST_ROOT", &self.root)
            .env("FJX_TEST_REAL_GIT", &self.real_git)
            .env(
                "PATH",
                std::env::join_paths(paths).unwrap_or_else(|error| panic!("{error}")),
            )
            .env_remove("FJX_HOST")
            .env_remove("FJX_REPO")
            .env_remove("FJX_TOKEN")
            .env_remove("FORGEJO_TOKEN")
            .env_remove("XDG_CONFIG_HOME");
        if let Some(call) = fail_after {
            command.env("FJX_TEST_FAIL_AFTER", call.to_string());
        }
        if fail_rollback {
            command.env("FJX_TEST_FAIL_ROLLBACK", "1");
        }
        command.output().unwrap_or_else(|error| panic!("{error}"))
    }

    fn values(&self, key: &str) -> Vec<Vec<u8>> {
        values(&self.real_git, &self.git_config, key)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap_or_else(|error| panic!("{error}"));
    }
}

#[test]
fn setup_git_commits_all_scoped_values_on_success() {
    let fixture = Fixture::new();

    let output = fixture.run(None, false);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"auth.setup-git\tok\n");
    assert_eq!(fixture.values(PATH_KEY), vec![b"true".to_vec()]);
    let helper_values = fixture.values(HELPER_KEY);
    assert_eq!(helper_values.len(), 2);
    assert!(helper_values[0].is_empty());
    assert_eq!(
        String::from_utf8_lossy(&helper_values[1]),
        format!("!'{}' auth git-credential", env!("CARGO_BIN_EXE_fjx"))
    );
}

#[test]
fn setup_git_restores_exact_values_after_each_mutation_failure() {
    for failed_call in 1..=3 {
        let fixture = Fixture::new();
        let helper_before = fixture.values(HELPER_KEY);
        let path_before = fixture.values(PATH_KEY);

        let output = fixture.run(Some(failed_call), false);

        assert_eq!(output.status.code(), Some(8));
        assert!(output.stdout.is_empty());
        assert_eq!(fixture.values(HELPER_KEY), helper_before);
        assert_eq!(fixture.values(PATH_KEY), path_before);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("Git config update failed during"));
        assert!(error.contains("restored keys"));
        assert!(!error.contains("first-secret-helper"));
        assert!(!error.contains("old-secret-path"));
        assert!(!error.contains("saved-secret"));
        assert!(!error.contains("child stderr"));
    }
}

#[test]
fn setup_git_reports_both_failures_without_leaking_values() {
    let fixture = Fixture::new();

    let output = fixture.run(Some(2), true);

    assert_eq!(output.status.code(), Some(8));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("set HTTP path matching"));
    assert!(error.contains("exit status: 42"));
    assert!(error.contains("warning: Git config restoration is incomplete"));
    assert!(error.contains(HELPER_KEY));
    assert!(error.contains(PATH_KEY));
    assert!(error.contains("restore"));
    assert!(error.contains("exit status: 43"));
    assert!(!error.contains("first-secret-helper"));
    assert!(!error.contains("old-secret-path"));
    assert!(!error.contains("saved-secret"));
    assert!(!error.contains("child stderr"));
}

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-setup-git-transaction-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn find_git() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success());
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

fn add_value(git: &Path, config: &Path, key: &str, value: &str) {
    let output = Command::new(git)
        .args(["config", "--file"])
        .arg(config)
        .args(["--add", key, value])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn values(git: &Path, config: &Path, key: &str) -> Vec<Vec<u8>> {
    let output = Command::new(git)
        .args(["config", "--file"])
        .arg(config)
        .args(["--null", "--get-all", key])
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(output.status.success());
    output
        .stdout
        .strip_suffix(&[0])
        .unwrap_or(&output.stdout)
        .split(|byte| *byte == 0)
        .map(<[u8]>::to_vec)
        .collect()
}
