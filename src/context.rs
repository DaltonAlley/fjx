use std::env;

use crate::args::Args;
use crate::config::{self, Config, Token};
use crate::error::Error;
use crate::host::Host;
use crate::repo::{self, Repo};

#[derive(Clone, Copy, Debug)]
pub(crate) enum TokenSource {
    Fjx,
    Forgejo,
    Config,
}

impl TokenSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Fjx => "FJX_TOKEN",
            Self::Forgejo => "FORGEJO_TOKEN",
            Self::Config => "config",
        }
    }
}

pub(crate) struct Context {
    pub(crate) host: Host,
    pub(crate) repo: Option<Repo>,
    pub(crate) token: Option<Token>,
    pub(crate) token_source: Option<TokenSource>,
    pub(crate) config: Config,
    pub(crate) config_path: Option<std::path::PathBuf>,
}

impl Context {
    pub(crate) fn resolve(args: &Args, need_repo: bool, need_token: bool) -> Result<Self, Error> {
        let config_path = config::path_for_context()?;
        #[cfg(unix)]
        let config = match config_path.as_deref() {
            Some(path) => Config::load(path)?,
            None => Config::empty(),
        };
        #[cfg(not(unix))]
        let config = Config::empty();
        let explicit_repo = args
            .repo
            .as_deref()
            .map(Repo::parse)
            .transpose()?
            .or_else(|| {
                env_nonempty("FJX_REPO")
                    .map(|value| Repo::parse(&value))
                    .transpose()
                    .ok()
                    .flatten()
            });
        let repo_env_invalid =
            args.repo.is_none() && env::var_os("FJX_REPO").is_some() && explicit_repo.is_none();
        if repo_env_invalid {
            return Err(Error::context("FJX_REPO is not a valid OWNER/REPO"));
        }
        let explicit_host = args
            .host
            .as_deref()
            .map(Host::parse)
            .transpose()?
            .or_else(|| {
                env_nonempty("FJX_HOST")
                    .map(|value| Host::parse(&value))
                    .transpose()
                    .ok()
                    .flatten()
            });
        let host_env_invalid =
            args.host.is_none() && env::var_os("FJX_HOST").is_some() && explicit_host.is_none();
        if host_env_invalid {
            return Err(Error::context("FJX_HOST is not a valid host URL"));
        }
        let remote = if (need_repo && explicit_repo.is_none()) || explicit_host.is_none() {
            repo::infer(&config, explicit_repo.as_ref())?
        } else {
            None
        };
        let repository = explicit_repo.or_else(|| remote.as_ref().map(|value| value.repo.clone()));
        if need_repo && repository.is_none() {
            return Err(Error::context(
                "repository context is required; use -R OWNER/REPO",
            ));
        }
        let selected_host = explicit_host.or_else(|| remote.map(|value| value.host));
        let host = match selected_host {
            Some(host) => host,
            None => config
                .default_host()?
                .ok_or_else(|| Error::context("host context is required; use --host URL"))?,
        };
        let (token, token_source) = if let Some(token) = token_from_env("FJX_TOKEN")? {
            (Some(token), Some(TokenSource::Fjx))
        } else if let Some(token) = token_from_env("FORGEJO_TOKEN")? {
            (Some(token), Some(TokenSource::Forgejo))
        } else if let Some(value) = config.token(&host) {
            (Some(value), Some(TokenSource::Config))
        } else {
            (None, None)
        };
        if need_token && token.is_none() {
            return Err(Error::context(format!(
                "no token is available for {}",
                host.as_str()
            )));
        }
        Ok(Self {
            host,
            repo: repository,
            token,
            token_source,
            config,
            config_path,
        })
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn token_from_env(name: &str) -> Result<Option<Token>, Error> {
    let Some(value) = env::var_os(name) else {
        return Ok(None);
    };
    let value = value
        .into_string()
        .map_err(|_| Error::context(format!("{name} must be valid UTF-8")))?;
    Token::parse(value)
        .map(Some)
        .map_err(|_| Error::context(format!("{name} is not a valid token")))
}
