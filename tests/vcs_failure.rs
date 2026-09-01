#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-vcs-failure-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn write_tool(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn run_without_context(root: &Path, bin: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fjx"))
        .args(["repo", "view"])
        .current_dir(root)
        .env("PATH", bin)
        .env("HOME", root)
        .env("FJX_CONFIG", root.join("hosts.json"))
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn assert_empty_stdout(output: &Output) {
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn missing_vcs_tools_are_a_local_error() {
    let root = temp_dir("missing-tools");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));

    let output = run_without_context(&root, &bin);

    assert_eq!(
        output.status.code(),
        Some(8),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_empty_stdout(&output);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn vcs_command_failures_are_local_errors_without_exposing_stderr() {
    let root = temp_dir("command-errors");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    write_tool(
        &bin,
        "jj",
        "printf '%s\\n' 'jj-private-value' >&2\nexit 71\n",
    );
    write_tool(
        &bin,
        "git",
        "printf '%s\\n' 'git-private-value' >&2\nexit 72\n",
    );

    let output = run_without_context(&root, &bin);

    assert_eq!(
        output.status.code(),
        Some(8),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_empty_stdout(&output);
    assert!(
        !output
            .stderr
            .windows(13)
            .any(|part| part == b"private-value")
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn git_not_a_repository_is_normal_context_absence() {
    let root = temp_dir("not-a-repo");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    write_tool(&bin, "jj", "exit 1\n");
    write_tool(
        &bin,
        "git",
        "if [ \"$LC_ALL\" = C ]; then\n  printf '%s\\n' 'fatal: not a git repository (or any of the parent directories): .git' >&2\n  exit 128\nfi\nprintf '%s\\n' 'origin http://127.0.0.1:1/owner/repo (fetch)'\n",
    );

    let output = run_without_context(&root, &bin);

    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_empty_stdout(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("repository context is required"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn valid_empty_remote_lists_are_normal_context_absence() {
    let root = temp_dir("empty-remotes");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    write_tool(&bin, "jj", "exit 0\n");
    write_tool(&bin, "git", "exit 0\n");

    let output = run_without_context(&root, &bin);

    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_empty_stdout(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("repository context is required"));
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn jj_failure_falls_back_to_a_valid_git_remote_list() {
    let root = temp_dir("git-fallback");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap_or_else(|error| panic!("{error}"));
    write_tool(
        &bin,
        "jj",
        "if [ \"$#\" -ne 4 ] || [ \"$1\" != --ignore-working-copy ] || [ \"$2\" != git ] || [ \"$3\" != remote ] || [ \"$4\" != list ]; then\n  exit 70\nfi\nprintf '%s\\n' 'jj-private-value' >&2\nexit 1\n",
    );
    write_tool(
        &bin,
        "git",
        "if [ \"$#\" -ne 2 ] || [ \"$1\" != remote ] || [ \"$2\" != -v ]; then\n  exit 71\nfi\nprintf '%s\\n' 'origin http://127.0.0.1:1/owner/repo (fetch)'\n",
    );

    let output = run_without_context(&root, &bin);

    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_empty_stdout(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("no token is available"));
    assert!(
        !output
            .stderr
            .windows(13)
            .any(|part| part == b"private-value")
    );
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
