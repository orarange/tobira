use std::fmt;

use crate::error::{BrowserError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let (scheme, remainder) = trimmed
            .split_once("://")
            .ok_or_else(|| BrowserError::message("URL must include a scheme such as http://"))?;

        if scheme != "http" && scheme != "https" {
            return Err(BrowserError::message(format!(
                "unsupported scheme: {scheme} (only http:// and https:// are supported)"
            )));
        }

        let split_index = remainder.find(['/', '?']).unwrap_or(remainder.len());
        let authority = &remainder[..split_index];
        let mut path = if split_index < remainder.len() {
            remainder[split_index..].to_string()
        } else {
            "/".to_string()
        };

        if path.starts_with('?') {
            path = format!("/{path}");
        }

        if authority.is_empty() {
            return Err(BrowserError::message("URL is missing a host"));
        }

        let (host, port) = parse_authority(authority, scheme)?;

        Ok(Self {
            scheme: scheme.to_string(),
            host,
            port,
            path: normalize_path(&path),
        })
    }

    pub fn resolve(&self, location: &str) -> Result<Self> {
        if location.contains("://") {
            return Self::parse(location);
        }

        let current_without_fragment = self.path.split('#').next().unwrap_or(&self.path);
        let current_path = current_without_fragment
            .split('?')
            .next()
            .unwrap_or(current_without_fragment);

        let next_path = if location.starts_with('/') {
            location.to_string()
        } else if location.starts_with('?') {
            format!("{current_path}{location}")
        } else if location.starts_with('#') {
            format!("{current_without_fragment}{location}")
        } else {
            let directory = match current_path.rsplit_once('/') {
                Some((prefix, _)) if prefix.is_empty() => "/".to_string(),
                Some((prefix, _)) => format!("{prefix}/"),
                None => "/".to_string(),
            };
            format!("{directory}{location}")
        };

        Ok(Self {
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.port,
            path: normalize_path(&next_path),
        })
    }

    pub fn host_header(&self) -> String {
        if (self.scheme == "http" && self.port == 80)
            || (self.scheme == "https" && self.port == 443)
        {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn shares_origin(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.port == other.port
            && self.host.eq_ignore_ascii_case(&other.host)
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if (self.scheme == "http" && self.port == 80)
            || (self.scheme == "https" && self.port == 443)
        {
            write!(f, "{}://{}{}", self.scheme, self.host, self.path)
        } else {
            write!(
                f,
                "{}://{}:{}{}",
                self.scheme, self.host, self.port, self.path
            )
        }
    }
}

fn parse_authority(authority: &str, scheme: &str) -> Result<(String, u16)> {
    if let Some((host, port)) = authority.rsplit_once(':') {
        if !host.is_empty() && port.chars().all(|char| char.is_ascii_digit()) {
            let parsed_port = port
                .parse::<u16>()
                .map_err(|_| BrowserError::message("port must be between 0 and 65535"))?;
            return Ok((host.to_string(), parsed_port));
        }
    }

    let default_port = if scheme == "https" { 443 } else { 80 };
    Ok((authority.to_string(), default_port))
}

fn normalize_path(input: &str) -> String {
    let suffix_start = input.find(['?', '#']).unwrap_or(input.len());
    let path_only = &input[..suffix_start];
    let suffix = &input[suffix_start..];
    let mut segments = Vec::new();

    for segment in path_only.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }

    let mut normalized = String::from("/");
    normalized.push_str(&segments.join("/"));

    if path_only.ends_with('/') && !normalized.ends_with('/') {
        normalized.push('/');
    }

    if normalized.is_empty() {
        normalized.push('/');
    }

    normalized.push_str(suffix);
    normalized
}

#[cfg(test)]
mod tests {
    use super::Url;

    #[test]
    fn parses_basic_http_url() {
        let url = Url::parse("http://example.com/docs/index.html?lang=ja").unwrap();

        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/docs/index.html?lang=ja");
    }

    #[test]
    fn parses_custom_port() {
        let url = Url::parse("http://localhost:8080/test").unwrap();

        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, 8080);
        assert_eq!(url.path, "/test");
    }

    #[test]
    fn parses_https_url() {
        let url = Url::parse("https://www.google.com/search?q=rust").unwrap();

        assert_eq!(url.host, "www.google.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "/search?q=rust");
    }

    #[test]
    fn resolves_relative_paths() {
        let base = Url::parse("http://example.com/notes/posts/start.html").unwrap();
        let next = base.resolve("../next.html").unwrap();

        assert_eq!(next.to_string(), "http://example.com/notes/next.html");
    }

    #[test]
    fn resolves_fragment_only_locations() {
        let base = Url::parse("https://example.com/find?src=home#old").unwrap();
        let next = base.resolve("#results").unwrap();

        assert_eq!(next.to_string(), "https://example.com/find?src=home#results");
    }

    #[test]
    fn resolves_query_locations_against_fragmented_urls() {
        let base = Url::parse("https://example.com/find?src=home#old").unwrap();
        let next = base.resolve("?q=rust").unwrap();

        assert_eq!(next.to_string(), "https://example.com/find?q=rust");
    }

    #[test]
    fn compares_same_origin_urls() {
        let left = Url::parse("https://Example.com/path").unwrap();
        let same = Url::parse("https://example.com/other").unwrap();
        let other_port = Url::parse("https://example.com:444/other").unwrap();
        let explicit_default_port = Url::parse("https://example.com:443/third").unwrap();

        assert!(left.shares_origin(&same));
        assert!(left.shares_origin(&explicit_default_port));
        assert!(!left.shares_origin(&other_port));
    }
}
