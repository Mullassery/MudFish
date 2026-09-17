use mudfish::cli::{CrawlArgs, OutputFormat};
use mudfish::crawl;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn args_for(url: String) -> CrawlArgs {
    CrawlArgs {
        url,
        depth: 2,
        concurrency: 10,
        per_host_concurrency: 4,
        allow_cross_domain: false,
        max_urls: 1_000,
        max_duration_secs: Some(20),
        timeout_secs: 5,
        request_delay_ms: 0,
        max_response_bytes: 20 * 1024 * 1024,
        no_robots: false,
        // wiremock binds to 127.0.0.1, which the crawler's SSRF guard
        // blocks by design; these are trusted local test fixtures.
        allow_private_networks: true,
        output: OutputFormat::Json,
        out_file: None,
    }
}

// `ResponseTemplate::set_body_string` hard-codes Content-Type to
// text/plain (wiremock rebuilds the header from its internal `mime` field
// at response-build time, so `insert_header` alone can't override it) —
// which the crawler correctly refuses to parse as HTML. `set_body_raw` is
// wiremock's actual API for controlling the mime type.
fn html_response(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/html; charset=utf-8")
}

const HOME_PAGE: &str = r#"
    <html><head><title>Home</title></head>
    <body>
        <a href="/about">About</a>
        <a href="/contact">Contact</a>
        <a href="https://not-this-site.example/other">External</a>
    </body></html>
"#;

const ABOUT_PAGE: &str = r#"
    <html><head><title>About</title></head>
    <body><a href="/">Home</a><a href="/deep">Deep</a></body></html>
"#;

#[tokio::test]
async fn follows_same_domain_links_up_to_depth() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(html_response(HOME_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/about"))
        .respond_with(html_response(ABOUT_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contact"))
        .respond_with(html_response("<html><body>contact</body></html>"))
        .mount(&server)
        .await;
    // /deep is at depth 2 from home (home -> about -> deep) and should NOT
    // be fetched when --depth 1.
    Mock::given(method("GET"))
        .and(path("/deep"))
        .respond_with(html_response("<html><body>too deep</body></html>"))
        .mount(&server)
        .await;

    let mut args = args_for(server.uri());
    args.depth = 1;
    let result = crawl::execute(&args).await.unwrap();

    let urls: Vec<String> = result
        .pages
        .iter()
        .map(|p| p.url.path().to_string())
        .collect();
    assert!(urls.contains(&"/".to_string()));
    assert!(urls.contains(&"/about".to_string()));
    assert!(urls.contains(&"/contact".to_string()));
    assert!(
        !urls.contains(&"/deep".to_string()),
        "depth limit should have excluded /deep, got {urls:?}"
    );
    assert!(
        !result
            .pages
            .iter()
            .any(|p| p.url.host_str() == Some("not-this-site.example")),
        "same-domain filtering should have excluded the external link"
    );
    assert_eq!(result.stats.urls_fetched, 3);
    assert_eq!(result.stats.errors, 0);
}

#[tokio::test]
async fn same_domain_scoping_follows_seed_redirect() {
    // Regression test: the seed itself redirects (standing in for a
    // www -> apex redirect). Links on the destination page must still be
    // treated as same-domain relative to where the seed actually landed,
    // not the literal URL typed in — this was a real bug caught by manual
    // testing against rust-lang.org (www.rust-lang.org redirects to the
    // apex domain, which silently zeroed out every discovered link).
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/old-home"))
        .respond_with(
            ResponseTemplate::new(301).insert_header("Location", format!("{}/", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(html_response(HOME_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/about"))
        .respond_with(html_response(ABOUT_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contact"))
        .respond_with(html_response("<html><body>contact</body></html>"))
        .mount(&server)
        .await;

    let mut args = args_for(format!("{}/old-home", server.uri()));
    args.depth = 1;
    let result = crawl::execute(&args).await.unwrap();

    assert_eq!(
        result.stats.urls_fetched,
        3,
        "expected the redirected home page and its two same-site links to be fetched, got pages: {:?}",
        result
            .pages
            .iter()
            .map(|p| p.url.to_string())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn dedups_links_reachable_by_multiple_paths() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(html_response(HOME_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/about"))
        .respond_with(html_response(ABOUT_PAGE))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/contact"))
        .respond_with(html_response("<html><body>contact</body></html>"))
        .mount(&server)
        .await;

    let mut args = args_for(server.uri());
    args.depth = 1;
    let result = crawl::execute(&args).await.unwrap();

    // "/" is linked from both the seed's own normalization and /about's
    // "Home" link; it must only be fetched once.
    let home_fetches = result.pages.iter().filter(|p| p.url.path() == "/").count();
    assert_eq!(home_fetches, 1);
}
