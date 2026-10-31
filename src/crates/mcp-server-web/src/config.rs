use std::time::Duration;

use anyhow::{Context, Result};
use url::{Host, Url};

const ALLOW_VAR: &str = "MCP_WEB_ALLOW_DOMAINS";
const DENY_VAR: &str = "MCP_WEB_DENY_DOMAINS";
const SEARCH_URL_VAR: &str = "MCP_WEB_SEARCH_URL";
const TIMEOUT_VAR: &str = "MCP_WEB_TIMEOUT_SECS";
const MAX_BYTES_VAR: &str = "MCP_WEB_MAX_BYTES";
const USER_AGENT_VAR: &str = "MCP_WEB_USER_AGENT";
const MAX_CHARS_VAR: &str = "MCP_WEB_MAX_CHARS";
const CACHE_TTL_VAR: &str = "MCP_WEB_CACHE_TTL_SECS";
const CACHE_MAX_ENTRIES_VAR: &str = "MCP_WEB_CACHE_MAX_ENTRIES";
const MAX_REDIRECTS_VAR: &str = "MCP_WEB_MAX_REDIRECTS";
const SEARCH_LIMIT_VAR: &str = "MCP_WEB_SEARCH_LIMIT";
const MAX_IMAGE_PIXELS_VAR: &str = "MCP_WEB_MAX_IMAGE_PIXELS";

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
    /// The most characters one `web_fetch` call may return.
    pub max_chars: usize,
    /// How long a fetched page stays cached. Zero turns the cache off.
    pub cache_ttl: Duration,
    /// How many pages the cache holds before it evicts the least recently
    /// used one. Zero turns the cache off.
    pub cache_max_entries: u64,
    /// How many redirects `web_fetch` follows before it gives up.
    pub max_redirects: usize,
    /// How many results `web_search` returns when the call has no `limit`.
    pub search_limit: usize,
    /// The most pixels an image may have. Bigger images are scaled down.
    /// Zero leaves images as they are.
    pub max_image_pixels: u64,
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
        let max_chars = parse_var(MAX_CHARS_VAR, 20_000)?;
        let cache_ttl = parse_var(CACHE_TTL_VAR, 5 * 60)?;
        let cache_max_entries = parse_var(CACHE_MAX_ENTRIES_VAR, 128)?;
        let max_redirects = parse_var(MAX_REDIRECTS_VAR, 10)?;
        let search_limit = parse_var(SEARCH_LIMIT_VAR, 10)?;
        let max_image_pixels = parse_var(MAX_IMAGE_PIXELS_VAR, 1024 * 1024)?;

        Ok(Self {
            policy,
            search_url,
            timeout: Duration::from_secs(timeout),
            max_bytes,
            user_agent,
            max_chars,
            cache_ttl: Duration::from_secs(cache_ttl),
            cache_max_entries,
            max_redirects,
            search_limit,
            max_image_pixels,
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
