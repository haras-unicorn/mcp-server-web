use std::sync::Arc;

use anyhow::{Context, Result, bail};
use readabilityrs::{Readability, ReadabilityOptions};
use reqwest::{Client, header, redirect};
use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use url::Url;

use crate::config::{Config, DomainPolicy};

const DEFAULT_MAX_CHARS: usize = 20_000;
const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchRequest {
    #[schemars(description = "The http(s) URL to fetch")]
    pub url: String,
    #[schemars(description = "Maximum number of characters to return (default 20000)")]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRequest {
    #[schemars(description = "The search query")]
    pub query: String,
    #[schemars(description = "Maximum number of results to return (default 10)")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    title: Option<String>,
    url: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebServer {
    fetch_client: Client,
    search_client: Client,
    policy: Arc<DomainPolicy>,
    search_url: Option<String>,
    max_bytes: usize,
}

impl WebServer {
    pub fn new(config: Config) -> Result<Self> {
        let policy = Arc::new(config.policy);

        // Every redirect hop is checked against the domain policy, so an
        // allowed site cannot bounce the fetch to a denied one.
        let redirect_policy = Arc::clone(&policy);
        let fetch_client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.timeout)
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if !redirect_policy.allows(attempt.url()) {
                    let message = format!("redirect to blocked url {}", attempt.url());
                    attempt.error(message)
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .context("failed to build the fetch client")?;

        // The search engine is chosen by the operator, so it is not subject to
        // the domain policy.
        let search_client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.timeout)
            .build()
            .context("failed to build the search client")?;

        Ok(Self {
            fetch_client,
            search_client,
            policy,
            search_url: config.search_url,
            max_bytes: config.max_bytes,
        })
    }

    /// Fetches a page and returns its main content as text.
    pub async fn fetch(&self, raw_url: &str, max_chars: usize) -> Result<String> {
        let url = Url::parse(raw_url).with_context(|| format!("invalid url {raw_url:?}"))?;
        if !self.policy.allows(&url) {
            bail!("{url} is not allowed by the domain policy");
        }

        let mut response = self
            .fetch_client
            .get(url.clone())
            .send()
            .await
            .with_context(|| format!("request to {url} failed"))?;

        let status = response.status();
        if !status.is_success() {
            bail!("{url} responded with {status}");
        }

        let final_url = response.url().clone();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.context("failed to read the body")? {
            if body.len() + chunk.len() > self.max_bytes {
                bail!("{url} is larger than {} bytes", self.max_bytes);
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&body).into_owned();

        let text = if content_type.contains("html") {
            let page_url = final_url.to_string();
            tokio::task::spawn_blocking(move || extract_article(&body, &page_url))
                .await
                .context("content extraction panicked")?
        } else if is_textual(&content_type) {
            body
        } else {
            bail!("{url} has unsupported content type {content_type:?}");
        };

        Ok(truncate(text, max_chars))
    }

    /// Searches the web and returns a numbered list of results.
    pub async fn search(&self, query: &str, limit: usize) -> Result<String> {
        let template = self
            .search_url
            .as_deref()
            .context("web_search is not configured: set MCP_WEB_SEARCH_URL")?;
        let url = search_url(template, query)?;

        let response = self
            .search_client
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .context("search request failed")?;

        let status = response.status();
        if !status.is_success() {
            bail!("search engine responded with {status}");
        }

        let response: SearchResponse = response
            .json()
            .await
            .context("search engine did not return SearXNG style JSON")?;

        let results: Vec<_> = response
            .results
            .into_iter()
            .filter(|result| {
                Url::parse(&result.url)
                    .map(|url| self.policy.allows(&url))
                    .unwrap_or(false)
            })
            .take(limit)
            .collect();

        if results.is_empty() {
            return Ok("No results.".to_owned());
        }

        let mut output = String::new();
        for (index, result) in results.iter().enumerate() {
            let title = result.title.as_deref().unwrap_or(&result.url);
            output.push_str(&format!("{}. {title}\n   {}\n", index + 1, result.url));
            if let Some(content) = result.content.as_deref().map(str::trim)
                && !content.is_empty()
            {
                output.push_str(&format!("   {content}\n"));
            }
            output.push('\n');
        }
        Ok(output.trim_end().to_owned())
    }
}

#[tool_router]
impl WebServer {
    #[tool(
        description = "Fetch a web page and return its main content as Markdown, without navigation and other clutter. Non-HTML text such as JSON is returned as is."
    )]
    async fn web_fetch(
        &self,
        Parameters(FetchRequest { url, max_chars }): Parameters<FetchRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let max_chars = max_chars.unwrap_or(DEFAULT_MAX_CHARS);
        Ok(into_result(self.fetch(&url, max_chars).await))
    }

    #[tool(description = "Search the web and return result titles, URLs and snippets.")]
    async fn web_search(
        &self,
        Parameters(SearchRequest { query, limit }): Parameters<SearchRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        Ok(into_result(self.search(&query, limit).await))
    }
}

#[tool_handler]
impl ServerHandler for WebServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "mcp-server-web",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Fetch web pages as Markdown with web_fetch and search the web with web_search.",
            )
    }
}

fn into_result(outcome: Result<String>) -> CallToolResult {
    match outcome {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
    }
}

fn is_textual(content_type: &str) -> bool {
    content_type.is_empty()
        || content_type.starts_with("text/")
        || ["json", "xml", "yaml", "javascript"]
            .iter()
            .any(|kind| content_type.contains(kind))
}

/// Strips an HTML page down to its article, as Markdown. Falls back to the
/// page as is when no article can be found.
fn extract_article(html: &str, url: &str) -> String {
    let options = ReadabilityOptions::builder().output_markdown(true).build();
    let article = Readability::new(html, Some(url), Some(options))
        .ok()
        .and_then(|readability| readability.parse());

    let Some(article) = article else {
        return html.to_owned();
    };
    let Some(body) = article
        .markdown_content
        .or(article.text_content)
        .or(article.content)
    else {
        return html.to_owned();
    };

    match article.title.filter(|title| !title.trim().is_empty()) {
        Some(title) => format!("# {}\n\n{body}", title.trim()),
        None => body,
    }
}

fn truncate(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => {
            let omitted = text[index..].chars().count();
            format!(
                "{}\n\n[truncated, {omitted} more characters]",
                &text[..index]
            )
        }
        None => text,
    }
}

/// Builds the search request URL. A `{query}` placeholder in the template is
/// replaced with the URL-encoded query. Without one, `q` and `format=json`
/// parameters are appended, which is what SearXNG expects.
fn search_url(template: &str, query: &str) -> Result<Url> {
    if template.contains("{query}") {
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = template.replace("{query}", &encoded);
        return Url::parse(&url)
            .with_context(|| format!("invalid MCP_WEB_SEARCH_URL {template:?}"));
    }

    let mut url =
        Url::parse(template).with_context(|| format!("invalid MCP_WEB_SEARCH_URL {template:?}"))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("format", "json");
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_appends_searxng_parameters() {
        let url = search_url("https://searx.example/search", "rust mcp").unwrap();
        assert_eq!(
            url.as_str(),
            "https://searx.example/search?q=rust+mcp&format=json"
        );
    }

    #[test]
    fn search_url_fills_the_placeholder() {
        let url = search_url("https://engine.example/api?term={query}&n=5", "a&b").unwrap();
        assert_eq!(url.as_str(), "https://engine.example/api?term=a%26b&n=5");
    }

    #[test]
    fn truncate_cuts_on_characters() {
        assert_eq!(truncate("héllo".to_owned(), 10), "héllo");
        assert_eq!(
            truncate("héllo".to_owned(), 2),
            "hé\n\n[truncated, 3 more characters]"
        );
    }

    #[test]
    fn textual_content_types() {
        assert!(is_textual("text/plain; charset=utf-8"));
        assert!(is_textual("application/json"));
        assert!(!is_textual("image/png"));
        assert!(!is_textual("application/pdf"));
    }
}
