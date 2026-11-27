//! `auth list` reads saved logins from the token file only: no network, no
//! token check, and no token in any output.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fjx-auth-list-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join("hosts.json")
    }

    fn write_config(&self, value: &Value) {
        fs::write(self.config(), value.to_string()).unwrap_or_else(|error| panic!("{error}"));
        fs::set_permissions(self.config(), fs::Permissions::from_mode(0o600))
            .unwrap_or_else(|error| panic!("{error}"));
    }

    fn run(&self, args: &[&str]) -> Output {
        // An unreachable proxy and no git on PATH: any request or repository
        // lookup would fail the command.
        let output = Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args(args)
            .current_dir(&self.0)
            .env_clear()
            .env("HOME", &self.0)
            .env("FJX_CONFIG", self.config())
            .env("FJX_TOKEN", "environment-secret")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("PATH", self.0.join("no-bin"))
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains("secret"), "token leaked: {text}");
        }
        output
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lists_saved_logins_without_network_or_tokens() {
    let root = Root::new();
    root.write_config(&json!({
        "version": 1,
        "default_host": "https://b.example/forgejo",
        "hosts": {
            "https://b.example/forgejo": {
                "user": "dalton",
                "token": "b-secret",
                "oauth": {"client_id": "client", "refresh_token": "refresh-secret", "expires_at": 1},
                "ssh_aliases": ["b-ssh", "b2-ssh"],
            },
            // Older files can lack a user name.
            "https://a.example": {"token": "a-secret"},
        },
    }));
    let output = root.run(&["auth", "list", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        value,
        json!([
            {"kind":"login","host":"https://a.example","user":null,"default":false,"ssh_aliases":[]},
            {"kind":"login","host":"https://b.example/forgejo","user":"dalton","default":true,
             "ssh_aliases":["b-ssh","b2-ssh"]},
        ])
    );

    let plain = root.run(&["auth", "list"]);
    assert!(plain.status.success());
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout),
        "https://a.example\t\tfalse\t\nhttps://b.example/forgejo\tdalton\ttrue\tb-ssh,b2-ssh\n"
    );

    let fields = root.run(&["auth", "list", "--json", "--fields", "host,default"]);
    assert!(fields.status.success());
    let value: Value = serde_json::from_slice(&fields.stdout).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        value[1],
        json!({"host":"https://b.example/forgejo","default":true})
    );
}

#[test]
fn missing_token_file_lists_nothing() {
    let root = Root::new();
    let output = root.run(&["auth", "list", "--json"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"[]\n");
    let plain = root.run(&["auth", "list"]);
    assert!(plain.status.success());
    assert!(plain.stdout.is_empty());
}

#[test]
fn rejects_scoping_flags_and_unsafe_token_files() {
    let root = Root::new();
    for args in [
        &["auth", "list", "--host", "https://a.example"][..],
        &["auth", "list", "-R", "owner/repo"],
        &["auth", "list", "--dry-run"],
        &["auth", "list", "--yes"],
        &["auth", "list", "--all"],
    ] {
        let output = root.run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    root.write_config(&json!({"version": 1, "default_host": null, "hosts": {}}));
    fs::set_permissions(root.config(), fs::Permissions::from_mode(0o644))
        .unwrap_or_else(|error| panic!("{error}"));
    let output = root.run(&["auth", "list"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
}

#[test]
fn token_file_rewrites_keep_fields_from_other_builds() {
    let root = Root::new();
    root.write_config(&json!({
        "version": 1,
        "default_host": "https://a.example",
        "future_top": {"keep": true},
        "hosts": {"https://a.example": {"user": "u", "token": "a-secret", "future_host": [1]}},
    }));
    let output = root.run(&[
        "auth",
        "add-ssh-alias",
        "a-ssh",
        "--host",
        "https://a.example",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(root.config()).unwrap_or_else(|error| panic!("{error}")))
            .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(saved["future_top"], json!({"keep": true}));
    let entry = &saved["hosts"]["https://a.example"];
    assert_eq!(entry["future_host"], json!([1]));
    assert_eq!(entry["ssh_aliases"], json!(["a-ssh"]));
    assert_eq!(entry["token"], "a-secret");
}
