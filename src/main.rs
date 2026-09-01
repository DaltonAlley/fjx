mod args;
mod commands;
mod config;
mod context;
mod error;
mod host;
mod http;
mod output;
mod repo;

use std::io::Write;

use args::{Args, Command};
use error::Error;
use output::Outcome;

fn main() {
    let result = args::parse().and_then(|args| run(&args));
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
        Command::Help => Ok(Outcome::text(HELP)),
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

const HELP: &str = concat!(
    "fjx ",
    env!("CARGO_PKG_VERSION"),
    " - a small Forgejo client

Usage:
  fjx auth login [--with-token]
  fjx auth status
  fjx auth logout
  fjx auth setup-git
  fjx repo view
  fjx issue list [--state open|closed|all] [--page N | --all] [--limit N]
  fjx issue view NUMBER
  fjx issue create --title TEXT [--body TEXT | --body-file PATH|-]
  fjx issue comment NUMBER (--body TEXT | --body-file PATH|-)
  fjx issue close NUMBER
  fjx issue reopen NUMBER
  fjx pr list [--state open|closed|all] [--page N | --all] [--limit N]
  fjx pr view NUMBER
  fjx pr create --head REF --title TEXT [--base REF] [--body TEXT | --body-file PATH|-] [--draft]
  fjx pr diff NUMBER
  fjx pr checks NUMBER
  fjx pr comment NUMBER (--body TEXT | --body-file PATH|-)
  fjx pr review NUMBER --event approve|request-changes|comment [--body TEXT | --body-file PATH|-]
  fjx pr merge NUMBER [--style merge|rebase|rebase-merge|squash] [--title TEXT] [--message TEXT] [--delete-branch] --yes
  fjx pr close NUMBER
  fjx pr reopen NUMBER
  fjx run list [--page N | --all] [--limit N]
  fjx run view ID
  fjx run watch ID [--poll SECONDS] [--wait SECONDS]
  fjx release list [--page N | --all] [--limit N]
  fjx release view TAG
  fjx release create --tag TAG --title TEXT [--body TEXT | --body-file PATH|-] [--target REF] [--draft] [--prerelease]
  fjx release upload RELEASE_ID PATH [--name NAME]
  fjx label list [--page N | --all] [--limit N]
  fjx label create --name TEXT --color HEX [--description TEXT]
  fjx milestone list [--state open|closed|all] [--page N | --all] [--limit N]
  fjx milestone create --title TEXT [--description TEXT] [--due RFC3339]
  fjx branch list [--page N | --all] [--limit N]
  fjx branch delete NAME --yes
  fjx workflow dispatch FILE --ref REF [--field KEY=VALUE]...
  fjx api PATH [-X GET|POST|PUT|PATCH|DELETE] [--input PATH|-] [--paginate]

Common flags:
  --host URL       Forgejo HTTPS base URL (loopback HTTP is allowed)
  -R OWNER/REPO    Repository context
  --json           Emit one compact JSON value
  --dry-run        Print a Forgejo write without sending it
  --yes            Confirm pull-request merge or a destructive delete
  -h, --help       Show this help
  -V, --version    Show the version

Exit codes:
  0 success; 1 unsuccessful Forgejo work; 2 usage; 3 context/auth;
  4 network/TLS/timeout; 5 Forgejo HTTP error; 6 safety refusal;
  7 incompatible Forgejo data; 8 local file/VCS error; 130 interrupted
"
);
