use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, IsTerminal, Read};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::process::{Command as ProcessCommand, Output};

use serde::{Deserialize, Serialize};

use crate::args::{Args, GitCredentialOperation};
use crate::config::{self, Config, Token};
use crate::context::Context;
use crate::error::Error;
use crate::http::Client;
use crate::output::{Outcome, plain_field};

#[derive(Deserialize)]
struct UserResponse {
    login: String,
}

#[derive(Serialize)]
struct AuthRecord<'a> {
    kind: &'static str,
    host: &'a str,
    user: &'a str,
    source: &'a str,
}

#[derive(Serialize)]
struct ResultRecord {
    kind: &'static str,
    action: &'static str,
    ok: bool,
    number: Option<u64>,
    html_url: Option<String>,
}

pub(crate) fn login(args: &Args, with_token: bool) -> Result<Outcome, Error> {
    require_persistence()?;
    reject_auth_flags(args)?;
    let mut context = Context::resolve(args, false, false)?;
    let token = read_token(with_token, args.json)?;
    let client = Client::new(context.host.clone(), token.clone());
    let user = authenticated_user(&client)?;
    context.config.login(&context.host, user.login, token)?;
    context.config.save(config_path(&context)?)?;
    result(args.json, "auth.login")
}

pub(crate) fn setup_git(args: &Args) -> Result<Outcome, Error> {
    require_persistence()?;
    reject_auth_flags(args)?;
    let mut context = Context::resolve(args, false, false)?;
    if !context.config.has_git_login(&context.host) {
        let token = context.config.token(&context.host).ok_or_else(|| {
            Error::context("auth setup-git requires a saved token; run fjx auth login first")
        })?;
        let client = Client::new(context.host.clone(), token);
        let user = authenticated_user(&client)?;
        context.config.save_user(&context.host, user.login)?;
        context.config.save(config_path(&context)?)?;
    }
    let helper = git_helper_command()?;
    let helper_key = OsString::from(format!("credential.{}.helper", context.host.as_str()));
    let path_key = OsString::from(format!("credential.{}.useHttpPath", context.host.as_str()));
    let snapshots = [
        GitConfigSnapshot::read(helper_key)?,
        GitConfigSnapshot::read(path_key)?,
    ];
    update_git_config(&snapshots, helper.as_os_str())?;
    result(args.json, "auth.setup-git")
}

pub(crate) fn git_credential(
    args: &Args,
    operation: GitCredentialOperation,
) -> Result<Outcome, Error> {
    require_persistence()?;
    reject_auth_flags(args)?;
    if args.host.is_some() || args.repo.is_some() || args.json {
        return Err(Error::usage(
            "common flags are not valid for the Git credential helper",
        ));
    }
    let request = GitCredentialRequest::read()?;
    if !matches!(operation, GitCredentialOperation::Get) {
        return Ok(Outcome::text(""));
    }
    let Some(protocol) = request.protocol else {
        return Ok(Outcome::text(""));
    };
    let Some(host) = request.host else {
        return Ok(Outcome::text(""));
    };
    let config = Config::load(&config::path()?)?;
    let Some(credential) =
        config.git_credential(&protocol, &host, request.path.as_deref().unwrap_or(""))
    else {
        return Ok(Outcome::text(""));
    };
    Ok(Outcome::text(format!(
        "username={}\npassword={}\n",
        credential.user,
        credential.token.as_str()
    )))
}

#[cfg(unix)]
fn git_helper_command() -> Result<OsString, Error> {
    let executable = std::env::current_exe()
        .map_err(|error| Error::local(format!("could not locate fjx: {error}")))?;
    let mut command = Vec::from(&b"!"[..]);
    shell_quote(executable.as_os_str(), &mut command);
    command.extend_from_slice(b" auth git-credential");
    Ok(OsString::from_vec(command))
}

#[cfg(not(unix))]
fn git_helper_command() -> Result<OsString, Error> {
    let executable = std::env::current_exe()
        .map_err(|error| Error::local(format!("could not locate fjx: {error}")))?;
    let executable = executable.to_str().ok_or_else(|| {
        Error::local("could not encode the fjx path for the Git credential helper")
    })?;
    let mut command = String::from("!");
    shell_quote(executable, &mut command);
    command.push_str(" auth git-credential");
    Ok(OsString::from(command))
}

#[cfg(unix)]
fn shell_quote(value: &OsStr, output: &mut Vec<u8>) {
    output.push(b'\'');
    for byte in value.as_bytes() {
        if *byte == b'\'' {
            output.extend_from_slice(b"'\"'\"'");
        } else {
            output.push(*byte);
        }
    }
    output.push(b'\'');
}

#[cfg(not(unix))]
fn shell_quote(value: &str, output: &mut String) {
    output.push('\'');
    for character in value.chars() {
        if character == '\'' {
            output.push_str("'\"'\"'");
        } else {
            output.push(character);
        }
    }
    output.push('\'');
}

struct GitConfigSnapshot {
    key: OsString,
    values: Vec<OsString>,
}

impl GitConfigSnapshot {
    fn read(key: OsString) -> Result<Self, Error> {
        let output = run_git_config(&[
            OsStr::new("--global"),
            OsStr::new("--null"),
            OsStr::new("--get-all"),
            key.as_os_str(),
        ])
        .map_err(|cause| {
            Error::local(format!(
                "could not snapshot Git config key {}: {cause}",
                key.to_string_lossy()
            ))
        })?;
        let values = if output.status.success() {
            let values = output
                .stdout
                .strip_suffix(&[0])
                .unwrap_or(&output.stdout)
                .split(|byte| *byte == 0);
            #[cfg(unix)]
            let values = values
                .map(|value| OsString::from_vec(value.to_vec()))
                .collect();
            #[cfg(not(unix))]
            let values = values
                .map(git_config_value)
                .collect::<Result<Vec<_>, _>>()?;
            values
        } else if output.status.code() == Some(1) {
            Vec::new()
        } else {
            return Err(Error::local(format!(
                "could not snapshot Git config key {}: git config failed with status {}",
                key.to_string_lossy(),
                output.status
            )));
        };
        Ok(Self { key, values })
    }

    fn restore(&self) -> Result<(), String> {
        let output = run_git_config(&[
            OsStr::new("--global"),
            OsStr::new("--unset-all"),
            self.key.as_os_str(),
        ])?;
        if !output.status.success() && output.status.code() != Some(5) {
            return Err(format!("clear failed with status {}", output.status));
        }
        for value in &self.values {
            let output = run_git_config(&[
                OsStr::new("--global"),
                OsStr::new("--add"),
                self.key.as_os_str(),
                value.as_os_str(),
            ])?;
            if !output.status.success() {
                return Err(format!(
                    "value restore failed with status {}",
                    output.status
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(unix))]
fn git_config_value(value: &[u8]) -> Result<OsString, Error> {
    String::from_utf8(value.to_vec())
        .map(OsString::from)
        .map_err(|_| Error::local("Git config returned a value that is not valid UTF-8"))
}

fn update_git_config(snapshots: &[GitConfigSnapshot; 2], helper: &OsStr) -> Result<(), Error> {
    let helper_key = snapshots[0].key.as_os_str();
    let path_key = snapshots[1].key.as_os_str();
    let mutations = [
        (
            "replace credential helper",
            vec![
                OsStr::new("--global"),
                OsStr::new("--replace-all"),
                helper_key,
                OsStr::new(""),
            ],
        ),
        (
            "set HTTP path matching",
            vec![
                OsStr::new("--global"),
                OsStr::new("--replace-all"),
                path_key,
                OsStr::new("true"),
            ],
        ),
        (
            "add fjx credential helper",
            vec![
                OsStr::new("--global"),
                OsStr::new("--add"),
                helper_key,
                helper,
            ],
        ),
    ];
    for (stage, arguments) in mutations {
        let failure = match run_git_config(&arguments) {
            Ok(output) if output.status.success() => continue,
            Ok(output) => format!("git config failed with status {}", output.status),
            Err(cause) => cause,
        };
        return Err(rollback_git_config(snapshots, stage, &failure));
    }
    Ok(())
}

fn rollback_git_config(
    snapshots: &[GitConfigSnapshot; 2],
    mutation_stage: &str,
    mutation_cause: &str,
) -> Error {
    let failures: Vec<String> = snapshots
        .iter()
        .filter_map(|snapshot| {
            snapshot
                .restore()
                .err()
                .map(|cause| format!("restore {}: {cause}", snapshot.key.to_string_lossy()))
        })
        .collect();
    if failures.is_empty() {
        Error::local(format!(
            "Git config update failed during {mutation_stage}: {mutation_cause}; restored keys {} and {}",
            snapshots[0].key.to_string_lossy(),
            snapshots[1].key.to_string_lossy()
        ))
    } else {
        Error::local(format!(
            "Git config update failed during {mutation_stage}: {mutation_cause}; warning: Git config restoration is incomplete for keys {} and {}; {}",
            snapshots[0].key.to_string_lossy(),
            snapshots[1].key.to_string_lossy(),
            failures.join("; ")
        ))
    }
}

fn run_git_config(arguments: &[&OsStr]) -> Result<Output, String> {
    let output = ProcessCommand::new("git")
        .args(["config"])
        .args(arguments)
        .output()
        .map_err(|error| format!("could not run git config: {error}"))?;
    Ok(output)
}

#[derive(Default)]
struct GitCredentialRequest {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

impl GitCredentialRequest {
    fn read() -> Result<Self, Error> {
        let mut input = String::new();
        io::stdin()
            .lock()
            .take(16_385)
            .read_to_string(&mut input)
            .map_err(|error| {
                Error::local(format!("could not read Git credential input: {error}"))
            })?;
        if input.len() > 16_384 {
            return Err(Error::local("Git credential input is too large"));
        }
        let mut request = Self::default();
        for line in input.lines().take_while(|line| !line.is_empty()) {
            let Some((key, value)) = line.split_once('=') else {
                return Err(Error::local("invalid Git credential input"));
            };
            match key {
                "protocol" => set_credential_field(&mut request.protocol, value, key)?,
                "host" => set_credential_field(&mut request.host, value, key)?,
                "path" => set_credential_field(&mut request.path, value, key)?,
                _ => {}
            }
        }
        Ok(request)
    }
}

fn set_credential_field(field: &mut Option<String>, value: &str, name: &str) -> Result<(), Error> {
    if value.is_empty() || value.chars().any(char::is_control) || field.is_some() {
        return Err(Error::local(format!("invalid Git credential {name} field")));
    }
    *field = Some(value.to_owned());
    Ok(())
}

pub(crate) fn status(args: &Args) -> Result<Outcome, Error> {
    reject_read_flags(args)?;
    let context = Context::resolve(args, false, true)?;
    let token = context
        .token
        .ok_or_else(|| Error::context("token is required"))?;
    let source = context
        .token_source
        .ok_or_else(|| Error::context("token source is missing"))?;
    let client = Client::new(context.host.clone(), token);
    let user = authenticated_user(&client)?;
    if args.json {
        Outcome::json(&AuthRecord {
            kind: "auth",
            host: context.host.as_str(),
            user: &user.login,
            source: source.as_str(),
        })
    } else {
        Ok(Outcome::text(format!(
            "{}\t{}\t{}\n",
            context.host.as_str(),
            plain_field(&user.login),
            source.as_str()
        )))
    }
}

pub(crate) fn logout(args: &Args) -> Result<Outcome, Error> {
    require_persistence()?;
    reject_auth_flags(args)?;
    let mut context = Context::resolve(args, false, false)?;
    context.config.logout(&context.host);
    context.config.save(config_path(&context)?)?;
    result(args.json, "auth.logout")
}

fn config_path(context: &Context) -> Result<&std::path::Path, Error> {
    context
        .config_path
        .as_deref()
        .ok_or_else(|| Error::context("saved token path is unavailable"))
}

fn require_persistence() -> Result<(), Error> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err(config::persistence_unavailable())
    }
}

fn authenticated_user(client: &Client) -> Result<UserResponse, Error> {
    let response = client.send("GET", "user", None)?;
    serde_json::from_slice(&response.bytes)
        .map_err(|error| Error::data(format!("Forgejo returned an invalid user: {error}")))
}

fn read_token(with_token: bool, json: bool) -> Result<Token, Error> {
    let value = if with_token || !io::stdin().is_terminal() {
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|error| Error::local(format!("could not read token: {error}")))?;
        line.trim().to_owned()
    } else {
        let prompt = if json { "" } else { "Token: " };
        rpassword::prompt_password(prompt)
            .map(|value| value.trim().to_owned())
            .map_err(|error| Error::local(format!("could not read hidden token: {error}")))?
    };
    Token::parse(value)
}

fn result(json: bool, action: &'static str) -> Result<Outcome, Error> {
    if json {
        Outcome::json(&ResultRecord {
            kind: "result",
            action,
            ok: true,
            number: None,
            html_url: None,
        })
    } else {
        Ok(Outcome::text(format!("{action}\tok\n")))
    }
}

fn reject_auth_flags(args: &Args) -> Result<(), Error> {
    if args.dry_run {
        return Err(Error::usage("--dry-run is not valid for auth commands"));
    }
    if args.yes {
        return Err(Error::usage("--yes is not valid for auth commands"));
    }
    Ok(())
}

fn reject_read_flags(args: &Args) -> Result<(), Error> {
    if args.dry_run || args.yes {
        Err(Error::usage(
            "write safety flags are not valid for auth status",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    use super::UserResponse;
    #[cfg(unix)]
    use super::shell_quote;
    #[cfg(not(unix))]
    use super::{git_config_value, shell_quote};

    #[test]
    fn forgejo_15_user_fixture_decodes_with_unknown_fields() {
        let user: UserResponse = serde_json::from_str(include_str!(
            "../../tests/fixtures/forgejo-15.0.7/user.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(user.login, "dalton");
    }

    #[test]
    #[cfg(unix)]
    fn shell_quote_handles_quotes_and_non_utf8_paths() {
        let mut quoted = Vec::new();

        shell_quote(OsStr::from_bytes(b"/tmp/a'b\xff/fjx"), &mut quoted);

        assert_eq!(quoted, b"'/tmp/a'\"'\"'b\xff/fjx'");
    }

    #[test]
    #[cfg(not(unix))]
    fn non_unix_git_values_are_never_lossily_decoded() {
        let mut quoted = String::new();
        shell_quote("C:\\Program Files\\a'b\\fjx.exe", &mut quoted);

        assert_eq!(quoted, "'C:\\Program Files\\a'\"'\"'b\\fjx.exe'");
        assert!(git_config_value(b"valid-value").is_ok());
        assert!(git_config_value(b"invalid-\xff-value").is_err());
    }
}
