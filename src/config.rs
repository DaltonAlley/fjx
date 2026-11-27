use std::collections::BTreeMap;
use std::env;
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::Read;
#[cfg(any(unix, windows))]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
mod acl;
#[cfg(windows)]
mod windows;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::host::Host;

#[derive(Clone)]
pub(crate) struct Token(String, bool);

impl Token {
    pub(crate) fn parse(value: String) -> Result<Self, Error> {
        Self::validate(&value)?;
        Ok(Self(value, false))
    }

    pub(crate) fn oauth(value: String) -> Result<Self, Error> {
        Self::validate(&value)?;
        Ok(Self(value, true))
    }

    pub(crate) const fn scheme(&self) -> &'static str {
        if self.1 { "Bearer" } else { "token" }
    }

    fn validate(value: &str) -> Result<(), Error> {
        if value.trim().is_empty() {
            return Err(Error::context("token must not be empty"));
        }
        if !value
            .as_bytes()
            .iter()
            .all(|byte| matches!(byte, b' '..=b'~'))
        {
            return Err(Error::context(
                "token must contain only printable ASCII bytes",
            ));
        }
        Ok(())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Config {
    version: u8,
    default_host: Option<String>,
    hosts: BTreeMap<String, HostEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct HostEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    oauth: Option<OAuthCredentials>,
    /// SSH host names in repository remotes that stand for this web host,
    /// such as a `Host` alias from the user's SSH configuration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ssh_aliases: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OAuthCredentials {
    pub(crate) client_id: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: u64,
}

pub(crate) struct GitCredential {
    pub(crate) host: Host,
    pub(crate) user: String,
    pub(crate) token: Token,
}

impl Config {
    pub(crate) fn empty() -> Self {
        Self {
            version: 1,
            default_host: None,
            hosts: BTreeMap::new(),
        }
    }

    pub(crate) fn load(path: &Path) -> Result<Self, Error> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(error) => {
                return Err(Error::context(format!(
                    "could not inspect token file: {error}"
                )));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::context(
                "token file must be a regular file, not a link",
            ));
        }
        validate_token_file_security(&metadata)?;
        #[cfg(not(windows))]
        let mut file = fs::File::open(path)
            .map_err(|error| Error::context(format!("could not read token file: {error}")))?;
        #[cfg(windows)]
        let mut file = {
            use std::os::windows::fs::OpenOptionsExt;
            // Inspect the final component itself and prevent replacement while
            // checking and reading this handle.
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1) // FILE_SHARE_READ
                .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
                .open(path)
                .map_err(|error| Error::context(format!("could not read token file: {error}")))?
        };
        #[cfg(windows)]
        {
            let metadata = file.metadata().map_err(|error| {
                Error::context(format!("could not inspect token file: {error}"))
            })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::context(
                    "token file must be a regular file, not a link",
                ));
            }
            validate_token_file_security(&metadata)?;
            validate_open_token_file(&file)?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| Error::context(format!("could not read token file: {error}")))?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|error| Error::context(format!("invalid token file: {error}")))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err(Error::context("unsupported token file version"));
        }
        if let Some(value) = &self.default_host {
            let host = Host::parse(value)?;
            if host.as_str() != value {
                return Err(Error::context(
                    "default host in token file is not canonical",
                ));
            }
        }
        for (value, entry) in &self.hosts {
            let host = Host::parse(value)?;
            if host.as_str() != value {
                return Err(Error::context("invalid host entry in token file"));
            }
            Token::validate(&entry.token)
                .map_err(|_| Error::context("invalid token in token file"))?;
            if let Some(oauth) = &entry.oauth {
                Token::validate(&oauth.refresh_token)
                    .map_err(|_| Error::context("invalid OAuth credentials in token file"))?;
                validate_client_id(&oauth.client_id)?;
            }
            if entry.user.as_deref().is_some_and(|user| !valid_user(user)) {
                return Err(Error::context("invalid user in token file"));
            }
            for alias in &entry.ssh_aliases {
                // One alias must select one saved login, never a choice of tokens.
                let owners = self
                    .hosts
                    .values()
                    .flat_map(|other| &other.ssh_aliases)
                    .filter(|other| *other == alias)
                    .count();
                if !valid_ssh_alias(alias) || owners != 1 {
                    return Err(Error::context("invalid SSH alias in token file"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn default_host(&self) -> Result<Option<Host>, Error> {
        match &self.default_host {
            None => Ok(None),
            Some(value) if self.hosts.contains_key(value) => Host::parse(value).map(Some),
            Some(_) => Err(Error::context("saved default host has no token")),
        }
    }

    pub(crate) fn token(&self, host: &Host) -> Option<Token> {
        self.hosts
            .get(host.as_str())
            .map(|entry| Token(entry.token.clone(), entry.oauth.is_some()))
    }

    pub(crate) fn oauth(&self, host: &Host) -> Option<&OAuthCredentials> {
        self.hosts
            .get(host.as_str())
            .and_then(|entry| entry.oauth.as_ref())
    }

    pub(crate) fn save_oauth(
        &mut self,
        host: &Host,
        token: Token,
        oauth: OAuthCredentials,
    ) -> Result<(), Error> {
        let entry = self
            .hosts
            .get_mut(host.as_str())
            .ok_or_else(|| Error::context("no saved login is available for this host"))?;
        entry.token = token.into_string();
        entry.oauth = Some(oauth);
        Ok(())
    }

    pub(crate) fn has_git_login(&self, host: &Host) -> bool {
        self.hosts
            .get(host.as_str())
            .is_some_and(|entry| entry.user.is_some())
    }

    pub(crate) fn save_user(&mut self, host: &Host, user: String) -> Result<(), Error> {
        if !valid_user(&user) {
            return Err(Error::data("Forgejo returned an invalid user name"));
        }
        let entry = self
            .hosts
            .get_mut(host.as_str())
            .ok_or_else(|| Error::context("no saved token is available for this host"))?;
        entry.user = Some(user);
        Ok(())
    }

    fn host_matching_dns(&self, dns: &str) -> Option<Host> {
        self.hosts
            .keys()
            .filter_map(|value| Host::parse(value).ok())
            .find(|host| host.dns_host().eq_ignore_ascii_case(dns))
    }

    /// The saved login an SSH remote stands for: the host with that DNS name,
    /// otherwise the host that records the name as an SSH alias. Only saved
    /// logins are candidates, so a remote cannot direct a token to a new host.
    pub(crate) fn host_for_ssh(&self, ssh_host: &str) -> Option<Host> {
        self.host_matching_dns(ssh_host).or_else(|| {
            let alias = ssh_host.to_ascii_lowercase();
            self.hosts
                .iter()
                .find(|(_, entry)| entry.ssh_aliases.contains(&alias))
                .and_then(|(value, _)| Host::parse(value).ok())
        })
    }

    /// Every saved login in host order, without its token: host, user (absent
    /// in token files from before `auth setup-git` saved it), whether it is
    /// the default host, and its SSH aliases.
    pub(crate) fn logins(&self) -> impl Iterator<Item = (&str, Option<&str>, bool, &[String])> {
        self.hosts.iter().map(|(host, entry)| {
            (
                host.as_str(),
                entry.user.as_deref(),
                self.default_host.as_deref() == Some(host.as_str()),
                entry.ssh_aliases.as_slice(),
            )
        })
    }

    pub(crate) fn ssh_aliases(&self, host: &Host) -> &[String] {
        self.hosts
            .get(host.as_str())
            .map_or(&[], |entry| entry.ssh_aliases.as_slice())
    }

    /// Record an alias on a saved login. Returns the stored (lowercase) name.
    pub(crate) fn add_ssh_alias(&mut self, host: &Host, alias: &str) -> Result<String, Error> {
        let alias = alias.to_ascii_lowercase();
        if !valid_ssh_alias(&alias) {
            return Err(Error::usage(
                "SSH alias must be a host name of letters, digits, '.' or '-' (up to 253 bytes)",
            ));
        }
        if !self.hosts.contains_key(host.as_str()) {
            return Err(Error::context(format!(
                "no saved login for {}; run fjx auth login --host {} first",
                host.as_str(),
                host.as_str()
            )));
        }
        if let Some(named) = self.host_matching_dns(&alias) {
            return Err(Error::usage(format!(
                "{alias} is the DNS name of saved host {}; its SSH remotes already resolve there",
                named.as_str()
            )));
        }
        if let Some((other, _)) = self
            .hosts
            .iter()
            .find(|(value, entry)| *value != host.as_str() && entry.ssh_aliases.contains(&alias))
        {
            return Err(Error::usage(format!(
                "SSH alias {alias} already names {other}; run fjx auth remove-ssh-alias {alias} --host {other} first"
            )));
        }
        if let Some(entry) = self.hosts.get_mut(host.as_str())
            && !entry.ssh_aliases.contains(&alias)
        {
            entry.ssh_aliases.push(alias.clone());
            entry.ssh_aliases.sort_unstable();
        }
        Ok(alias)
    }

    pub(crate) fn remove_ssh_alias(&mut self, host: &Host, alias: &str) -> Result<String, Error> {
        let alias = alias.to_ascii_lowercase();
        let entry = self
            .hosts
            .get_mut(host.as_str())
            .filter(|entry| entry.ssh_aliases.contains(&alias))
            .ok_or_else(|| {
                Error::usage(format!(
                    "no SSH alias {alias:?} is recorded for {}",
                    host.as_str()
                ))
            })?;
        entry.ssh_aliases.retain(|existing| *existing != alias);
        Ok(alias)
    }

    pub(crate) fn login(&mut self, host: &Host, user: String, token: Token) -> Result<(), Error> {
        if !valid_user(&user) {
            return Err(Error::data("Forgejo returned an invalid user name"));
        }
        // A new token for the same host keeps the aliases that point at it.
        let ssh_aliases = self
            .hosts
            .remove(host.as_str())
            .map(|entry| entry.ssh_aliases)
            .unwrap_or_default();
        self.hosts.insert(
            host.as_str().to_owned(),
            HostEntry {
                user: Some(user),
                token: token.into_string(),
                oauth: None,
                ssh_aliases,
            },
        );
        self.default_host = Some(host.as_str().to_owned());
        Ok(())
    }

    pub(crate) fn git_credential(
        &self,
        protocol: &str,
        authority: &str,
        path: &str,
    ) -> Option<GitCredential> {
        self.hosts
            .iter()
            .filter_map(|(value, entry)| {
                let host = Host::parse(value).ok()?;
                if !host.matches_git_credential(protocol, authority, path) {
                    return None;
                }
                Some((
                    value.len(),
                    GitCredential {
                        host,
                        user: entry.user.clone()?,
                        token: Token(entry.token.clone(), entry.oauth.is_some()),
                    },
                ))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, credential)| credential)
    }

    pub(crate) fn logout(&mut self, host: &Host) -> bool {
        let removed = self.hosts.remove(host.as_str()).is_some();
        if self.default_host.as_deref() == Some(host.as_str()) {
            self.default_host = None;
        }
        removed
    }

    #[cfg(unix)]
    pub(crate) fn save(&self, path: &Path) -> Result<(), Error> {
        self.validate()?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::local("token file path has no parent directory"))?;
        let directory = prepare_directory(parent)?;
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() =>
            {
                return Err(Error::context(
                    "token file must be a regular file, not a link",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::local(format!(
                    "could not inspect token file: {error}"
                )));
            }
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| Error::local(format!("could not encode token file: {error}")))?;
        let mut temporary = None;
        for attempt in 0..100_u8 {
            let candidate =
                directory.join(format!(".fjx-hosts.{}.{}.tmp", std::process::id(), attempt));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&candidate)
            {
                Ok(mut file) => {
                    if let Err(error) = file.set_permissions(fs::Permissions::from_mode(0o600)) {
                        let _ = fs::remove_file(&candidate);
                        return Err(Error::local(format!(
                            "could not secure token file: {error}"
                        )));
                    }
                    let metadata = match file.metadata() {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            let _ = fs::remove_file(&candidate);
                            return Err(Error::local(format!(
                                "could not inspect token file: {error}"
                            )));
                        }
                    };
                    if !metadata.is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
                        let _ = fs::remove_file(&candidate);
                        return Err(Error::local(
                            "temporary token file must be a regular file with permissions 0600",
                        ));
                    }
                    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                        let _ = fs::remove_file(&candidate);
                        return Err(Error::local(format!("could not write token file: {error}")));
                    }
                    temporary = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(Error::local(format!(
                        "could not create token file: {error}"
                    )));
                }
            }
        }
        let temporary = temporary.ok_or_else(|| Error::local("could not allocate token file"))?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(Error::local(format!(
                "could not replace token file: {error}"
            )));
        }
        fs::File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| Error::local(format!("could not sync token directory: {error}")))?;
        Ok(())
    }

    #[cfg(windows)]
    pub(crate) fn save(&self, path: &Path) -> Result<(), Error> {
        self.validate()?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::local("token file path has no parent directory"))?;
        let directory = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            fs::create_dir_all(parent).map_err(|error| {
                Error::local(format!("could not create token directory: {error}"))
            })?;
            parent
        };
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() =>
            {
                return Err(Error::context(
                    "token file must be a regular file, not a link",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::local(format!(
                    "could not inspect token file: {error}"
                )));
            }
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| Error::local(format!("could not encode token file: {error}")))?;
        let mut temporary = None;
        for attempt in 0..100_u8 {
            let candidate =
                directory.join(format!(".fjx-hosts.{}.{}.tmp", std::process::id(), attempt));
            let mut file = match windows::create_private(&candidate) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(Error::local(format!(
                        "could not create private token file: {error}"
                    )));
                }
            };
            let written = validate_open_token_file(&file)
                .map_err(|error| Error::local(error.to_string()))
                .and_then(|()| {
                    file.write_all(&bytes)
                        .and_then(|()| file.sync_all())
                        .map_err(|error| {
                            Error::local(format!("could not write token file: {error}"))
                        })
                });
            // The file is not shareable, so close it before removing or renaming it.
            drop(file);
            if let Err(error) = written {
                let _ = fs::remove_file(&candidate);
                return Err(error);
            }
            temporary = Some(candidate);
            break;
        }
        let temporary = temporary.ok_or_else(|| Error::local("could not allocate token file"))?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(Error::local(format!(
                "could not replace token file: {error}"
            )));
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    pub(crate) fn save(&self, _path: &Path) -> Result<(), Error> {
        self.validate()?;
        Err(persistence_unavailable())
    }
}

#[cfg(unix)]
fn validate_token_file_security(metadata: &fs::Metadata) -> Result<(), Error> {
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(Error::context("token file permissions must be 0600"));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_token_file_security(metadata: &fs::Metadata) -> Result<(), Error> {
    use std::os::windows::fs::MetadataExt;

    if metadata.file_attributes() & 0x0400 != 0 {
        return Err(Error::context("token file must not be a reparse point"));
    }
    // The DACL is checked on the open handle; see `validate_open_token_file`.
    Ok(())
}

#[cfg(windows)]
fn validate_open_token_file(file: &fs::File) -> Result<(), Error> {
    match windows::check_private(file) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(reason)) => Err(Error::context(format!(
            "token file must be private to the current user, but {reason}"
        ))),
        Err(error) => Err(Error::context(format!(
            "could not verify token file access: {error}"
        ))),
    }
}

#[cfg(not(any(unix, windows)))]
fn validate_token_file_security(_metadata: &fs::Metadata) -> Result<(), Error> {
    Err(persistence_unavailable())
}

pub(crate) fn persistence_unavailable() -> Error {
    Error::context(
        "saved tokens are unavailable because private file access cannot be verified on this platform; use FJX_TOKEN instead",
    )
}

fn valid_user(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

/// Stored aliases are lowercase and use the characters `repo` accepts in an
/// SSH remote host.
fn valid_ssh_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
}

pub(crate) fn validate_client_id(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Error::context(
            "OAuth client ID must contain only letters, digits, '.', '_' or '-' (up to 256 bytes)",
        ));
    }
    Ok(())
}

/// A stable sidecar lock survives atomic replacement of the credential file.
/// It contains no credentials; dropping the handle releases the OS lock, even
/// when a process fails during token exchange. Never unlink a live lock file.
pub(crate) fn auth_lock(path: &Path) -> Result<fs::File, Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::local("token file path has no parent directory"))?;
    #[cfg(unix)]
    let parent = prepare_directory(parent)?;
    #[cfg(not(unix))]
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        fs::create_dir_all(parent).map_err(|_| Error::local("could not create token directory"))?;
        parent
    };
    let lock_path = parent.join(".fjx-auth.lock");
    #[cfg(windows)]
    match windows::create_private(&lock_path) {
        Ok(file) => drop(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(Error::local("could not create private authentication lock")),
    }
    if let Ok(metadata) = fs::symlink_metadata(&lock_path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(Error::local("authentication lock must be a regular file"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x0020_0000);
    }
    #[cfg(unix)]
    let file = match options.create_new(true).mode(0o600).open(&lock_path) {
        Ok(file) => {
            // A restrictive umask can remove owner access even from mode 0600.
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Error::local("could not secure authentication lock"))?;
            file
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => options
            .create_new(false)
            .open(&lock_path)
            .map_err(|_| Error::local("could not open authentication lock"))?,
        Err(_) => return Err(Error::local("could not create authentication lock")),
    };
    #[cfg(not(unix))]
    let file = options
        .open(lock_path)
        .map_err(|_| Error::local("could not open authentication lock"))?;
    let metadata = file
        .metadata()
        .map_err(|_| Error::local("could not inspect authentication lock"))?;
    if !metadata.is_file() {
        return Err(Error::local("authentication lock must be a regular file"));
    }
    validate_token_file_security(&metadata)?;
    #[cfg(windows)]
    validate_open_token_file(&file)?;
    if crate::http::has_request_control() {
        crate::http::checkpoint()?;
        file.try_lock().map_err(|_| {
            Error::context(
                "saved authentication is busy; retry when the other login or refresh finishes",
            )
        })?;
    } else {
        file.lock()
            .map_err(|_| Error::local("could not lock saved authentication"))?;
    }
    Ok(file)
}

#[cfg(unix)]
fn prepare_directory(parent: &Path) -> Result<&Path, Error> {
    if parent.as_os_str().is_empty() {
        return Ok(Path::new("."));
    }
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => return Ok(parent),
        Ok(_) => return Err(Error::local("token directory path is not a directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Error::local(format!(
                "could not inspect token directory: {error}"
            )));
        }
    }

    if let Some(ancestor) = parent.parent()
        && !ancestor.as_os_str().is_empty()
    {
        fs::create_dir_all(ancestor)
            .map_err(|error| Error::local(format!("could not create token directory: {error}")))?;
    }

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(parent) {
        Ok(()) => {
            let created = fs::symlink_metadata(parent).map_err(|error| {
                Error::local(format!("could not inspect token directory: {error}"))
            })?;
            if !created.file_type().is_dir() {
                return Err(Error::local("created token directory is not a directory"));
            }
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|error| {
                Error::local(format!("could not secure token directory: {error}"))
            })?;
            let secured = fs::symlink_metadata(parent).map_err(|error| {
                Error::local(format!("could not inspect token directory: {error}"))
            })?;
            if !secured.file_type().is_dir()
                || secured.dev() != created.dev()
                || secured.ino() != created.ino()
                || secured.permissions().mode() & 0o777 != 0o700
            {
                return Err(Error::local(
                    "created token directory must have permissions 0700",
                ));
            }
            Ok(parent)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::metadata(parent).map_err(|error| {
                Error::local(format!("could not inspect token directory: {error}"))
            })?;
            if !metadata.is_dir() {
                return Err(Error::local("token directory path is not a directory"));
            }
            Ok(parent)
        }
        Err(error) => Err(Error::local(format!(
            "could not create token directory: {error}"
        ))),
    }
}

pub(crate) fn path() -> Result<PathBuf, Error> {
    path_for_context()?.ok_or_else(|| {
        Error::context("APPDATA and USERPROFILE are not set; set FJX_CONFIG explicitly")
    })
}

pub(crate) fn path_for_context() -> Result<Option<PathBuf>, Error> {
    if let Some(value) = env::var_os("FJX_CONFIG") {
        if value.is_empty() {
            return Err(Error::context("FJX_CONFIG must not be empty"));
        }
        return Ok(Some(PathBuf::from(value)));
    }
    #[cfg(unix)]
    {
        unix_default_path().map(Some)
    }
    #[cfg(windows)]
    {
        windows_default_path(env::var_os("APPDATA"), env::var_os("USERPROFILE"))
    }
}

#[cfg(unix)]
fn unix_default_path() -> Result<PathBuf, Error> {
    if let Some(value) = env::var_os("XDG_CONFIG_HOME") {
        if value.is_empty() {
            return Err(Error::context("XDG_CONFIG_HOME must not be empty"));
        }
        return Ok(PathBuf::from(value).join("fjx/hosts.json"));
    }
    let home = env::var_os("HOME").ok_or_else(|| Error::context("HOME is not set"))?;
    if home.is_empty() {
        return Err(Error::context("HOME must not be empty"));
    }
    Ok(PathBuf::from(home).join(".config/fjx/hosts.json"))
}

#[cfg(windows)]
fn windows_default_path(
    app_data: Option<OsString>,
    user_profile: Option<OsString>,
) -> Result<Option<PathBuf>, Error> {
    if let Some(value) = app_data {
        if value.is_empty() {
            return Err(Error::context("APPDATA must not be empty"));
        }
        return Ok(Some(PathBuf::from(value).join("fjx/hosts.json")));
    }
    if let Some(value) = user_profile {
        if value.is_empty() {
            return Err(Error::context("USERPROFILE must not be empty"));
        }
        return Ok(Some(
            PathBuf::from(value).join("AppData/Roaming/fjx/hosts.json"),
        ));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[cfg(any(unix, windows))]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(not(any(unix, windows)))]
    use std::path::Path;
    #[cfg(windows)]
    use std::{ffi::OsString, path::PathBuf};

    #[cfg(windows)]
    use super::windows_default_path;
    use super::{Config, Token};
    use crate::host::Host;

    #[test]
    fn login_and_logout_preserve_other_hosts() {
        let first = Host::parse("https://one.example").unwrap_or_else(|error| panic!("{error}"));
        let second = Host::parse("https://two.example").unwrap_or_else(|error| panic!("{error}"));
        let mut config = Config::empty();
        config
            .login(
                &first,
                "first-user".to_owned(),
                Token::parse("first".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        config
            .login(
                &second,
                "second-user".to_owned(),
                Token::parse("second".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(config.logout(&second));
        assert!(
            config
                .default_host()
                .unwrap_or_else(|error| panic!("{error}"))
                .is_none()
        );
        assert_eq!(
            config.token(&first).map(|token| token.0),
            Some("first".to_owned())
        );
    }

    #[test]
    #[cfg(unix)]
    fn mcp_refresh_does_not_wait_for_another_authentication_lock() {
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        use std::time::{Duration, Instant};
        let root = std::env::temp_dir().join(format!("fjx-mcp-auth-lock-{}", std::process::id()));
        let path = root.join("hosts.json");
        let lock = super::auth_lock(&path).unwrap_or_else(|e| panic!("{e}"));
        let control = crate::http::RequestControl {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + Duration::from_secs(30),
            secrets: Arc::new(Mutex::new(Vec::new())),
        };
        crate::http::controlled(control, || {
            let error = super::auth_lock(&path)
                .err()
                .unwrap_or_else(|| panic!("lock must be busy"));
            assert_eq!(error.code(), 3);
        });
        drop(lock);
        let acquired = super::auth_lock(&path).unwrap_or_else(|e| panic!("{e}"));
        drop(acquired);
        fs::remove_dir_all(root).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn login_uses_the_canonical_dns_key() {
        let host =
            Host::parse("https://FORGE.Example/base").unwrap_or_else(|error| panic!("{error}"));
        let mut config = Config::empty();

        config
            .login(
                &host,
                "user".to_owned(),
                Token::parse("secret".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            config.default_host.as_deref(),
            Some("https://forge.example/base")
        );
        assert!(config.hosts.contains_key("https://forge.example/base"));
        assert!(!config.hosts.contains_key("https://FORGE.Example/base"));
    }

    #[test]
    #[cfg(unix)]
    fn saves_mode_0600_and_round_trips() {
        let root = std::env::temp_dir().join(format!("fjx-config-test-{}", std::process::id()));
        let path = root.join("hosts.json");
        let _ = fs::remove_dir_all(&root);
        let host = Host::parse("https://forge.example").unwrap_or_else(|error| panic!("{error}"));
        let mut config = Config::empty();
        config
            .login(
                &host,
                "user".to_owned(),
                Token::parse("secret".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        config.save(&path).unwrap_or_else(|error| panic!("{error}"));
        let mode = fs::metadata(&path)
            .unwrap_or_else(|error| panic!("{error}"))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let loaded = Config::load(&path).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            loaded.token(&host).map(|token| token.0),
            Some("secret".to_owned())
        );
        fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    #[cfg(windows)]
    fn saves_an_owner_only_file_and_rejects_shared_access() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = std::env::temp_dir().join(format!("fjx-config-test-{}", std::process::id()));
        let path = root.join("nested/hosts.json");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap_or_else(|error| panic!("{error}"));
        // Even a shared parent must not contribute inherited access to the file.
        let status = std::process::Command::new("icacls")
            .arg(root.join("nested"))
            .args(["/grant", "*S-1-1-0:(OI)(CI)(R)"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success());
        let host = Host::parse("https://forge.example").unwrap_or_else(|error| panic!("{error}"));
        let mut config = Config::empty();
        config
            .login(
                &host,
                "user".to_owned(),
                Token::parse("secret".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        config.save(&path).unwrap_or_else(|error| panic!("{error}"));
        // Saving again replaces the file in place.
        config.save(&path).unwrap_or_else(|error| panic!("{error}"));
        let loaded = Config::load(&path).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            loaded.token(&host).map(|token| token.0),
            Some("secret".to_owned())
        );
        let leftovers = std::fs::read_dir(root.join("nested"))
            .unwrap_or_else(|error| panic!("{error}"))
            .count();
        assert_eq!(leftovers, 1);

        // Grant Everyone (S-1-1-0) read access, as a careless copy or edit might.
        let status = std::process::Command::new("icacls")
            .arg(&path)
            .args(["/grant", "*S-1-1-0:R"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success());
        let error = Config::load(&path)
            .err()
            .unwrap_or_else(|| panic!("a shared token file must be rejected"));
        assert_eq!(error.code(), 3);
        assert!(
            error
                .to_string()
                .contains("token file must be private to the current user"),
            "{error}"
        );
        // A failed replacement leaves the old token intact and removes the temp file.
        let before = fs::read(&path).unwrap_or_else(|error| panic!("{error}"));
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(config.save(&path).is_err());
        assert_eq!(
            fs::read_dir(root.join("nested"))
                .unwrap_or_else(|error| panic!("{error}"))
                .count(),
            1
        );
        drop(file);
        assert_eq!(
            fs::read(&path).unwrap_or_else(|error| panic!("{error}")),
            before
        );
        std::fs::remove_dir_all(root).unwrap_or_else(|error| panic!("{error}"));
    }

    #[test]
    #[cfg(not(any(unix, windows)))]
    fn refuses_to_persist_tokens_without_verifiable_private_access() {
        let error = Config::empty()
            .save(Path::new("hosts.json"))
            .expect_err("non-Unix token persistence must fail closed");

        assert!(error.to_string().contains("use FJX_TOKEN instead"));
    }

    #[test]
    #[cfg(windows)]
    fn windows_config_path_prefers_app_data_and_has_a_profile_fallback() {
        assert_eq!(
            windows_default_path(
                Some(OsString::from(r"C:\Users\agent\AppData\Roaming")),
                Some(OsString::from(r"C:\Users\ignored")),
            )
            .unwrap_or_else(|error| panic!("{error}")),
            Some(PathBuf::from(
                r"C:\Users\agent\AppData\Roaming\fjx/hosts.json"
            ))
        );
        assert_eq!(
            windows_default_path(None, Some(OsString::from(r"C:\Users\agent")))
                .unwrap_or_else(|error| panic!("{error}")),
            Some(PathBuf::from(
                r"C:\Users\agent\AppData/Roaming/fjx/hosts.json"
            ))
        );
        assert_eq!(
            windows_default_path(None, None).unwrap_or_else(|error| panic!("{error}")),
            None
        );
    }

    fn logged_in(hosts: &[&str]) -> Config {
        let mut config = Config::empty();
        for value in hosts {
            let host = Host::parse(value).unwrap_or_else(|error| panic!("{error}"));
            config
                .login(
                    &host,
                    "user".to_owned(),
                    Token::parse(format!("token-for-{value}"))
                        .unwrap_or_else(|error| panic!("{error}")),
                )
                .unwrap_or_else(|error| panic!("{error}"));
        }
        config
    }

    #[test]
    fn ssh_aliases_resolve_only_to_the_saved_login_that_records_them() {
        let gitea = Host::parse("https://code.example").unwrap_or_else(|error| panic!("{error}"));
        let forgejo =
            Host::parse("https://forge.example/base").unwrap_or_else(|error| panic!("{error}"));
        let mut config = logged_in(&["https://code.example", "https://forge.example/base"]);
        assert!(config.host_for_ssh("forge-ssh").is_none());
        assert_eq!(
            config
                .add_ssh_alias(&gitea, "Forge-SSH")
                .unwrap_or_else(|error| panic!("{error}")),
            "forge-ssh"
        );
        // Recording the same alias again changes nothing.
        config
            .add_ssh_alias(&gitea, "forge-ssh")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(config.ssh_aliases(&gitea), ["forge-ssh"]);
        assert_eq!(config.host_for_ssh("FORGE-SSH"), Some(gitea.clone()));
        assert_eq!(config.host_for_ssh("forge.example"), Some(forgejo.clone()));
        assert!(config.host_for_ssh("other").is_none());

        // One alias never selects between two tokens.
        assert_eq!(
            config
                .add_ssh_alias(&forgejo, "forge-ssh")
                .err()
                .map(|error| error.code()),
            Some(2)
        );
        // A saved host's own DNS name always wins, so it cannot be an alias.
        for (host, alias) in [(&gitea, "forge.example"), (&gitea, "code.example")] {
            assert!(config.add_ssh_alias(host, alias).is_err(), "{alias}");
        }
        for alias in ["", "bad host", "a/b", "user@forge-ssh", "forge-ssh:22"] {
            assert!(config.add_ssh_alias(&gitea, alias).is_err(), "{alias:?}");
        }
        let unknown = Host::parse("https://new.example").unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            config
                .add_ssh_alias(&unknown, "elsewhere")
                .err()
                .map(|error| error.code()),
            Some(3)
        );
        assert!(config.host_for_ssh("elsewhere").is_none());

        // A new token for the host keeps its aliases; logout drops them.
        config
            .login(
                &gitea,
                "user".to_owned(),
                Token::parse("rotated".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(config.host_for_ssh("forge-ssh"), Some(gitea.clone()));
        assert!(config.remove_ssh_alias(&gitea, "missing").is_err());
        config
            .remove_ssh_alias(&gitea, "forge-ssh")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(config.host_for_ssh("forge-ssh").is_none());
        config
            .add_ssh_alias(&gitea, "forge-ssh")
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(config.logout(&gitea));
        assert!(config.host_for_ssh("forge-ssh").is_none());
    }

    #[test]
    fn token_files_with_shared_or_malformed_ssh_aliases_are_rejected() {
        for (first, second) in [
            (r#"["forge-ssh"]"#, r#"["forge-ssh"]"#),
            (r#"["Forge-SSH"]"#, "[]"),
            (r#"["bad host"]"#, "[]"),
            (r#"["forge-ssh","forge-ssh"]"#, "[]"),
        ] {
            let text = format!(
                r#"{{"version":1,"default_host":null,"hosts":{{"https://one.example":{{"token":"a","ssh_aliases":{first}}},"https://two.example":{{"token":"b","ssh_aliases":{second}}}}}}}"#
            );
            let config: Config =
                serde_json::from_str(&text).unwrap_or_else(|error| panic!("{error}"));
            assert!(config.validate().is_err(), "{text}");
        }
        let text = r#"{"version":1,"default_host":null,"hosts":{"https://one.example":{"token":"a","ssh_aliases":["forge-ssh"]},"https://two.example":{"token":"b"}}}"#;
        let config: Config = serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}"));
        config.validate().unwrap_or_else(|error| panic!("{error}"));
        // Hosts without aliases keep the file format older versions wrote.
        let saved = serde_json::to_string(&logged_in(&["https://one.example"]))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!saved.contains("ssh_aliases"), "{saved}");
    }

    #[test]
    fn git_credentials_use_the_longest_matching_host_base() {
        let root = Host::parse("https://forge.example").unwrap_or_else(|error| panic!("{error}"));
        let nested =
            Host::parse("https://forge.example/code").unwrap_or_else(|error| panic!("{error}"));
        let mut config = Config::empty();
        config
            .login(
                &root,
                "root-user".to_owned(),
                Token::parse("root-token".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        config
            .login(
                &nested,
                "nested-user".to_owned(),
                Token::parse("nested-token".to_owned()).unwrap_or_else(|error| panic!("{error}")),
            )
            .unwrap_or_else(|error| panic!("{error}"));

        let credential = config
            .git_credential("https", "forge.example", "code/owner/repo.git")
            .unwrap_or_else(|| panic!("credential is missing"));
        assert_eq!(credential.user, "nested-user");
        assert_eq!(credential.token.as_str(), "nested-token");
    }
}
