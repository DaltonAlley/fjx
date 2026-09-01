use std::net::Ipv6Addr;

use crate::error::Error;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct Host {
    base: String,
    dns_host: String,
}

impl Host {
    pub(crate) fn parse(value: &str) -> Result<Self, Error> {
        let value = value.trim();
        if value.is_empty()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b' ')
            || value.contains(['?', '#', '\\'])
        {
            return Err(Error::context("host must be a safe HTTPS base URL"));
        }
        let (scheme, rest) = value
            .split_once("://")
            .ok_or_else(|| Error::context("host must include https://"))?;
        if scheme != "https" && scheme != "http" {
            return Err(Error::context("host scheme must be HTTPS"));
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() || authority.contains('@') {
            return Err(Error::context("host must not contain user information"));
        }
        let (authority, dns_host) = canonical_authority(authority)?;
        if scheme == "http" && !matches!(dns_host.as_str(), "localhost" | "127.0.0.1" | "::1") {
            return Err(Error::context(
                "plain HTTP is allowed only for loopback hosts",
            ));
        }
        validate_base_path(path)?;
        let path = path.trim_end_matches('/');
        let base = if path.is_empty() {
            format!("{scheme}://{authority}")
        } else {
            format!("{scheme}://{authority}/{path}")
        };
        Ok(Self { base, dns_host })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.base
    }

    pub(crate) fn dns_host(&self) -> &str {
        &self.dns_host
    }

    pub(crate) fn api_url(&self, relative: &str) -> String {
        format!("{}/api/v1/{}", self.base, relative.trim_start_matches('/'))
    }

    pub(crate) fn matches_git_credential(
        &self,
        protocol: &str,
        authority: &str,
        path: &str,
    ) -> bool {
        let Ok(origin) = Self::parse(&format!("{protocol}://{authority}")) else {
            return false;
        };
        let Some(base_path) = self.base.strip_prefix(origin.as_str()) else {
            return false;
        };
        if base_path.is_empty() {
            return true;
        }
        let Some(base_path) = base_path.strip_prefix('/') else {
            return false;
        };
        let path = path.trim_start_matches('/');
        path == base_path || path.starts_with(&format!("{base_path}/"))
    }
}

fn canonical_authority(authority: &str) -> Result<(String, String), Error> {
    if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed
            .find(']')
            .ok_or_else(|| Error::context("invalid bracketed host"))?;
        let host = &bracketed[..end];
        let suffix = &bracketed[end + 1..];
        let host = host
            .parse::<Ipv6Addr>()
            .map_err(|_| Error::context("invalid host authority"))?
            .to_string();
        if !suffix.is_empty() && !valid_port_suffix(suffix) {
            return Err(Error::context("invalid host authority"));
        }
        return Ok((format!("[{host}]{suffix}"), host));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        _ => (authority, None),
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        || port.is_some_and(|value| !valid_port(value))
    {
        return Err(Error::context("invalid host authority"));
    }
    let host = host.to_ascii_lowercase();
    let authority = port.map_or_else(|| host.clone(), |port| format!("{host}:{port}"));
    Ok((authority, host))
}

fn valid_port_suffix(suffix: &str) -> bool {
    suffix.strip_prefix(':').is_some_and(valid_port)
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

fn validate_base_path(path: &str) -> Result<(), Error> {
    for segment in path.split('/') {
        let bytes = segment.as_bytes();
        let mut index = 0;
        let mut dots = 0;
        let mut only_dots = !segment.is_empty();

        while index < bytes.len() {
            match bytes[index] {
                b'.' => {
                    dots += 1;
                    index += 1;
                }
                b'%' => {
                    let encoded = bytes
                        .get(index + 1..index + 3)
                        .and_then(percent_byte)
                        .ok_or_else(|| Error::context("host base path has invalid encoding"))?;
                    if matches!(encoded, b'/' | b'\\') {
                        return Err(Error::context(
                            "host base path must not encode path separators",
                        ));
                    }
                    if encoded == b'.' {
                        dots += 1;
                    } else {
                        only_dots = false;
                    }
                    index += 3;
                }
                _ => {
                    only_dots = false;
                    index += 1;
                }
            }
        }

        if only_dots && matches!(dots, 1 | 2) {
            return Err(Error::context(
                "host base path must not contain dot segments",
            ));
        }
    }
    Ok(())
}

fn percent_byte(digits: &[u8]) -> Option<u8> {
    let [high, low] = digits else {
        return None;
    };
    Some(hex_value(*high)? * 16 + hex_value(*low)?)
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Host;

    #[test]
    fn accepts_contract_hosts() {
        for value in [
            "https://forge.example",
            "https://forge.example/base/",
            "https://forge.example/a.b/%20base",
            "https://forge.example:65535/base",
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
        ] {
            assert!(Host::parse(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn rejects_unsafe_hosts() {
        for value in [
            "http://forge.example",
            "https://user@forge.example",
            "https://forge.example?x=1",
            "https://forge.example/#x",
            "file:///tmp/forge",
        ] {
            assert!(Host::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn canonicalizes_dns_names_without_changing_ports_or_paths() {
        let host = Host::parse("https://FORGE.Example:0443/Forgejo/")
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(host.as_str(), "https://forge.example:0443/Forgejo");
        assert_eq!(host.dns_host(), "forge.example");
    }

    #[test]
    fn canonicalizes_https_ipv6_without_changing_ports_or_paths() {
        let host = Host::parse("https://[2001:0DB8:0000:0000:0000:0000:0000:0001]:0443/Forgejo/")
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(host.as_str(), "https://[2001:db8::1]:0443/Forgejo");
        assert_eq!(host.dns_host(), "2001:db8::1");
    }

    #[test]
    fn keeps_plain_http_ipv6_loopback_only() {
        assert!(Host::parse("http://[::1]:3000/base").is_ok());
        assert!(Host::parse("http://[2001:db8::1]:3000/base").is_err());
    }

    #[test]
    fn rejects_encoded_dot_segments_and_encoded_separators() {
        for value in [
            "https://forge.example/prefix/%2e%2e/base",
            "https://forge.example/prefix/%2E/base",
            "https://forge.example/prefix/.%2e/base",
            "https://forge.example/prefix/%2e./base",
            "https://forge.example/prefix%2f%2e%2e/base",
            "https://forge.example/prefix/%2e%2e%2Fbase",
            "https://forge.example/prefix%5C.%2E/base",
            "https://forge.example/prefix/%2E%2e%5cbase",
        ] {
            assert!(Host::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn rejects_ports_outside_u16() {
        for value in [
            "https://forge.example:65536",
            "http://127.0.0.1:999999999999999999999",
            "http://[::1]:65536",
        ] {
            assert!(Host::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn matches_git_credentials_only_within_the_host_base() {
        let root = Host::parse("https://forge.example").unwrap_or_else(|error| panic!("{error}"));
        let nested =
            Host::parse("https://forge.example/code").unwrap_or_else(|error| panic!("{error}"));

        assert!(root.matches_git_credential("https", "FORGE.example", "owner/repo.git"));
        assert!(nested.matches_git_credential("https", "forge.example", "code/owner/repo.git"));
        assert!(!nested.matches_git_credential("https", "forge.example", "other/repo.git"));
        assert!(!root.matches_git_credential("https", "other.example", "owner/repo.git"));
        assert!(!root.matches_git_credential("http", "forge.example", "owner/repo.git"));
    }
}
