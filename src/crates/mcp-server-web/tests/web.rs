//! Runs the tools against a small fake web server on localhost.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use mcp_server_web::config::{Config, DomainPolicy};
use mcp_server_web::server::{FetchOutput, Fetched, ImageOutput, Mode, SearchHit, WebServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn response(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    reply
}

fn ok(content_type: &str, body: &str) -> Vec<u8> {
    response("200 OK", content_type, body.as_bytes())
}

fn ok_bytes(content_type: &str, body: &[u8]) -> Vec<u8> {
    response("200 OK", content_type, body)
}

fn redirect(location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

/// Serves canned responses by request path (without the query) and returns
/// the port. Unknown paths get a 404.
async fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> u16 {
    serve_counting(routes).await.0
}

/// Like `serve`, and also counts the requests it receives.
async fn serve_counting(routes: Vec<(&'static str, Vec<u8>)>) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let routes = routes.clone();
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                counter.fetch_add(1, Ordering::SeqCst);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let target = request.split_whitespace().nth(1).unwrap_or("/");
                let path = target.split('?').next().unwrap_or("/");

                let reply = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map(|(_, reply)| reply.clone())
                    .unwrap_or_else(|| response("404 Not Found", "text/plain", b"not found"));
                let _ = stream.write_all(&reply).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    (port, hits)
}

fn config(policy: DomainPolicy, search_url: Option<String>) -> Config {
    Config {
        policy,
        search_url,
        timeout: Duration::from_secs(5),
        max_bytes: 1024 * 1024,
        user_agent: "mcp-server-web-tests".to_owned(),
        max_chars: 20_000,
        cache_ttl: Duration::from_secs(60),
        cache_max_entries: 16,
        max_redirects: 10,
        search_limit: 10,
        max_image_pixels: 1024 * 1024,
    }
}

fn server(policy: DomainPolicy, search_url: Option<String>) -> WebServer {
    WebServer::new(config(policy, search_url)).unwrap()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// The slice of text in a fetch result.
fn slice(fetched: Fetched) -> FetchOutput {
    match fetched {
        Fetched::Text(slice) => slice,
        other => panic!("expected text, got {other:?}"),
    }
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
        .fetch(
            &format!("http://127.0.0.1:{port}/post"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap();

    let text = slice(text).content;
    assert!(text.contains("Test Article"), "{text}");
    assert!(text.contains("Readability keeps the paragraphs"), "{text}");
    assert!(!text.contains("Navigation Junk"), "{text}");
}

#[tokio::test]
async fn fetch_in_full_mode_returns_the_page_as_is() {
    let port = serve(vec![(
        "/post",
        ok("text/html; charset=utf-8", &article_page()),
    )])
    .await;
    let server = server(open(), None);

    let text = server
        .fetch(
            &format!("http://127.0.0.1:{port}/post"),
            Mode::Full,
            0,
            None,
        )
        .await
        .unwrap();

    assert_eq!(slice(text).content, article_page());
}

#[tokio::test]
async fn fetch_returns_json_as_is() {
    let port = serve(vec![("/data", ok("application/json", r#"{"a": 1}"#))]).await;
    let server = server(open(), None);

    for mode in [Mode::Readable, Mode::Full] {
        let text = server
            .fetch(&format!("http://127.0.0.1:{port}/data"), mode, 0, None)
            .await
            .unwrap();

        assert_eq!(
            slice(text),
            FetchOutput {
                offset: 0,
                length: 8,
                next_offset: None,
                content: r#"{"a": 1}"#.to_owned(),
            }
        );
    }
}

#[tokio::test]
async fn fetch_returns_the_requested_slice() {
    let body = (0..100)
        .map(|n| char::from(b'a' + n % 26))
        .collect::<String>();
    let port = serve(vec![("/long", ok("text/plain", &body))]).await;
    let server = server(open(), None);
    let url = format!("http://127.0.0.1:{port}/long");

    let text = server
        .fetch(&url, Mode::Readable, 10, Some(20))
        .await
        .unwrap();
    assert_eq!(
        slice(text),
        FetchOutput {
            offset: 10,
            length: 20,
            next_offset: Some(30),
            content: body[10..30].to_owned(),
        }
    );

    let text = server
        .fetch(&url, Mode::Readable, 90, Some(20))
        .await
        .unwrap();
    assert_eq!(
        slice(text),
        FetchOutput {
            offset: 90,
            length: 10,
            next_offset: None,
            content: body[90..].to_owned(),
        }
    );
}

#[tokio::test]
async fn fetch_slices_as_long_as_the_server_maximum_by_default() {
    let port = serve(vec![("/long", ok("text/plain", &"x".repeat(100)))]).await;
    let server = WebServer::new(Config {
        max_chars: 10,
        ..config(open(), None)
    })
    .unwrap();

    let text = server
        .fetch(
            &format!("http://127.0.0.1:{port}/long"),
            Mode::Full,
            0,
            None,
        )
        .await
        .unwrap();

    let text = slice(text);
    assert_eq!((text.length, text.next_offset), (10, Some(10)));
}

#[tokio::test]
async fn fetch_refuses_a_length_over_the_maximum() {
    let port = serve(vec![("/long", ok("text/plain", &"x".repeat(100)))]).await;
    let server = WebServer::new(Config {
        max_chars: 50,
        ..config(open(), None)
    })
    .unwrap();

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/long"),
            Mode::Full,
            0,
            Some(51),
        )
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("larger than the maximum of 50"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_refuses_an_offset_past_the_end() {
    let port = serve(vec![("/short", ok("text/plain", "abc"))]).await;
    let server = server(open(), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/short"),
            Mode::Full,
            4,
            None,
        )
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("past the end"), "{error:#}");
}

#[tokio::test]
async fn fetch_caches_pages_for_both_modes() {
    let (port, hits) = serve_counting(vec![(
        "/post",
        ok("text/html; charset=utf-8", &article_page()),
    )])
    .await;
    let server = server(open(), None);
    let url = format!("http://127.0.0.1:{port}/post");

    let readable = server.fetch(&url, Mode::Readable, 0, None).await.unwrap();
    let full = server.fetch(&url, Mode::Full, 0, None).await.unwrap();
    let again = server.fetch(&url, Mode::Readable, 0, None).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_ne!(readable, full);
    assert_eq!(readable, again);
}

#[tokio::test]
async fn fetch_downloads_again_after_the_ttl() {
    let (port, hits) = serve_counting(vec![("/page", ok("text/plain", "hello"))]).await;
    let server = WebServer::new(Config {
        cache_ttl: Duration::from_millis(200),
        ..config(open(), None)
    })
    .unwrap();
    let url = format!("http://127.0.0.1:{port}/page");

    server.fetch(&url, Mode::Full, 0, None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    server.fetch(&url, Mode::Full, 0, None).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fetch_does_not_cache_with_a_zero_ttl() {
    let (port, hits) = serve_counting(vec![("/page", ok("text/plain", "hello"))]).await;
    let server = WebServer::new(Config {
        cache_ttl: Duration::ZERO,
        ..config(open(), None)
    })
    .unwrap();
    let url = format!("http://127.0.0.1:{port}/page");

    server.fetch(&url, Mode::Full, 0, None).await.unwrap();
    server.fetch(&url, Mode::Full, 0, None).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fetch_does_not_cache_errors() {
    let (port, hits) = serve_counting(vec![]).await;
    let server = server(open(), None);
    let url = format!("http://127.0.0.1:{port}/missing");

    server.fetch(&url, Mode::Full, 0, None).await.unwrap_err();
    server.fetch(&url, Mode::Full, 0, None).await.unwrap_err();

    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fetch_rejects_binary_content() {
    let port = serve(vec![("/file", ok("application/pdf", "not really a pdf"))]).await;
    let server = server(open(), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/file"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("unsupported content type"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_returns_small_images_untouched() {
    let data = png(10, 10);
    let port = serve(vec![("/logo", ok_bytes("image/png", &data))]).await;
    let server = server(open(), None);

    for mode in [Mode::Readable, Mode::Full] {
        let fetched = server
            .fetch(&format!("http://127.0.0.1:{port}/logo"), mode, 0, None)
            .await
            .unwrap();

        assert_eq!(
            fetched,
            Fetched::Image(ImageOutput {
                content_type: "image/png".to_owned(),
                bytes: data.len(),
                data: STANDARD.encode(&data),
            })
        );
    }
}

#[tokio::test]
async fn fetch_refuses_offset_and_length_for_images() {
    let port = serve(vec![("/logo", ok_bytes("image/png", &png(10, 10)))]).await;
    let server = server(open(), None);
    let url = format!("http://127.0.0.1:{port}/logo");

    for (offset, length) in [(1, None), (0, Some(10))] {
        let error = server
            .fetch(&url, Mode::Full, offset, length)
            .await
            .unwrap_err();

        assert!(
            format!("{error:#}").contains("do not apply to images"),
            "{error:#}"
        );
    }
}

#[tokio::test]
async fn fetch_scales_big_images_down_to_jpeg() {
    let port = serve(vec![("/big", ok_bytes("image/png", &png(400, 200)))]).await;
    let server = WebServer::new(Config {
        max_image_pixels: 20_000,
        ..config(open(), None)
    })
    .unwrap();

    let fetched = server
        .fetch(&format!("http://127.0.0.1:{port}/big"), Mode::Full, 0, None)
        .await
        .unwrap();

    let image = match fetched {
        Fetched::Image(image) => image,
        other => panic!("expected an image, got {other:?}"),
    };
    assert_eq!(image.content_type, "image/jpeg");
    let jpeg = STANDARD.decode(&image.data).unwrap();
    assert_eq!(image.bytes, jpeg.len());
    let decoded = image::load_from_memory(&jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (200, 100));
}

#[tokio::test]
async fn fetch_refuses_broken_images() {
    let port = serve(vec![("/broken", ok_bytes("image/png", b"abc"))]).await;
    let server = server(open(), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/broken"),
            Mode::Full,
            0,
            None,
        )
        .await
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("failed to process the image"),
        "{error:#}"
    );
}

#[tokio::test]
async fn fetch_downloads_a_page_once_for_parallel_calls() {
    let (port, hits) = serve_counting(vec![("/page", ok("text/plain", &"x".repeat(100)))]).await;
    let server = server(open(), None);
    let url = format!("http://127.0.0.1:{port}/page");

    let (first, second, third) = tokio::join!(
        server.fetch(&url, Mode::Full, 0, Some(10)),
        server.fetch(&url, Mode::Full, 10, Some(10)),
        server.fetch(&url, Mode::Full, 20, Some(10)),
    );

    assert_eq!(slice(first.unwrap()).next_offset, Some(10));
    assert_eq!(slice(second.unwrap()).next_offset, Some(20));
    assert_eq!(slice(third.unwrap()).next_offset, Some(30));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fetch_reports_http_errors() {
    let port = serve(vec![]).await;
    let server = server(open(), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/missing"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("404"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_denied_domains() {
    let port = serve(vec![("/", ok("text/plain", "secret"))]).await;
    let server = server(DomainPolicy::new("", "127.0.0.1"), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("domain policy"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_domains_missing_from_the_allowlist() {
    let port = serve(vec![("/", ok("text/plain", "secret"))]).await;
    let server = server(DomainPolicy::new("example.com", ""), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("domain policy"), "{error:#}");
}

#[tokio::test]
async fn fetch_refuses_redirects_to_denied_domains() {
    let port = serve(vec![("/bounce", redirect("http://localhost:1/secret"))]).await;
    let server = server(DomainPolicy::new("", "localhost"), None);

    let error = server
        .fetch(
            &format!("http://127.0.0.1:{port}/bounce"),
            Mode::Readable,
            0,
            None,
        )
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
        .fetch(
            &format!("http://127.0.0.1:{port}/bounce"),
            Mode::Readable,
            0,
            None,
        )
        .await
        .unwrap();

    assert_eq!(slice(text).content, "arrived");
}

#[tokio::test]
async fn fetch_gives_up_after_the_maximum_number_of_redirects() {
    let port = serve(vec![
        ("/one", redirect("/two")),
        ("/two", redirect("/three")),
        ("/three", ok("text/plain", "arrived")),
    ])
    .await;
    let url = format!("http://127.0.0.1:{port}/one");

    let strict = WebServer::new(Config {
        max_redirects: 1,
        ..config(open(), None)
    })
    .unwrap();
    let error = strict.fetch(&url, Mode::Full, 0, None).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("too many redirects"),
        "{error:#}"
    );

    let relaxed = WebServer::new(Config {
        max_redirects: 2,
        ..config(open(), None)
    })
    .unwrap();
    let text = relaxed.fetch(&url, Mode::Full, 0, None).await.unwrap();
    assert_eq!(slice(text).content, "arrived");
}

fn search_results() -> Vec<u8> {
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

    let output = server.search("anything", 10).await.unwrap();

    assert_eq!(
        output.results,
        vec![
            SearchHit {
                title: Some("First".to_owned()),
                url: "https://one.example/a".to_owned(),
                snippet: Some("the first snippet".to_owned()),
            },
            SearchHit {
                title: None,
                url: "https://three.example/c".to_owned(),
                snippet: None,
            },
        ]
    );
}

#[tokio::test]
async fn search_honours_the_limit() {
    let port = serve(vec![("/search", search_results())]).await;
    let server = server(open(), Some(format!("http://127.0.0.1:{port}/search")));

    let output = server.search("anything", 1).await.unwrap();

    assert_eq!(output.results.len(), 1);
    assert_eq!(output.results[0].title.as_deref(), Some("First"));
}

#[tokio::test]
async fn search_works_with_a_query_placeholder() {
    let port = serve(vec![("/api", search_results())]).await;
    let server = server(
        open(),
        Some(format!("http://127.0.0.1:{port}/api?term={{query}}")),
    );

    let output = server.search("rust mcp", 1).await.unwrap();

    assert_eq!(output.results[0].title.as_deref(), Some("First"));
}

#[tokio::test]
async fn search_reports_an_empty_result_list() {
    let port = serve(vec![(
        "/search",
        ok("application/json", r#"{"results": []}"#),
    )])
    .await;
    let server = server(open(), Some(format!("http://127.0.0.1:{port}/search")));

    assert!(
        server
            .search("nothing", 10)
            .await
            .unwrap()
            .results
            .is_empty()
    );
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
