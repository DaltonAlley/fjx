#![allow(clippy::expect_used, reason = "test fixtures must parse successfully")]
#![allow(
    dead_code,
    reason = "parser integration tests include the binary's private modules"
)]
#[path = "../src/args.rs"]
mod args;
#[path = "../src/error.rs"]
mod error;

use args::{Command, IssueArgs, PullArgs, RunArgs};
use std::ffi::OsString;

fn parse(words: &[&str]) -> Result<args::Args, error::Error> {
    args::parse_from(words.iter().map(OsString::from))
}

#[test]
fn output_modes_and_projection() {
    let parsed = parse(&["issue", "view", "1", "--json", "--fields", "number,title"])
        .expect("valid parser fixture");
    assert_eq!(parsed.fields, ["number", "title"]);
    assert!(
        parse(&["issue", "view", "1", "--human"])
            .expect("valid parser fixture")
            .human
    );
    for words in [
        vec!["issue", "view", "1", "--fields", "title"],
        vec!["issue", "view", "1", "--json", "--human"],
        vec!["issue", "list", "--human"],
        vec!["issue", "view", "1", "--json", "--fields", "title,"],
    ] {
        assert!(parse(&words).is_err(), "{words:?}");
    }
}

#[test]
fn scoped_help_skips_required_inputs_but_rejects_wrong_scopes() {
    for words in [
        vec!["pr", "create", "--draft", "--help"],
        vec!["issue", "comment", "--body-file", "missing", "--help"],
        vec!["pr", "review", "--comments-file", "-", "--help"],
        vec!["workflow", "dispatch", "--field", "k=v", "--help"],
        vec!["help", "issue", "edit"],
        vec!["--help"],
    ] {
        assert!(
            matches!(
                parse(&words).expect("valid parser fixture").command,
                Command::Help { .. }
            ),
            "{words:?}"
        );
    }
    for words in [
        vec!["pr", "view", "--draft", "--help"],
        vec!["help", "bogus"],
        vec!["issue", "list", "--unknown", "--help"],
        vec!["pr", "list", "--search", "x", "--help"],
    ] {
        assert!(parse(&words).is_err(), "{words:?}");
    }
    assert!(
        matches!(parse(&["schema", "pr", "review", "--json"]).expect("valid parser fixture").command, Command::Schema { path } if path == ["pr", "review"])
    );
    assert!(parse(&["schema", "issue"]).is_err());
}

#[test]
fn metadata_and_edit_contracts() {
    let parsed = parse(&[
        "issue",
        "create",
        "--title",
        "x",
        "--label",
        "bug",
        "--label-id",
        "2",
        "--assignee",
        "alice",
        "--milestone-id",
        "3",
    ])
    .expect("valid parser fixture");
    let Command::Issue(IssueArgs::Create { metadata, .. }) = parsed.command else {
        panic!("wrong command")
    };
    assert_eq!(metadata.labels, ["bug"]);
    assert_eq!(metadata.label_ids, [2]);
    assert_eq!(metadata.assignees, ["alice"]);
    assert_eq!(metadata.milestone_id, Some(3));
    let Command::Pull(PullArgs::Edit { edit, .. }) = parse(&[
        "pr",
        "edit",
        "2",
        "--base",
        "next",
        "--add-label",
        "bug",
        "--clear-milestone",
    ])
    .expect("valid parser fixture")
    .command
    else {
        panic!("wrong command")
    };
    assert_eq!(edit.base.as_deref(), Some("next"));
    assert!(edit.clear_milestone);
    for words in [
        vec!["issue", "edit", "1"],
        vec!["issue", "edit", "1", "--base", "main"],
        vec![
            "issue",
            "edit",
            "1",
            "--add-label",
            "a",
            "--remove-label",
            "a",
        ],
        vec![
            "issue",
            "edit",
            "1",
            "--clear-milestone",
            "--milestone",
            "m",
        ],
        vec![
            "issue",
            "create",
            "--title",
            "x",
            "--milestone",
            "m",
            "--milestone-id",
            "1",
        ],
        vec!["issue", "create", "--title", "x", "--label-id", "0"],
    ] {
        assert!(parse(&words).is_err(), "{words:?}");
    }
}

#[test]
fn list_filters_and_dates() {
    let Command::Issue(IssueArgs::List { filters, .. }) = parse(&[
        "issue",
        "list",
        "--label",
        "bug",
        "--assignee",
        "alice",
        "--author",
        "bob",
        "--since",
        "2024-02-29T00:00:00Z",
        "--search",
        "words",
    ])
    .expect("valid parser fixture")
    .command
    else {
        panic!("wrong command")
    };
    assert_eq!(filters.assignee.as_deref(), Some("alice"));
    assert_eq!(filters.labels, ["bug"]);
    for words in [
        vec!["issue", "list", "--since", "2023-02-29T00:00:00Z"],
        vec!["pr", "list", "--search", "x"],
        vec!["pr", "list", "--assignee", "alice"],
    ] {
        assert!(parse(&words).is_err(), "{words:?}");
    }
    let Command::Run(RunArgs::List { filters, .. }) = parse(&[
        "run",
        "list",
        "--event",
        "push",
        "--ref",
        "main",
        "--status",
        "success",
        "--head-sha",
        "abc",
        "--workflow",
        "ci.yml",
    ])
    .expect("valid parser fixture")
    .command
    else {
        panic!("wrong command")
    };
    assert_eq!(filters.reference.as_deref(), Some("main"));
    assert_eq!(filters.event.as_deref(), Some("push"));
}

#[test]
fn new_read_workflows_and_paging() {
    for command in ["comments", "reviews", "files"] {
        assert!(parse(&["pr", command, "1", "--all", "--limit", "50"]).is_ok());
    }
    assert!(matches!(
        parse(&["pr", "review-comments", "1", "2"])
            .expect("valid parser fixture")
            .command,
        Command::Pull(PullArgs::ReviewComments {
            number: 1,
            review_id: 2,
            ..
        })
    ));
    assert!(parse(&["issue", "comments", "1", "--page", "2", "--all"]).is_err());
    assert!(
        parse(&[
            "pr",
            "request-review",
            "1",
            "--reviewer",
            "alice",
            "--team",
            "maintainers"
        ])
        .is_ok()
    );
    assert!(parse(&["pr", "request-review", "1"]).is_err());
}

#[test]
fn review_and_merge_safety() {
    assert!(
        parse(&[
            "pr",
            "review",
            "1",
            "--event",
            "comment",
            "--comments-file",
            "-",
            "--commit",
            "abc"
        ])
        .is_ok()
    );
    assert!(
        parse(&[
            "pr",
            "review",
            "1",
            "--event",
            "comment",
            "--comments-file",
            "-"
        ])
        .is_err()
    );
    assert!(parse(&["pr", "review", "1", "--event", "request-changes"]).is_err());
    let parsed = parse(&[
        "pr",
        "merge",
        "1",
        "--match-head",
        "abc",
        "--auto",
        "--yes",
        "--dry-run",
    ])
    .expect("valid parser fixture");
    assert!(parsed.yes && parsed.dry_run);
    assert!(
        matches!(parsed.command, Command::Pull(PullArgs::Merge { match_head: Some(head), auto: true, .. }) if head == "abc")
    );
}
