#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

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

fn temp_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-config-context-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

#[cfg(unix)]
fn write_config(path: &Path, host: &str, token: &str) {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "default_host": host,
        "hosts": {host: {"token": token}},
    }))
    .unwrap_or_else(|error| panic!("{error}"));
    fs::write(path, bytes).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn assert_context_error_without_secrets(output: &std::process::Output, fragments: &[&[u8]]) {
    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    for fragment in fragments {
        assert!(
            !output
                .stderr
                .windows(fragment.len())
                .any(|bytes| bytes == *fragment),
            "token fragment leaked in stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[cfg(unix)]
fn relative_config_leaf_saves_in_cwd_without_changing_cwd_mode() {
    let root = temp_dir("relative");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o751))
        .unwrap_or_else(|error| panic!("{error}"));
    let config = root.join("hosts.json");
    let host = "http://127.0.0.1:1";
    let token = "saved-secret";
    write_config(&config, host, token);

    let output = command()
        .args(["auth", "logout", "--json", "--host", host])
        .current_dir(&root)
        .env("FJX_CONFIG", "hosts.json")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(token));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(token));
    assert_eq!(
        fs::metadata(&config)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&root)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    assert!(
        !fs::read_to_string(&config)
            .unwrap_or_else(|error| panic!("{error}"))
            .contains(token)
    );
    assert_eq!(
        fs::read_dir(&root)
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".fjx-hosts."))
            .count(),
        0
    );

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn empty_fjx_token_does_not_fall_through_to_forgejo_token() {
    let root = temp_dir("empty-fjx");
    let output = command()
        .args(["auth", "status", "--host", "http://127.0.0.1:1"])
        .env("FJX_CONFIG", root.join("hosts.json"))
        .env("FJX_TOKEN", "")
        .env("FORGEJO_TOKEN", "lower-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_context_error_without_secrets(&output, &[b"lower-secret"]);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn non_utf8_fjx_token_does_not_fall_through_to_forgejo_token() {
    let root = temp_dir("non-utf8-fjx");
    let output = command()
        .args(["auth", "status", "--host", "http://127.0.0.1:1"])
        .env("FJX_CONFIG", root.join("hosts.json"))
        .env(
            "FJX_TOKEN",
            OsString::from_vec(b"top-secret-\xff-value".to_vec()),
        )
        .env("FORGEJO_TOKEN", "lower-secret")
        .output()
        .unwrap_or_else(|error| panic!("{error}"));

    assert_context_error_without_secrets(&output, &[b"top-secret", b"value", b"lower-secret"]);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[cfg(unix)]
fn invalid_forgejo_token_does_not_fall_through_to_config() {
    let root = temp_dir("invalid-forgejo");
    let config = root.join("hosts.json");
    let host = "http://127.0.0.1:1";
    write_config(&config, host, "config-secret");

    for value in [
        OsString::from(""),
        OsString::from_vec(b"forgejo-\xff-secret".to_vec()),
    ] {
        let output = command()
            .args(["auth", "status", "--host", host])
            .env("FJX_CONFIG", &config)
            .env("FORGEJO_TOKEN", value)
            .output()
            .unwrap_or_else(|error| panic!("{error}"));

        assert_context_error_without_secrets(&output, &[b"forgejo", b"secret", b"config-secret"]);
    }

    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
