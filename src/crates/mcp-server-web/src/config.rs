use std::time::Duration;

use anyhow::{Context, Result};
use url::{Host, Url};

const ALLOW_VAR: &str = "MCP_WEB_ALLOW_DOMAINS";
const DENY_VAR: &str = "MCP_WEB_DENY_DOMAINS";
const SEARCH_URL_VAR: &str = "MCP_WEB_SEARCH_URL";
const TIMEOUT_VAR: &str = "MCP_WEB_TIMEOUT_SECS";
const MAX_BYTES_VAR: &str = "MCP_WEB_MAX_BYTES";
const USER_AGENT_VAR: &str = "MCP_WEB_USER_AGENT";

/// Which domains `web_fetch` (and search results) may touch.
///
/// A host matches an entry when it is equal to it or a subdomain of it. The
/// denylist always wins. An empty allowlist allows everything that is not
/// denied.
#[derive(Debug, Default, Clone)]
pub struct DomainPolicy {
    allow: Vec<String>,
    deny: Vec<String>,
}

impl DomainPolicy {
    pub fn new(allow: &str, deny: &str) -> Self {
        Self {
            allow: parse_domains(allow),
            deny: parse_domains(deny),
        }
    }

    pub fn allows(&self, url: &Url) -> bool {
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }
        // NOTE: only names have subdomains - an address must match exactly
        let (host, is_name) = match url.host() {
            Some(Host::Domain(name)) => (name.trim_end_matches('.').to_ascii_lowercase(), true),
            Some(Host::Ipv4(address)) => (address.to_string(), false),
            Some(Host::Ipv6(address)) => (address.to_string(), false),
            None => return false,
        };

        if self
            .deny
            .iter()
            .any(|entry| matches_host(&host, is_name, entry))
        {
            return false;
        }
        self.allow.is_empty()
            || self
                .allow
                .iter()
                .any(|entry| matches_host(&host, is_name, entry))
    }
}

fn parse_domains(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(|entry| {
            entry
                .trim()
                .trim_start_matches("*.")
                .trim_matches(['.', '[', ']'])
        })
        .filter(|entry| !entry.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn matches_host(host: &str, is_name: bool, entry: &str) -> bool {
    host == entry
        || (is_name
            && host
                .strip_suffix(entry)
                .is_some_and(|rest| rest.ends_with('.')))
}

#[derive(Debug, Clone)]
pub struct Config {
    pub policy: DomainPolicy,
    pub search_url: Option<String>,
    pub timeout: Duration,
    pub max_bytes: usize,
    pub user_agent: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let policy = DomainPolicy::new(&var(ALLOW_VAR), &var(DENY_VAR));
        let search_url = Some(var(SEARCH_URL_VAR)).filter(|value| !value.trim().is_empty());
        let timeout = parse_var(TIMEOUT_VAR, 30)?;
        let max_bytes = parse_var(MAX_BYTES_VAR, 5 * 1024 * 1024)?;
        let user_agent = Some(var(USER_AGENT_VAR))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("mcp-server-web/{}", env!("CARGO_PKG_VERSION")));

        Ok(Self {
            policy,
            search_url,
            timeout: Duration::from_secs(timeout),
            max_bytes,
            user_agent,
        })
    }
}

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn parse_var<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse()
            .with_context(|| format!("invalid value for {name}")),
        _ => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allows(policy: &DomainPolicy, url: &str) -> bool {
        policy.allows(&Url::parse(url).unwrap())
    }

    #[test]
    fn empty_policy_allows_http_and_https_only() {
        let policy = DomainPolicy::default();
        assert!(allows(&policy, "https://example.com/a"));
        assert!(allows(&policy, "http://example.com/a"));
        assert!(!allows(&policy, "file:///etc/passwd"));
        assert!(!allows(&policy, "ftp://example.com/a"));
    }

    #[test]
    fn allowlist_matches_domain_and_subdomains() {
        let policy = DomainPolicy::new("example.com", "");
        assert!(allows(&policy, "https://example.com"));
        assert!(allows(&policy, "https://docs.example.com"));
        assert!(!allows(&policy, "https://notexample.com"));
        assert!(!allows(&policy, "https://example.org"));
    }

    #[test]
    fn denylist_beats_allowlist() {
        let policy = DomainPolicy::new("example.com", "evil.example.com");
        assert!(allows(&policy, "https://example.com"));
        assert!(!allows(&policy, "https://evil.example.com"));
        assert!(!allows(&policy, "https://a.evil.example.com"));
    }

    #[test]
    fn denylist_alone_blocks_only_listed_domains() {
        let policy = DomainPolicy::new("", "bad.com");
        assert!(allows(&policy, "https://good.com"));
        assert!(!allows(&policy, "https://bad.com"));
    }

    #[test]
    fn addresses_only_match_exactly() {
        let policy = DomainPolicy::new("", "1.1, [::1]");
        assert!(allows(&policy, "http://8.8.1.1"));
        assert!(!allows(&policy, "http://[::1]:8080"));

        let policy = DomainPolicy::new("", "127.0.0.1");
        assert!(!allows(&policy, "http://127.0.0.1:3000"));
        assert!(allows(&policy, "http://127.0.0.2"));
    }

    #[test]
    fn entries_are_normalized() {
        let policy = DomainPolicy::new(" *.Example.com , other.org\nthird.net ", "");
        assert!(allows(&policy, "https://example.com"));
        assert!(allows(&policy, "https://other.org"));
        assert!(allows(&policy, "https://third.net"));
        assert!(!allows(&policy, "https://fourth.io"));
    }
}
