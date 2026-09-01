use std::collections::BTreeMap;
use std::env;
#[cfg(windows)]
use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::Read;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::host::Host;

#[derive(Clone)]
pub(crate) struct Token(String);

impl Token {
    pub(crate) fn parse(value: String) -> Result<Self, Error> {
        Self::validate(&value)?;
        Ok(Self(value))
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
}

pub(crate) struct GitCredential {
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
        let mut bytes = Vec::new();
        fs::File::open(path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
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
            if entry.user.as_deref().is_some_and(|user| !valid_user(user)) {
                return Err(Error::context("invalid user in token file"));
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
            .map(|entry| Token(entry.token.clone()))
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

    pub(crate) fn host_matching_dns(&self, dns: &str) -> Option<Host> {
        self.hosts
            .keys()
            .filter_map(|value| Host::parse(value).ok())
            .find(|host| host.dns_host().eq_ignore_ascii_case(dns))
    }

    pub(crate) fn login(&mut self, host: &Host, user: String, token: Token) -> Result<(), Error> {
        if !valid_user(&user) {
            return Err(Error::data("Forgejo returned an invalid user name"));
        }
        self.hosts.insert(
            host.as_str().to_owned(),
            HostEntry {
                user: Some(user),
                token: token.into_string(),
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
                        user: entry.user.clone()?,
                        token: Token(entry.token.clone()),
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

    #[cfg(not(unix))]
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

#[cfg(not(unix))]
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
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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
