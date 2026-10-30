//! Runs the tools against a small fake web server on localhost.

use std::time::Duration;

use mcp_server_web::config::{Config, DomainPolicy};
use mcp_server_web::server::WebServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn ok(content_type: &str, body: &str) -> String {
    response("200 OK", content_type, body)
}

fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// Serves canned responses by request path (without the query) and returns
/// the port. Unknown paths get a 404.
async fn serve(routes: Vec<(&'static str, String)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let target = request.split_whitespace().nth(1).unwrap_or("/");
                let path = target.split('?').next().unwrap_or("/");

                let reply = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map(|(_, reply)| reply.clone())
                    .unwrap_or_else(|| response("404 Not Found", "text/plain", "not found"));
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    port
}

fn server(policy: DomainPolicy, search_url: Option<String>) -> WebServer {
    WebServer::new(Config {
        policy,
        search_url,
        timeout: Duration::from_secs(5),
        max_bytes: 1024 * 1024,
        user_agent: "mcp-server-web-tests".to_owned(),
    })
    .unwrap()
}

fn open() -> DomainPolicy {
    DomainPolicy::default()
}

fn article_page() -> String {
    let paragraph = "Readability keeps the paragraphs that carry the article and \
                     drops the rest of the page, so a model sees mostly useful text. ";
    let body = paragraph.repeat(12);
    format!(
        "<html><head><title>Test Article</title></head><body>\
         <nav><ul><li><a href=\"/a\">Navigation Junk</a></li></ul></nav>\
         <article><h1>Test Article</h1><p>{body}</p><p>{body}</p></article>\
         <footer>Footer Junk</footer></body></html>"
    )
}

#[tokio::test]
async fn fetch_returns_the_article_of_a_page() {
    let port = serve(vec![(
        "/post",
        ok("text/html; charset=utf-8", &article_page()),
    )])
    .await;
    let server = server(open(), None);

    let text = server
        .fetch(&format!("http://127.0.0.1:{port}/post"), 20_000)
        .await
        .unwrap();

    assert!(text.contains("Test Article"), "{text}");
    assert!(text.contains("Readability keeps the paragraphs"), "{text}");
}

#[tokio::test]
async fn fetch_returns_json_as_is() {
    let port = serve(vec![("/data", ok("application/json", r#"{"a": 1}"#))]).await;
    let server = server(open(), None);

    let text = server
        .fetch(&format!("http://127.0.0.1:{port}/data"), 20_000)
        .await
        .unwrap();

    assert_eq!(text, r#"{"a": 1}"#);
}

#[tokio::test]
async fn fetch_truncates_long_content() {
    let port = serve(vec![("/long", ok("text/plain", &"x".repeat(100)))]).await;
    let server = server(open(), None);

    let text = server
        .fetch(&format!("http://127.0.0.1:{port}/long"), 10)
        .await
        .unwrap();

    assert!(
        text.starts_with("xxxxxxxxxx\n\n[truncated, 90 more characters]"),
        "{text}"
    );
}

#[tokio::test]
async fn fetch_rejects_binary_content() {
    let port = serve(vec![("/image", ok("image/png", "not really a png"))]).await;
    let server = server(open(), None);

    let error = server
        .fetch(&format!("http://127.0.0.1:{port}/image"), 20_000)
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("unsupported content type"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_reports_http_errors() {
    let port = serve(vec![]).await;
    let server = server(open(), None);

    let error = server
        .fetch(&format!("http://127.0.0.1:{port}/missing"), 20_000)
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("404"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_denied_domains() {
    let port = serve(vec![("/", ok("text/plain", "secret"))]).await;
    let server = server(DomainPolicy::new("", "127.0.0.1"), None);

    let error = server
        .fetch(&format!("http://127.0.0.1:{port}/"), 20_000)
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("domain policy"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_domains_missing_from_the_allowlist() {
    let port = serve(vec![("/", ok("text/plain", "secret"))]).await;
    let server = server(DomainPolicy::new("example.com", ""), None);

    let error = server
        .fetch(&format!("http://127.0.0.1:{port}/"), 20_000)
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("domain policy"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_redirects_to_denied_domains() {
    let port = serve(vec![("/bounce", redirect("http://localhost:1/secret"))]).await;
    let server = server(DomainPolicy::new("", "localhost"), None);

    let error = server
        .fetch(&format!("http://127.0.0.1:{port}/bounce"), 20_000)
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("blocked url"), "{error:#}");
}

#[tokio::test]
async fn fetch_follows_redirects_to_allowed_domains() {
    let port = serve(vec![
        ("/bounce", redirect("/final")),
        ("/final", ok("text/plain", "arrived")),
    ])
    .await;
    let server = server(open(), None);

    let text = server
        .fetch(&format!("http://127.0.0.1:{port}/bounce"), 20_000)
        .await
        .unwrap();

    assert_eq!(text, "arrived");
}

fn search_results() -> String {
    ok(
        "application/json",
        r#"{"results": [
            {"title": "First", "url": "https://one.example/a", "content": "the first snippet"},
            {"title": "Hidden", "url": "https://blocked.example/b", "content": "denied"},
            {"url": "https://three.example/c"}
        ]}"#,
    )
}

#[tokio::test]
async fn search_lists_results_and_skips_denied_domains() {
    let port = serve(vec![("/search", search_results())]).await;
    let server = server(
        DomainPolicy::new("", "blocked.example"),
        Some(format!("http://127.0.0.1:{port}/search")),
    );

    let text = server.search("anything", 10).await.unwrap();

    assert_eq!(
        text,
        "1. First\n   https://one.example/a\n   the first snippet\n\n\
         2. https://three.example/c\n   https://three.example/c"
    );
}

#[tokio::test]
async fn search_honours_the_limit() {
    let port = serve(vec![("/search", search_results())]).await;
    let server = server(open(), Some(format!("http://127.0.0.1:{port}/search")));

    let text = server.search("anything", 1).await.unwrap();

    assert!(text.starts_with("1. First"), "{text}");
    assert!(!text.contains("2."), "{text}");
}

#[tokio::test]
async fn search_works_with_a_query_placeholder() {
    let port = serve(vec![("/api", search_results())]).await;
    let server = server(
        open(),
        Some(format!("http://127.0.0.1:{port}/api?term={{query}}")),
    );

    let text = server.search("rust mcp", 1).await.unwrap();

    assert!(text.starts_with("1. First"), "{text}");
}

#[tokio::test]
async fn search_reports_an_empty_result_list() {
    let port = serve(vec![(
        "/search",
        ok("application/json", r#"{"results": []}"#),
    )])
    .await;
    let server = server(open(), Some(format!("http://127.0.0.1:{port}/search")));

    assert_eq!(server.search("nothing", 10).await.unwrap(), "No results.");
}

#[tokio::test]
async fn search_needs_a_configured_engine() {
    let server = server(open(), None);

    let error = server.search("anything", 10).await.unwrap_err();

    assert!(
        format!("{error:#}").contains("MCP_WEB_SEARCH_URL"),
        "{error:#}"
    );
}
