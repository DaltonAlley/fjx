#![allow(
    dead_code,
    reason = "test harness imports private binary modules directly"
)]

#[path = "../src/args.rs"]
mod args;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/help.rs"]
mod help;
#[path = "../src/output.rs"]
mod output;

use serde_json::Value;
use std::collections::BTreeSet;
use std::process::{Command, Stdio};

fn path(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn schema(words: &[&str]) -> Value {
    let outcome = help::schema(&path(words)).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(outcome.code, 0);
    serde_json::from_slice(&outcome.bytes).unwrap_or_else(|error| panic!("{error}"))
}

fn text(words: &[&str]) -> String {
    let outcome = help::render(&path(words)).unwrap_or_else(|error| panic!("{error}"));
    String::from_utf8(outcome.bytes).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn concise_root_preserves_version_and_api_contract() {
    let root = text(&[]);
    assert!(root.starts_with(concat!(
        "fjx ",
        env!("CARGO_PKG_VERSION"),
        " - a small Forgejo client\n"
    )));
    assert!(root.contains("fjx api PATH"));
    assert!(root.len() < 1200);
    assert!(!root.contains("--match-head"));
}

#[test]
fn all_group_and_leaf_help_comes_from_schema_metadata() {
    let root = schema(&[]);
    assert_eq!(root["schema_version"], 1);
    let commands = root["commands"]
        .as_array()
        .unwrap_or_else(|| panic!("commands array"));
    let mut names = BTreeSet::new();
    for command in commands {
        let name = command["name"]
            .as_str()
            .unwrap_or_else(|| panic!("command name"));
        assert!(names.insert(name));
        let words: Vec<_> = name.split_whitespace().collect();
        let leaf = schema(&words);
        assert_eq!(leaf["commands"].as_array().map(Vec::len), Some(1));
        let rendered = text(&words);
        assert!(
            rendered.contains(command["usage"].as_str().unwrap_or_default()),
            "{name}"
        );
        assert!(rendered.contains("Example:"), "{name}");
        assert!(command["flags"].is_array());
        assert!(command["examples"].is_array());
        assert!(command["write"].is_boolean());
        assert!(command["destructive"].is_boolean());
        let fields = command["output_fields"]
            .as_array()
            .unwrap_or_else(|| panic!("fields array"));
        let mut seen = BTreeSet::new();
        for field in fields {
            let field = field.as_str().unwrap_or_else(|| panic!("field name"));
            assert!(!field.is_empty());
            assert!(field.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert!(seen.insert(field));
            assert!(!field.contains("token"));
            assert!(rendered.contains(field));
        }
    }
    for group in [
        "auth",
        "repo",
        "issue",
        "pr",
        "run",
        "release",
        "label",
        "milestone",
        "branch",
        "workflow",
    ] {
        let group_schema = schema(&[group]);
        let rendered = text(&[group]);
        for command in group_schema["commands"]
            .as_array()
            .unwrap_or_else(|| panic!("group commands"))
        {
            assert!(
                command["name"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with(&format!("{group} "))
            );
            assert!(rendered.contains(command["usage"].as_str().unwrap_or_default()));
        }
        assert!(!rendered.contains("fjx api PATH"));
    }
}

#[test]
fn leaf_help_explains_filters_defaults_and_safety() {
    let list = text(&["issue", "list"]);
    for flag in [
        "--search",
        "--since",
        "--before",
        "--sort",
        "default 30",
        "default open",
    ] {
        assert!(list.contains(flag), "{flag}");
    }
    let merge = text(&["pr", "merge"]);
    for flag in [
        "--match-head",
        "--auto",
        "--yes",
        "--dry-run",
        "never retried",
    ] {
        assert!(merge.contains(flag), "{flag}");
    }
    assert!(!merge.contains("--with-token"));
    assert!(text(&["pr", "review"]).contains("--comments-file"));
    assert!(text(&["issue", "view"]).contains("--human"));
}

#[test]
fn unknown_paths_are_usage_errors_with_scoped_hints() {
    for words in [
        &["unknown"][..],
        &["issue", "unknown"],
        &["pr", "merge", "extra"],
    ] {
        let error = help::render(&path(words))
            .err()
            .unwrap_or_else(|| panic!("expected error"));
        assert_eq!(error.code(), 2);
        assert!(error.to_string().contains("fjx help"));
        assert!(help::schema(&path(words)).is_err());
    }
}

#[test]
fn projection_uses_typed_metadata_and_rejects_raw_commands() {
    let fields = help::output_fields(&args::Command::Issue(args::IssueArgs::View { number: 1 }))
        .unwrap_or_else(|| panic!("issue fields"));
    assert_eq!(
        fields,
        help::output_fields_for("issue", "view").unwrap_or_default()
    );
    assert!(fields.contains(&"title"));
    assert!(help::output_fields(&args::Command::Help { path: vec![] }).is_none());
    assert!(help::output_fields_for("missing", "view").is_none());
    assert!(
        help::output_fields_for("pr", "files")
            .unwrap_or_default()
            .contains(&"filename")
    );
    assert!(
        help::output_fields_for("issue", "comment")
            .unwrap_or_default()
            .contains(&"id")
    );
}

#[test]
fn command_help_and_schema_skip_context_stdin_and_network() {
    for words in [
        &["help", "issue", "create"][..],
        &["pr", "review", "--help"],
        &["schema", "pr", "files", "--json"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_fjx"))
            .args(words)
            .env("FJX_HOST", "not a valid URL")
            .env("FJX_REPO", "invalid/repo/extra")
            .env_remove("FJX_TOKEN")
            .env_remove("FORGEJO_TOKEN")
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(
            result.status.success(),
            "{words:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty());
        assert!(!result.stdout.is_empty());
    }
}
