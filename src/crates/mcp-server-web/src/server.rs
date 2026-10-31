use std::{borrow::Cow, io::Cursor, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    ExtendedColorType, ImageEncoder, ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use moka::{future::Cache, policy::EvictionPolicy};
use readabilityrs::{Readability, ReadabilityOptions};
use reqwest::{Client, header, redirect};
use rmcp::{
    ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{Config, DomainPolicy};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchRequest {
    #[schemars(description = "The http(s) URL to fetch")]
    pub url: String,
    #[schemars(
        description = "`readable` (default) returns the main content as Markdown, `full` returns the page as is, HTML included"
    )]
    pub mode: Option<Mode>,
    #[schemars(
        description = "Character to start at (default 0). An offset only means something in the mode that returned it, so keep the mode the same while paging through a page"
    )]
    pub offset: Option<usize>,
    #[schemars(
        description = "Number of characters to return (default and maximum set by the server)"
    )]
    pub length: Option<usize>,
}

/// How `web_fetch` presents an HTML page.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The main content as Markdown.
    #[default]
    Readable,
    /// The page as is.
    Full,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchRequest {
    #[schemars(description = "The search query")]
    pub query: String,
    #[schemars(description = "Maximum number of results to return (default set by the server)")]
    pub limit: Option<usize>,
}

/// One slice of a page.
#[derive(Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct FetchOutput {
    /// The character the slice starts at.
    pub offset: usize,
    /// The number of characters in the slice. Shorter than requested on the
    /// last slice.
    pub length: usize,
    /// The offset to read next, or null when this is the last slice.
    pub next_offset: Option<usize>,
    /// The text of the slice.
    pub content: String,
}

/// The result of `web_fetch`: a slice of text, or a whole image.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    Text(FetchOutput),
    Image(ImageOutput),
}

/// A whole image. The image itself travels as an image block, so only its
/// description is structured content.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ImageOutput {
    /// The MIME type of the image.
    pub content_type: String,
    /// The size of the image in bytes.
    pub bytes: usize,
    /// The image, base64 encoded.
    #[serde(skip)]
    pub data: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SearchOutput {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SearchHit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
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

/// A downloaded page, cached as is. The `readable` conversion happens on
/// every call, so both modes share one entry.
#[derive(Debug)]
struct Page {
    url: Url,
    body: Body,
}

#[derive(Debug)]
enum Body {
    Html(String),
    Text(String),
    Image {
        content_type: &'static str,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct WebServer {
    fetch_client: Client,
    search_client: Client,
    policy: Arc<DomainPolicy>,
    search_url: Option<String>,
    max_bytes: usize,
    max_chars: usize,
    search_limit: usize,
    max_image_pixels: u64,
    cache: Option<Cache<String, Arc<Page>>>,
}

impl WebServer {
    pub fn new(config: Config) -> Result<Self> {
        let policy = Arc::new(config.policy);

        // Every redirect hop is checked against the domain policy, so an
        // allowed site cannot bounce the fetch to a denied one.
        let redirect_policy = Arc::clone(&policy);
        let max_redirects = config.max_redirects;
        let fetch_client = Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.timeout)
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() > max_redirects {
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

        let cache = (!config.cache_ttl.is_zero() && config.cache_max_entries > 0).then(|| {
            Cache::builder()
                .max_capacity(config.cache_max_entries)
                .time_to_live(config.cache_ttl)
                .eviction_policy(EvictionPolicy::lru())
                .build()
        });

        Ok(Self {
            fetch_client,
            search_client,
            policy,
            search_url: config.search_url,
            max_bytes: config.max_bytes,
            max_chars: config.max_chars,
            search_limit: config.search_limit,
            max_image_pixels: config.max_image_pixels,
            cache,
        })
    }

    /// Fetches a page and returns one slice of it as text, or an image whole.
    pub async fn fetch(
        &self,
        raw_url: &str,
        mode: Mode,
        offset: usize,
        length: Option<usize>,
    ) -> Result<Fetched> {
        let requested = length;
        let length = requested.unwrap_or(self.max_chars);
        if length > self.max_chars {
            bail!(
                "length {length} is larger than the maximum of {} characters",
                self.max_chars
            );
        }

        let url = Url::parse(raw_url).with_context(|| format!("invalid url {raw_url:?}"))?;
        if !self.policy.allows(&url) {
            bail!("{url} is not allowed by the domain policy");
        }

        let page = self.load(&url).await?;
        let text = match &page.body {
            Body::Image { content_type, data } => {
                if offset != 0 || requested.is_some() {
                    bail!("offset and length do not apply to images, which are returned whole");
                }
                return Ok(Fetched::Image(ImageOutput {
                    content_type: (*content_type).to_owned(),
                    bytes: data.len(),
                    data: STANDARD.encode(data),
                }));
            }
            Body::Html(html) if mode == Mode::Readable => {
                let html = html.clone();
                let page_url = page.url.to_string();
                Cow::Owned(
                    tokio::task::spawn_blocking(move || extract_article(&html, &page_url))
                        .await
                        .context("content extraction panicked")?,
                )
            }
            Body::Html(text) | Body::Text(text) => Cow::Borrowed(text.as_str()),
        };

        paginate(&text, offset, length).map(Fetched::Text)
    }

    /// Returns the page from the cache, or downloads and caches it. Calls for
    /// a page that is being downloaded wait for that download.
    async fn load(&self, url: &Url) -> Result<Arc<Page>> {
        let Some(cache) = &self.cache else {
            return Ok(Arc::new(self.download(url).await?));
        };

        cache
            .try_get_with(url.as_str().to_owned(), async {
                self.download(url).await.map(Arc::new)
            })
            .await
            .map_err(|error| anyhow!("{error:#}"))
    }

    async fn download(&self, url: &Url) -> Result<Page> {
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
        let body = if let Some(image) = image_type(&content_type) {
            let max_pixels = self.max_image_pixels;
            let (content_type, data) = tokio::task::spawn_blocking(move || {
                shrink_image(image, body, max_pixels, MAX_DECODED_PIXELS)
            })
            .await
            .context("image processing panicked")?
            .with_context(|| format!("failed to process the image at {url}"))?;
            Body::Image { content_type, data }
        } else if content_type.contains("html") {
            Body::Html(String::from_utf8_lossy(&body).into_owned())
        } else if is_textual(&content_type) {
            Body::Text(String::from_utf8_lossy(&body).into_owned())
        } else {
            bail!("{url} has unsupported content type {content_type:?}");
        };

        Ok(Page {
            url: final_url,
            body,
        })
    }

    /// Searches the web and returns the results the domain policy allows.
    pub async fn search(&self, query: &str, limit: usize) -> Result<SearchOutput> {
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
            .map(|result| SearchHit {
                title: result.title.filter(|title| !title.trim().is_empty()),
                url: result.url,
                snippet: result
                    .content
                    .map(|content| content.trim().to_owned())
                    .filter(|content| !content.is_empty()),
            })
            .collect();

        Ok(SearchOutput { results })
    }
}

#[tool_router]
impl WebServer {
    #[tool(
        description = "Fetch a web page and return a slice of it: the content with its offset, length and next_offset. In readable mode (default) HTML becomes its main content as Markdown, without navigation and other clutter. In full mode it is returned as is. Non-HTML text such as JSON is always returned as is. Pages longer than one slice are read by repeating the call with the next_offset, which is null on the last slice. Offsets count characters of the result in the chosen mode, so keep the mode the same between calls. PNG, JPEG, GIF and WebP images are returned whole as image content, so leave offset and length out for them."
    )]
    async fn web_fetch(
        &self,
        Parameters(FetchRequest {
            url,
            mode,
            offset,
            length,
        }): Parameters<FetchRequest>,
    ) -> Result<CallToolResult, String> {
        let mode = mode.unwrap_or_default();
        let offset = offset.unwrap_or(0);
        let fetched = self
            .fetch(&url, mode, offset, length)
            .await
            .map_err(describe)?;
        fetched_result(fetched)
    }

    #[tool(description = "Search the web and return result titles, URLs and snippets.")]
    async fn web_search(
        &self,
        Parameters(SearchRequest { query, limit }): Parameters<SearchRequest>,
    ) -> Result<Json<SearchOutput>, String> {
        let limit = limit.unwrap_or(self.search_limit);
        self.search(&query, limit).await.map(Json).map_err(describe)
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

/// A failed call is a tool error with the whole cause chain as its text, not
/// a protocol error, so the agent can react to it.
fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// Text is structured content. An image is an image block, with its
/// description as structured content, and also as text for clients that do
/// not read structured content.
fn fetched_result(fetched: Fetched) -> Result<CallToolResult, String> {
    match fetched {
        Fetched::Text(slice) => serde_json::to_value(&slice)
            .map(CallToolResult::structured)
            .map_err(|error| error.to_string()),
        Fetched::Image(image) => {
            let description = serde_json::to_value(&image).map_err(|error| error.to_string())?;
            let mut result = CallToolResult::success(vec![
                ContentBlock::image(image.data, image.content_type),
                ContentBlock::text(description.to_string()),
            ]);
            result.structured_content = Some(description);
            Ok(result)
        }
    }
}

/// The most pixels an image may have to be decoded at all. A small file can
/// unpack into gigabytes, so bigger images are refused.
const MAX_DECODED_PIXELS: u64 = 50_000_000;

const JPEG_QUALITY: u8 = 85;

/// Scales an image down to at most `max_pixels`, keeping its aspect ratio,
/// and encodes it as JPEG. An image that is small enough is returned as it
/// is, and so is any image when `max_pixels` is zero. Animated images lose
/// all frames but the first.
fn shrink_image(
    content_type: &'static str,
    data: Vec<u8>,
    max_pixels: u64,
    max_decoded_pixels: u64,
) -> Result<(&'static str, Vec<u8>)> {
    if max_pixels == 0 {
        return Ok((content_type, data));
    }

    let (width, height) = ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .context("failed to read the image")?
        .into_dimensions()
        .context("failed to read the size of the image")?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels <= max_pixels {
        return Ok((content_type, data));
    }
    if pixels > max_decoded_pixels {
        bail!("image has {pixels} pixels, more than the {max_decoded_pixels} that can be decoded");
    }

    let image = ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .context("failed to read the image")?
        .decode()
        .context("failed to decode the image")?;

    // NOTE: rounding down keeps the result within the limit
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let width = ((f64::from(width) * scale) as u32).max(1);
    let height = ((f64::from(height) * scale) as u32).max(1);
    let image = image
        .resize_exact(width, height, FilterType::Lanczos3)
        .into_rgb8();

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
        .context("failed to encode the image")?;
    Ok(("image/jpeg", jpeg))
}

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The image type in a `Content-Type` header, when `web_fetch` supports it.
fn image_type(content_type: &str) -> Option<&'static str> {
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    IMAGE_TYPES.into_iter().find(|image| *image == essence)
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

/// Cuts `length` characters from `offset`. The next offset is `None` when
/// nothing is left.
fn paginate(text: &str, offset: usize, length: usize) -> Result<FetchOutput> {
    let total = text.chars().count();
    if offset > total {
        bail!("offset {offset} is past the end of the content, which has {total} characters");
    }

    let length = length.min(total - offset);
    let start = byte_index(text, offset);
    let end = start + byte_index(&text[start..], length);

    Ok(FetchOutput {
        offset,
        length,
        next_offset: (offset + length < total).then_some(offset + length),
        content: text[start..end].to_owned(),
    })
}

/// The byte index of the character at `chars`, or the end of the text.
fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(index, _)| index)
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
    fn paginate_cuts_on_characters() {
        assert_eq!(
            paginate("héllo", 1, 2).unwrap(),
            FetchOutput {
                offset: 1,
                length: 2,
                next_offset: Some(3),
                content: "él".to_owned(),
            }
        );
    }

    #[test]
    fn paginate_shortens_the_last_slice() {
        assert_eq!(
            paginate("héllo", 3, 10).unwrap(),
            FetchOutput {
                offset: 3,
                length: 2,
                next_offset: None,
                content: "lo".to_owned(),
            }
        );
    }

    #[test]
    fn paginate_allows_an_offset_at_the_end_only() {
        assert_eq!(
            paginate("abc", 3, 5).unwrap(),
            FetchOutput {
                offset: 3,
                length: 0,
                next_offset: None,
                content: String::new(),
            }
        );
        assert!(paginate("abc", 4, 5).is_err());
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn shrink_image_leaves_small_images_alone() {
        let data = png(10, 10);

        let (content_type, shrunk) = shrink_image("image/png", data.clone(), 1000, 5000).unwrap();

        assert_eq!(content_type, "image/png");
        assert_eq!(shrunk, data);
    }

    #[test]
    fn shrink_image_does_nothing_without_a_limit() {
        let data = png(40, 20);

        let (content_type, shrunk) = shrink_image("image/png", data.clone(), 0, 5000).unwrap();

        assert_eq!(content_type, "image/png");
        assert_eq!(shrunk, data);
    }

    #[test]
    fn shrink_image_scales_down_to_jpeg_keeping_the_aspect_ratio() {
        let (content_type, shrunk) = shrink_image("image/png", png(40, 20), 200, 5000).unwrap();

        assert_eq!(content_type, "image/jpeg");
        let image = image::load_from_memory(&shrunk).unwrap();
        assert_eq!((image.width(), image.height()), (20, 10));
    }

    #[test]
    fn shrink_image_refuses_to_decode_huge_images() {
        let error = shrink_image("image/png", png(40, 20), 100, 500).unwrap_err();

        assert!(
            format!("{error:#}").contains("that can be decoded"),
            "{error:#}"
        );
    }

    #[test]
    fn shrink_image_refuses_data_that_is_no_image() {
        let error = shrink_image("image/png", b"abc".to_vec(), 100, 500).unwrap_err();

        assert!(format!("{error:#}").contains("failed to"), "{error:#}");
    }

    #[test]
    fn image_content_types() {
        assert_eq!(image_type("image/png"), Some("image/png"));
        assert_eq!(image_type("image/webp; q=1"), Some("image/webp"));
        assert_eq!(image_type("image/svg+xml"), None);
        assert_eq!(image_type("text/plain"), None);
    }

    #[test]
    fn textual_content_types() {
        assert!(is_textual("text/plain; charset=utf-8"));
        assert!(is_textual("application/json"));
        assert!(!is_textual("image/png"));
        assert!(!is_textual("application/pdf"));
    }
}
