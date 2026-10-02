mod args;
mod commands;
mod config;
mod context;
mod error;
mod help;
mod host;
mod http;
mod output;
mod repo;

use std::io::Write;

use args::{Args, Command};
use error::Error;
use output::Outcome;

fn main() {
    let result = args::parse().and_then(|args| execute(&args));
    match result {
        Ok(outcome) => {
            if let Err(error) = std::io::stdout().lock().write_all(&outcome.bytes) {
                eprintln!("fjx: could not write output: {error}");
                std::process::exit(8);
            }
            if outcome.code != 0 {
                std::process::exit(i32::from(outcome.code));
            }
        }
        Err(error) => fail(&error),
    }
}

fn execute(args: &Args) -> Result<Outcome, Error> {
    if matches!(args.command, Command::Help { .. }) {
        return run(args);
    }
    // A misspelled projection must not be discovered after a mutation has run.
    let fields = if args.dry_run {
        Some(["kind", "method", "url", "body"].as_slice())
    } else {
        help::output_fields(&args.command)
    };
    output::validate_fields(&args.fields, fields)?;
    let outcome = run(args)?;
    output::project(outcome, &args.fields)
}

fn run(args: &Args) -> Result<Outcome, Error> {
    match &args.command {
        Command::AuthLogin { with_token } => commands::auth::login(args, *with_token),
        Command::AuthStatus => commands::auth::status(args),
        Command::AuthLogout => commands::auth::logout(args),
        Command::AuthSetupGit => commands::auth::setup_git(args),
        Command::AuthGitCredential { operation } => {
            commands::auth::git_credential(args, *operation)
        }
        Command::RepoView => commands::repo::view(args),
        Command::Issue(command) => commands::issue::run(args, command),
        Command::Pull(command) => commands::pull::run(args, command),
        Command::Run(command) => commands::run::run(args, command),
        Command::Release(command) => commands::release::run(args, command),
        Command::Label(command) => commands::label::run(args, command),
        Command::Milestone(command) => commands::milestone::run(args, command),
        Command::Branch(command) => commands::branch::run(args, command),
        Command::Workflow(command) => commands::workflow::run(args, command),
        Command::Api(api) => commands::api::run(args, api),
        Command::Help { path } => help::render(path),
        Command::Schema { path } => help::schema(path),
        Command::Version => Ok(Outcome::text(concat!(
            "fjx ",
            env!("CARGO_PKG_VERSION"),
            "\n"
        ))),
    }
}

fn fail(error: &Error) -> ! {
    eprintln!("fjx: {error}");
    std::process::exit(i32::from(error.code()));
}
