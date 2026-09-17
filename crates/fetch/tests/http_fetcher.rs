use std::time::Duration;

use mudfish_core::CrawlConfig;
use mudfish_fetch::{HttpFetcher, PolitenessManager, RobotsManager};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config() -> CrawlConfig {
    CrawlConfig {
        request_timeout: Duration::from_secs(5),
        max_retries: 2,
        // wiremock binds to 127.0.0.1, which the SSRF guard blocks by
        // design — these are trusted local test fixtures, so opt out.
        allow_private_networks: true,
        ..CrawlConfig::default()
    }
}

#[tokio::test]
async fn fetches_basic_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hello"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello world"))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let url = Url::parse(&format!("{}/hello", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.as_ref(), b"hello world");
}

#[tokio::test]
async fn retries_on_503_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/flaky"))
        .respond_with(ResponseTemplate::new(200).set_body_string("recovered"))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let url = Url::parse(&format!("{}/flaky", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.as_ref(), b"recovered");
}

#[tokio::test]
async fn gives_up_after_max_retries_on_persistent_500() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/broken"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let url = Url::parse(&format!("{}/broken", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    // Retries exhausted: the caller still gets the last response back
    // (not an error), since a persistent 5xx is a valid, informative result.
    assert_eq!(resp.status, 500);
}

#[tokio::test]
async fn response_over_max_size_is_rejected() {
    let server = MockServer::start().await;
    let big_body = vec![b'x'; 2048];
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(big_body))
        .mount(&server)
        .await;

    let config = CrawlConfig {
        max_response_bytes: 1024,
        ..test_config()
    };
    let fetcher = HttpFetcher::new(&config).unwrap();
    let url = Url::parse(&format!("{}/big", server.uri())).unwrap();

    let err = fetcher.fetch(&url).await.unwrap_err();
    assert!(matches!(
        err,
        mudfish_fetch::FetchError::ResponseTooLarge { .. }
    ));
}

#[tokio::test]
async fn robots_disallow_blocks_matching_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("User-agent: *\nDisallow: /private\n"),
        )
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let robots = RobotsManager::new("MudfishCrawlerTest");

    let allowed_url = Url::parse(&format!("{}/public", server.uri())).unwrap();
    let denied_url = Url::parse(&format!("{}/private/secret", server.uri())).unwrap();

    assert!(robots.is_allowed(&fetcher, &allowed_url).await);
    assert!(!robots.is_allowed(&fetcher, &denied_url).await);
}

#[tokio::test]
async fn missing_robots_txt_allows_everything() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let robots = RobotsManager::new("MudfishCrawlerTest");
    let url = Url::parse(&format!("{}/anything", server.uri())).unwrap();

    assert!(robots.is_allowed(&fetcher, &url).await);
}

#[tokio::test]
async fn politeness_manager_spaces_out_requests() {
    let politeness = PolitenessManager::new(Duration::from_millis(100), 4);
    let start = std::time::Instant::now();
    politeness.wait_for_slot("example.com").await;
    politeness.wait_for_slot("example.com").await;
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(90));
}
