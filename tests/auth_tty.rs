use std::io::Write;
use std::process::{Command, Output, Stdio};

const TOKEN: &str = "pty-secret-4815";

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

fn login_in_terminal(json: bool) -> Output {
    let executable = env!("CARGO_BIN_EXE_fjx");
    let mut arguments = vec![executable, "auth", "login", "--host", "http://127.0.0.1:0"];
    if json {
        arguments.push("--json");
    }
    let command_line = arguments
        .into_iter()
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");

    let mut command = Command::new("script");
    command
        .args(["--quiet", "--echo", "never", "--return", "--command"])
        .arg(command_line)
        .arg("/dev/null");
    let mut child = command
        .env_remove("FJX_HOST")
        .env_remove("FJX_REPO")
        .env_remove("FJX_TOKEN")
        .env_remove("FORGEJO_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .env("SHELL", "/bin/sh")
        .env("FJX_CONFIG", "/tmp/fjx-auth-tty-unused-config")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("could not start script: {error}"));
    writeln!(
        child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("missing script stdin")),
        "{TOKEN}"
    )
    .unwrap_or_else(|error| panic!("could not send token: {error}"));
    child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("could not wait for script: {error}"))
}

fn terminal_text(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn shell_quote_handles_empty_and_special_words() {
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("two words"), "'two words'");
    assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    assert_eq!(shell_quote("$(printf leaked)"), "'$(printf leaked)'");
}

#[test]
fn plain_login_keeps_the_hidden_token_prompt() {
    let output = login_in_terminal(false);
    let terminal = terminal_text(&output);

    assert_eq!(output.status.code(), Some(4), "terminal: {terminal}");
    assert!(terminal.contains("Token: "), "terminal: {terminal}");
    assert!(!terminal.contains(TOKEN), "token leaked: {terminal}");
}

#[test]
fn json_login_does_not_write_a_token_prompt() {
    let output = login_in_terminal(true);
    let terminal = terminal_text(&output);

    assert_eq!(output.status.code(), Some(4), "terminal: {terminal}");
    assert!(!terminal.contains("Token:"), "terminal: {terminal}");
    assert!(!terminal.contains(TOKEN), "token leaked: {terminal}");
}
