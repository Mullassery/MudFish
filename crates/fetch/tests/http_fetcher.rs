use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mudfish_core::CrawlConfig;
use mudfish_fetch::{HttpFetcher, PolitenessManager, RobotsManager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config() -> CrawlConfig {
    CrawlConfig {
        request_timeout: Duration::from_secs(5),
        max_retries: 2,
        // Keep retry tests fast: the backoff schedule itself is unit-tested
        // with an injected factor, so integration tests only need to prove
        // retries happen and succeed/fail at the right time.
        retry_base_delay: Duration::from_millis(1),
        retry_max_delay: Duration::from_millis(5),
        // Rate limiting has dedicated deterministic tests below; existing
        // fetcher tests should not be paced.
        per_host_rate: 0.0,
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
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "one failed attempt plus one successful retry"
    );
}

#[tokio::test]
async fn retries_on_429_honoring_retry_after_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rate-limited"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rate-limited"))
        .respond_with(ResponseTemplate::new(200).set_body_string("after backoff"))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let url = Url::parse(&format!("{}/rate-limited", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.as_ref(), b"after backoff");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "a 429 with Retry-After must be retried exactly once here"
    );
}

#[tokio::test]
async fn retry_after_zero_does_not_fall_back_to_exponential_backoff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/retry-now"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/retry-now"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    // Use a deliberately long computed backoff; an honored Retry-After: 0
    // means the retry still happens immediately rather than sleeping 5s.
    let config = CrawlConfig {
        retry_base_delay: Duration::from_secs(5),
        retry_max_delay: Duration::from_secs(5),
        ..test_config()
    };
    let fetcher = HttpFetcher::new(&config).unwrap();
    let url = Url::parse(&format!("{}/retry-now", server.uri())).unwrap();
    let started = std::time::Instant::now();
    let resp = fetcher.fetch(&url).await.unwrap();

    assert_eq!(resp.status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "Retry-After: 0 must not be replaced with the 5s exponential backoff"
    );
}

#[tokio::test]
async fn gives_up_after_max_retries_on_persistent_500() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/broken"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let config = CrawlConfig {
        max_retries: 3,
        ..test_config()
    };
    let fetcher = HttpFetcher::new(&config).unwrap();
    let url = Url::parse(&format!("{}/broken", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    // Retries exhausted: the caller still gets the last response back
    // (not an error), since a persistent 5xx is a valid, informative result.
    assert_eq!(resp.status, 500);
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        4,
        "max_retries=3 means one initial attempt plus three retries"
    );
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
async fn default_headers_are_sent_with_every_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secret"))
        .and(header("cookie", "session=abc123"))
        .and(header("x-api-key", "s3cr3t"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let config = CrawlConfig {
        default_headers: vec![
            ("Cookie".to_string(), "session=abc123".to_string()),
            ("X-Api-Key".to_string(), "s3cr3t".to_string()),
        ],
        ..test_config()
    };
    let fetcher = HttpFetcher::new(&config).unwrap();
    let url = Url::parse(&format!("{}/secret", server.uri())).unwrap();
    let resp = fetcher.fetch(&url).await.unwrap();

    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn invalid_default_header_name_fails_fetcher_construction() {
    let config = CrawlConfig {
        default_headers: vec![("bad header".to_string(), "v".to_string())],
        ..test_config()
    };
    match HttpFetcher::new(&config) {
        Err(mudfish_fetch::FetchError::InvalidHeader(_)) => {}
        other => panic!("expected InvalidHeader, got {}", describe(other)),
    }
}

fn describe(result: Result<HttpFetcher, mudfish_fetch::FetchError>) -> String {
    match result {
        Ok(_) => "Ok(HttpFetcher)".to_string(),
        Err(e) => format!("Err({e})"),
    }
}

#[tokio::test]
async fn blocks_ip_literal_hosts_without_touching_network() {
    // Default config has `allow_private_networks: false`, so the SSRF guard
    // is active. These URLs are rejected by the URL-level pre-check before
    // any connection is attempted (nothing listens on these addresses).
    let fetcher = HttpFetcher::new(&CrawlConfig::default()).unwrap();
    for raw in [
        "http://127.0.0.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]/",
    ] {
        let url = Url::parse(raw).unwrap();
        let err = fetcher.fetch(&url).await.unwrap_err();
        assert!(
            matches!(err, mudfish_fetch::FetchError::PolicyBlocked(_)),
            "expected PolicyBlocked for {raw}, got {err}"
        );
    }
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

#[tokio::test]
async fn politeness_rate_limit_paces_concurrent_callers() {
    // 50 req/s with burst 1: the first call is immediate, each subsequent
    // reservation is ~20ms later. Small intervals keep this deterministic
    // without a long sleep.
    let politeness = PolitenessManager::with_rate_limit(Duration::ZERO, 8, 50.0, 1);
    let start = std::time::Instant::now();
    for _ in 0..4 {
        politeness.wait_for_slot("rate.example").await;
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(50),
        "4 requests at 50/s must take at least ~3 intervals (60ms), got {elapsed:?}"
    );
}

struct TrackingServer {
    base_url: String,
    requests: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl TrackingServer {
    async fn start(response_delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));

        let handle = {
            let requests = requests.clone();
            let in_flight = in_flight.clone();
            let max_in_flight = max_in_flight.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let requests = requests.clone();
                    let in_flight = in_flight.clone();
                    let max_in_flight = max_in_flight.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        let mut request = Vec::new();
                        loop {
                            match stream.read(&mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    request.extend_from_slice(&buf[..n]);
                                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                            }
                        }

                        let active = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        max_in_flight.fetch_max(active, Ordering::SeqCst);
                        requests.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(response_delay).await;
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                            )
                            .await;
                        let _ = stream.shutdown().await;
                        in_flight.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            })
        };

        Self {
            base_url,
            requests,
            max_in_flight,
            handle,
        }
    }
}

impl Drop for TrackingServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[tokio::test]
async fn per_host_concurrency_cap_holds_under_real_requests() {
    let server = TrackingServer::start(Duration::from_millis(20)).await;
    let manager = Arc::new(PolitenessManager::new(Duration::ZERO, 2));
    let host = Url::parse(&server.base_url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let sem = manager.host_semaphore(&host);
        let url = format!("{}/", server.base_url);
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.unwrap();
            reqwest::get(&url).await.unwrap().text().await.unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), "ok");
    }

    assert_eq!(server.requests.load(Ordering::SeqCst), 8);
    let max = server.max_in_flight.load(Ordering::SeqCst);
    assert!(max <= 2, "per-host concurrency cap exceeded: {max} > 2");
    assert!(max >= 1, "server observed no in-flight requests");
}

#[tokio::test]
async fn persists_set_cookie_across_requests() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/login"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "session=abc123; Path=/")
                .set_body_string("logged in"),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/account"))
        .and(header("cookie", "session=abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("account"))
        .mount(&server)
        .await;

    let fetcher = HttpFetcher::new(&test_config()).unwrap();
    let login = Url::parse(&format!("{}/login", server.uri())).unwrap();
    let account = Url::parse(&format!("{}/account", server.uri())).unwrap();

    assert_eq!(fetcher.fetch(&login).await.unwrap().status, 200);
    let response = fetcher.fetch(&account).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body.as_ref(), b"account");
}
