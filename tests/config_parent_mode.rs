#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

const HOST: &str = "http://127.0.0.1:1";
const TOKEN: &str = "parent-mode-secret";

fn temp_dir(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fjx-config-parent-{label}-{}-{stamp}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap_or_else(|error| panic!("{error}"));
    path
}

fn write_config(path: &Path) {
    fs::write(
        path,
        format!(
            "{{\"version\":1,\"default_host\":\"{HOST}\",\"hosts\":{{\"{HOST}\":{{\"token\":\"{TOKEN}\"}}}}}}"
        ),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn logout(root: &Path, config: &Path) -> Output {
    Command::new("/bin/sh")
        .args(["-c", "umask 0777; exec \"$@\"", "sh"])
        .arg(env!("CARGO_BIN_EXE_fjx"))
        .args(["auth", "logout", "--json", "--host", HOST])
        .current_dir(root)
        .env("FJX_CONFIG", config)
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn assert_clean_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"kind\":\"result\",\"action\":\"auth.logout\",\"ok\":true,\"number\":null,\"html_url\":null}\n"
    );
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|error| panic!("{error}"))
        .permissions()
        .mode()
        & 0o777
}

fn assert_atomic_mode_0600_replacement(config: &Path, old_inode: u64) {
    let metadata = fs::metadata(config).unwrap_or_else(|error| panic!("{error}"));
    assert!(metadata.is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_ne!(metadata.ino(), old_inode);
    assert!(
        !fs::read_to_string(config)
            .unwrap_or_else(|error| panic!("{error}"))
            .contains(TOKEN)
    );
    let parent = config
        .parent()
        .unwrap_or_else(|| panic!("config path has no parent"));
    assert_eq!(
        fs::read_dir(parent)
            .unwrap_or_else(|error| panic!("{error}"))
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".fjx-hosts."))
            .count(),
        0
    );
}

#[test]
fn absolute_existing_parent_keeps_its_mode() {
    let root = temp_dir("absolute");
    let parent = root.join("shared-config");
    fs::create_dir(&parent).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("{error}"));
    let config = parent.join("hosts.json");
    write_config(&config);
    let old_inode = fs::metadata(&config)
        .unwrap_or_else(|error| panic!("{error}"))
        .ino();

    let output = logout(&root, &config);

    assert_clean_success(&output);
    assert_eq!(mode(&parent), 0o755);
    assert_atomic_mode_0600_replacement(&config, old_inode);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn relative_existing_parent_keeps_its_mode() {
    let root = temp_dir("relative");
    let parent = root.join("relative-config");
    fs::create_dir(&parent).unwrap_or_else(|error| panic!("{error}"));
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o751))
        .unwrap_or_else(|error| panic!("{error}"));
    let config = parent.join("hosts.json");
    write_config(&config);
    let old_inode = fs::metadata(&config)
        .unwrap_or_else(|error| panic!("{error}"))
        .ino();

    let output = logout(&root, Path::new("relative-config/hosts.json"));

    assert_clean_success(&output);
    assert_eq!(mode(&parent), 0o751);
    assert_atomic_mode_0600_replacement(&config, old_inode);
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn newly_created_config_leaf_is_mode_0700() {
    let root = temp_dir("new-leaf");
    let parent = root.join("private-config");
    let config = parent.join("hosts.json");

    let output = logout(&root, &config);

    assert_clean_success(&output);
    assert_eq!(mode(&parent), 0o700);
    assert_eq!(mode(&config), 0o600);
    let saved = fs::read_to_string(&config).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(saved, "{\"version\":1,\"default_host\":null,\"hosts\":{}}");
    fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
}
