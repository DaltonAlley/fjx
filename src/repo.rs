use std::collections::BTreeMap;
use std::process::{Command, Output};

use crate::config::Config;
use crate::error::Error;
use crate::host::Host;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Repo {
    owner: String,
    name: String,
}

impl Repo {
    pub(crate) fn parse(value: &str) -> Result<Self, Error> {
        let (owner, name) = value
            .split_once('/')
            .ok_or_else(|| Error::context("repository must have OWNER/REPO form"))?;
        if name.contains('/') || !valid_part(owner) || !valid_part(name) {
            return Err(Error::context(
                "repository owner and name may use letters, digits, dot, underscore, and dash",
            ));
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

fn valid_part(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteContext {
    pub(crate) host: Host,
    pub(crate) repo: Repo,
}

pub(crate) fn infer(
    config: &Config,
    matching_repo: Option<&Repo>,
) -> Result<Option<RemoteContext>, Error> {
    let remotes = inspect_remotes()?;
    infer_from_remotes(&remotes, config, matching_repo)
}

fn infer_from_remotes(
    remotes: &BTreeMap<String, String>,
    config: &Config,
    matching_repo: Option<&Repo>,
) -> Result<Option<RemoteContext>, Error> {
    if let Some(repo) = matching_repo {
        for name in ["origin", "upstream"] {
            if let Some(context) = remotes
                .get(name)
                .and_then(|url| parse_remote(url, config).ok())
                .filter(|context| &context.repo == repo)
            {
                return Ok(Some(context));
            }
        }
        return Ok(remotes
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "origin" | "upstream"))
            .find_map(|(_, url)| {
                parse_remote(url, config)
                    .ok()
                    .filter(|context| &context.repo == repo)
            }));
    }

    let mut first_error = None;
    for name in ["origin", "upstream"] {
        if let Some(url) = remotes.get(name) {
            match parse_remote(url, config) {
                Ok(context) => return Ok(Some(context)),
                Err(error) => first_error.get_or_insert(error),
            };
        }
    }
    if remotes.len() == 1
        && let Some(url) = remotes.values().next()
    {
        return parse_remote(url, config).map(Some);
    }
    first_error.map_or(Ok(None), Err)
}

fn inspect_remotes() -> Result<BTreeMap<String, String>, Error> {
    let jj = Command::new("jj")
        .args(["--ignore-working-copy", "git", "remote", "list"])
        .output();
    if let Ok(output) = jj
        && output.status.success()
    {
        let parsed = parse_jj_remotes(&String::from_utf8_lossy(&output.stdout));
        if !parsed.is_empty() {
            return Ok(parsed);
        }
    }
    let git = Command::new("git")
        .args(["remote", "-v"])
        .env("LC_ALL", "C")
        .output();
    match git {
        Ok(output) if output.status.success() => {
            Ok(parse_git_remotes(&String::from_utf8_lossy(&output.stdout)))
        }
        Ok(output) if git_reports_not_a_repository(&output) => Ok(BTreeMap::new()),
        Ok(_) | Err(_) => Err(Error::local("could not inspect repository remotes")),
    }
}

fn git_reports_not_a_repository(output: &Output) -> bool {
    output.status.code() == Some(128) && output.stderr.starts_with(b"fatal: not a git repository")
}

fn parse_jj_remotes(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, url)| (name.trim().to_owned(), url.trim().to_owned()))
        .filter(|(name, url)| !name.is_empty() && !url.is_empty())
        .collect()
}

fn parse_git_remotes(text: &str) -> BTreeMap<String, String> {
    let mut remotes = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(url), Some("(fetch)")) = (parts.next(), parts.next(), parts.next())
        {
            remotes.insert(name.to_owned(), url.to_owned());
        }
    }
    remotes
}

pub(crate) fn parse_remote(value: &str, config: &Config) -> Result<RemoteContext, Error> {
    if let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    {
        let scheme = value.split_once("://").map_or("https", |part| part.0);
        let (authority, path) = rest
            .split_once('/')
            .ok_or_else(|| Error::context("remote is missing repository path"))?;
        let (prefix, repo) = split_remote_path(path)?;
        let base = if prefix.is_empty() {
            format!("{scheme}://{authority}")
        } else {
            format!("{scheme}://{authority}/{prefix}")
        };
        return Ok(RemoteContext {
            host: Host::parse(&base)?,
            repo,
        });
    }
    if let Some(rest) = value.strip_prefix("ssh://") {
        let (authority, path) = rest
            .split_once('/')
            .ok_or_else(|| Error::context("remote is missing repository path"))?;
        let dns = parse_ssh_authority(authority, true)?;
        return ssh_context(dns, path, config);
    }
    if !value.contains("://")
        && let Some((authority, path)) = value.split_once(':')
        && !authority.contains('/')
    {
        let dns = parse_ssh_authority(authority, false)?;
        return ssh_context(dns, path, config);
    }
    Err(Error::context("unsupported repository remote URL"))
}

fn parse_ssh_authority(authority: &str, allow_port: bool) -> Result<&str, Error> {
    let mut parts = authority.split('@');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    let invalid_user = second.is_some()
        && (first
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            || first.contains(['/', ':', '\\', '?', '#']));
    if parts.next().is_some()
        || first.is_empty()
        || second.is_some_and(str::is_empty)
        || invalid_user
    {
        return Err(Error::context("invalid SSH remote authority"));
    }
    let host_and_port = second.unwrap_or(first);
    if !allow_port {
        return Ok(host_and_port);
    }
    let Some((host, port)) = host_and_port.rsplit_once(':') else {
        return Ok(host_and_port);
    };
    if host.is_empty() || port.parse::<u16>().is_err() {
        return Err(Error::context("invalid SSH remote port"));
    }
    Ok(host)
}

fn ssh_context(dns: &str, path: &str, config: &Config) -> Result<RemoteContext, Error> {
    if dns.is_empty()
        || !dns
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(Error::context("invalid SSH remote host"));
    }
    let (prefix, repo) = split_remote_path(path)?;
    let host = config.host_matching_dns(dns).map_or_else(
        || {
            let base = if prefix.is_empty() {
                format!("https://{dns}")
            } else {
                format!("https://{dns}/{prefix}")
            };
            Host::parse(&base)
        },
        Ok,
    )?;
    Ok(RemoteContext { host, repo })
}

fn split_remote_path(path: &str) -> Result<(String, Repo), Error> {
    if path.is_empty()
        || path.contains(['?', '#', '\\'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(Error::context("invalid repository remote path"));
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return Err(Error::context("remote is missing owner or repository"));
    }
    let raw_name = parts.pop().unwrap_or_default();
    let name = raw_name.strip_suffix(".git").unwrap_or(raw_name);
    let owner = parts.pop().unwrap_or_default();
    let repo = Repo::parse(&format!("{owner}/{name}"))?;
    Ok((parts.join("/"), repo))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Repo, infer_from_remotes, parse_remote};
    use crate::config::Config;

    #[test]
    fn validates_repo_parts() {
        assert!(Repo::parse("dalton/monolith").is_ok());
        assert!(Repo::parse("dalton.dev/monolith.rs").is_ok());
        assert!(Repo::parse(".dalton/monolith.").is_ok());
        assert!(Repo::parse("/repo").is_err());
        assert!(Repo::parse("./repo").is_err());
        assert!(Repo::parse("../repo").is_err());
        assert!(Repo::parse("owner/.").is_err());
        assert!(Repo::parse("owner/..").is_err());
        assert!(Repo::parse("owner/repo/extra").is_err());
        assert!(Repo::parse("owner/re po").is_err());
    }

    #[test]
    fn parses_contract_remote_forms() {
        let config = Config::empty();
        let cases = [
            (
                "https://forge.example/owner/repo.git",
                "https://forge.example",
            ),
            (
                "https://forge.example/git/owner/repo",
                "https://forge.example/git",
            ),
            (
                "ssh://git@forge.example:2222/git/owner/repo.git",
                "https://forge.example/git",
            ),
            (
                "git@forge.example:git/owner/repo.git",
                "https://forge.example/git",
            ),
        ];
        for (remote, expected_host) in cases {
            let parsed =
                parse_remote(remote, &config).unwrap_or_else(|error| panic!("{remote}: {error}"));
            assert_eq!(parsed.host.as_str(), expected_host);
            assert_eq!(
                format!("{}/{}", parsed.repo.owner(), parsed.repo.name()),
                "owner/repo"
            );
        }
    }

    #[test]
    fn explicit_repo_only_uses_a_remote_for_that_repo() {
        let config = Config::empty();
        let remotes = BTreeMap::from([
            (
                "origin".to_owned(),
                "https://wrong.example/other/repo".to_owned(),
            ),
            (
                "target".to_owned(),
                "https://right.example/dalton/monolith".to_owned(),
            ),
        ]);
        let repo = Repo::parse("dalton/monolith").unwrap_or_else(|error| panic!("{error}"));

        let inferred = infer_from_remotes(&remotes, &config, Some(&repo))
            .unwrap_or_else(|error| panic!("{error}"))
            .unwrap_or_else(|| panic!("missing matching remote"));

        assert_eq!(inferred.host.as_str(), "https://right.example");
        assert_eq!(inferred.repo, repo);
    }

    #[test]
    fn explicit_repo_does_not_use_an_unrelated_origin() {
        let config = Config::empty();
        let remotes = BTreeMap::from([(
            "origin".to_owned(),
            "https://wrong.example/other/repo".to_owned(),
        )]);
        let repo = Repo::parse("dalton/monolith").unwrap_or_else(|error| panic!("{error}"));

        let inferred = infer_from_remotes(&remotes, &config, Some(&repo))
            .unwrap_or_else(|error| panic!("{error}"));

        assert!(inferred.is_none());
    }

    #[test]
    fn an_invalid_origin_does_not_hide_a_valid_upstream() {
        let config = Config::empty();
        let remotes = BTreeMap::from([
            ("origin".to_owned(), "../local-repo".to_owned()),
            (
                "upstream".to_owned(),
                "https://forge.example/dalton/monolith".to_owned(),
            ),
        ]);

        let inferred = infer_from_remotes(&remotes, &config, None)
            .unwrap_or_else(|error| panic!("{error}"))
            .unwrap_or_else(|| panic!("missing upstream context"));

        assert_eq!(inferred.host.as_str(), "https://forge.example");
        assert_eq!(inferred.repo.owner(), "dalton");
        assert_eq!(inferred.repo.name(), "monolith");
    }

    #[test]
    fn forgejo_15_repository_fixture_has_required_shape() {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/forgejo-15.0.7/repository.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        for field in [
            "name",
            "full_name",
            "description",
            "private",
            "archived",
            "default_branch",
            "html_url",
        ] {
            assert!(value.get(field).is_some(), "missing {field}");
        }
    }

    #[test]
    fn rejects_local_and_git_remotes() {
        let config = Config::empty();
        for remote in [
            "../repo",
            "/tmp/repo",
            "file:///tmp/repo",
            "git://forge.example/o/r",
        ] {
            assert!(parse_remote(remote, &config).is_err(), "{remote}");
        }
    }

    #[test]
    fn rejects_malformed_remote_forms() {
        let config = Config::empty();
        for remote in [
            "ssh://git@forge.example:not-a-port/owner/repo",
            "ssh://git:secret@forge.example/owner/repo",
            "ssh://git user@forge.example/owner/repo",
            "ssh://git@@forge.example/owner/repo",
            "git@@forge.example:owner/repo",
            "https://forge.example//owner/repo",
            "https://forge.example/owner/repo/",
        ] {
            assert!(parse_remote(remote, &config).is_err(), "{remote}");
        }
    }
}
