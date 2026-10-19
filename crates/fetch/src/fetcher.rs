use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use mudfish_core::CrawlConfig;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use url::Url;

use crate::error::FetchError;
use crate::security::SsrfGuardResolver;

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub url: Url,
    pub final_url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub body: Bytes,
}

impl FetchResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Rotates a set of pre-built clients (one per configured proxy) so a long
/// crawl isn't pinned to a single egress IP. Pure selection logic, no I/O,
/// so it's unit-testable without a live proxy.
struct ProxyPool {
    clients: Vec<reqwest::Client>,
    rotate_after: u32,
    current: AtomicUsize,
    used: AtomicU32,
}

impl ProxyPool {
    fn new(clients: Vec<reqwest::Client>, rotate_after: u32) -> Self {
        Self {
            clients,
            rotate_after: rotate_after.max(1),
            current: AtomicUsize::new(0),
            used: AtomicU32::new(0),
        }
    }

    /// Returns the client for the current proxy, advancing to the next proxy
    /// after `rotate_after` requests have gone through the current one.
    fn select(&self) -> &reqwest::Client {
        let idx = self.current.load(Ordering::Relaxed) % self.clients.len();
        let used = self.used.fetch_add(1, Ordering::Relaxed) + 1;
        if used >= self.rotate_after {
            self.used.store(0, Ordering::Relaxed);
            self.current.fetch_add(1, Ordering::Relaxed);
        }
        &self.clients[idx]
    }

    /// Rotates away from the current proxy immediately (called after a
    /// retryable failure so a dead/ratelimited proxy is abandoned rather
    /// than retried `rotate_after` more times).
    fn mark_failure(&self) {
        self.used.store(0, Ordering::Relaxed);
        self.current.fetch_add(1, Ordering::Relaxed);
    }

    fn index(&self) -> usize {
        self.current.load(Ordering::Relaxed) % self.clients.len()
    }
}

enum ClientSet {
    Single(reqwest::Client),
    Pool(ProxyPool),
}

/// The rest of the platform talks to `HttpFetcher` (and, eventually, a
/// browser-backed fetcher) only through this shape — nothing downstream
/// should need to know which one produced a given response.
pub struct HttpFetcher {
    clients: ClientSet,
    max_response_bytes: u64,
    max_retries: u32,
    retry: RetryPolicy,
    /// Whether the SSRF guard is active. The DNS resolver hook covers
    /// hostnames, but reqwest never calls it for IP-literal hosts, so the
    /// same policy has to be re-applied at the URL level (see
    /// `security::ip_literal_is_blocked`).
    guard_ssrf: bool,
}

/// Jittered exponential backoff. Attempt 1 is `base`, attempt 2 is `2 * base`,
/// and so on up to `max`. Jitter scales each delay by a random factor in
/// `[0.5, 1.0]`, so a batch of workers that all hit a 429/5xx spread out
/// instead of retrying in lockstep. Explicit `Retry-After` headers bypass
/// this entirely (the server's value is honored).
#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    base: Duration,
    max: Duration,
}

impl RetryPolicy {
    fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max: max.max(base),
        }
    }

    fn delay(&self, attempt: u32) -> Duration {
        self.delay_with_factor(attempt, rand::random_range(0.5..=1.0))
    }

    fn delay_with_factor(&self, attempt: u32, factor: f64) -> Duration {
        let shift = attempt.saturating_sub(1).min(16);
        let base_ms = self.base.as_millis().min(u64::MAX as u128) as u64;
        let max_ms = self.max.as_millis().min(u64::MAX as u128) as u64;
        let exp_ms = base_ms.saturating_mul(1u64 << shift).min(max_ms);
        let factor = factor.clamp(0.0, 1.0);
        Duration::from_millis((exp_ms as f64 * factor) as u64)
    }
}

/// Builds one `reqwest::Client`, optionally pinned to a proxy. The SSRF
/// resolver hook and redirect policy are applied identically to every client
/// (direct and proxied) so proxy rotation can't be used to bypass the guard.
fn build_client(config: &CrawlConfig, proxy: Option<&str>) -> Result<reqwest::Client, FetchError> {
    let guard_ssrf = !config.allow_private_networks;
    let max_redirects = config.max_redirects as usize;
    let mut builder = reqwest::Client::builder()
        .user_agent(config.user_agent.clone())
        .timeout(config.request_timeout)
        // Persist Set-Cookie values across requests in one crawler instance,
        // so a login/session page can set a cookie that later pages send back.
        .cookie_store(true);
    if guard_ssrf {
        // Reject IP-literal redirect targets at the URL level too; the
        // resolver hook never sees them. Wrap the normal limit policy so
        // redirect-loop protection is preserved.
        let follow = reqwest::redirect::Policy::limited(max_redirects);
        builder = builder.dns_resolver(Arc::new(SsrfGuardResolver)).redirect(
            reqwest::redirect::Policy::custom(move |attempt| {
                if crate::security::ip_literal_is_blocked(attempt.url()) {
                    attempt.error("redirect target is a blocked IP literal")
                } else {
                    follow.redirect(attempt)
                }
            }),
        );
    } else {
        builder = builder.redirect(reqwest::redirect::Policy::limited(max_redirects));
    }
    if !config.default_headers.is_empty() {
        builder = builder.default_headers(build_header_map(&config.default_headers)?);
    }
    if let Some(raw) = proxy {
        let proxy = reqwest::Proxy::all(raw)
            .map_err(|e| FetchError::PolicyBlocked(format!("invalid proxy '{raw}': {e}")))?;
        builder = builder.proxy(proxy);
    }
    Ok(builder.build()?)
}

impl HttpFetcher {
    pub fn new(config: &CrawlConfig) -> Result<Self, FetchError> {
        let guard_ssrf = !config.allow_private_networks;
        let clients = if config.proxies.is_empty() {
            ClientSet::Single(build_client(config, None)?)
        } else {
            let mut clients = Vec::with_capacity(config.proxies.len());
            for proxy in &config.proxies {
                clients.push(build_client(config, Some(proxy))?);
            }
            ClientSet::Pool(ProxyPool::new(clients, config.proxy_rotate_after))
        };
        Ok(Self {
            clients,
            max_response_bytes: config.max_response_bytes,
            max_retries: config.max_retries,
            retry: RetryPolicy::new(config.retry_base_delay, config.retry_max_delay),
            guard_ssrf,
        })
    }

    /// Index of the proxy currently in use (0 and always 0 when no proxies
    /// are configured). Exposed for observability/tests.
    pub fn current_proxy_index(&self) -> usize {
        match &self.clients {
            ClientSet::Single(_) => 0,
            ClientSet::Pool(pool) => pool.index(),
        }
    }

    /// Fetches `url`, retrying transient failures (connect/timeout errors,
    /// HTTP 429, HTTP 5xx) with exponential backoff and jitter. Honors a
    /// `Retry-After` header when present instead of guessing. When proxies
    /// are configured, a retryable failure also rotates to the next proxy.
    pub async fn fetch(&self, url: &Url) -> Result<FetchResponse, FetchError> {
        let mut attempt: u32 = 0;
        loop {
            let result = self.fetch_once(url).await;
            let should_retry = attempt < self.max_retries
                && match &result {
                    Ok(resp) => is_retryable_status(resp.status),
                    Err(err) => is_retryable_error(err),
                };

            if !should_retry {
                return result;
            }

            if let ClientSet::Pool(pool) = &self.clients {
                pool.mark_failure();
            }

            let delay = match &result {
                Ok(resp) => resp
                    .header("retry-after")
                    .and_then(parse_retry_after)
                    .unwrap_or_else(|| self.retry.delay(attempt + 1)),
                Err(_) => self.retry.delay(attempt + 1),
            };
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }

    async fn fetch_once(&self, url: &Url) -> Result<FetchResponse, FetchError> {
        if self.guard_ssrf && crate::security::ip_literal_is_blocked(url) {
            return Err(FetchError::PolicyBlocked(format!(
                "target host '{}' is a blocked IP literal",
                url.host_str().unwrap_or("<unknown>")
            )));
        }
        let client = match &self.clients {
            ClientSet::Single(client) => client,
            ClientSet::Pool(pool) => pool.select(),
        };
        let resp = client.get(url.clone()).send().await?;
        let status = resp.status().as_u16();
        let final_url = resp.url().clone();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    String::from_utf8_lossy(v.as_bytes()).to_string(),
                )
            })
            .collect();
        let content_type = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone());

        if let Some(len) = resp.content_length() {
            if len > self.max_response_bytes {
                return Err(FetchError::ResponseTooLarge {
                    limit: self.max_response_bytes,
                });
            }
        }

        let body = read_body_capped(resp, self.max_response_bytes).await?;

        Ok(FetchResponse {
            url: url.clone(),
            final_url,
            status,
            headers,
            content_type,
            body,
        })
    }
}

fn build_header_map(headers: &[(String, String)]) -> Result<HeaderMap, FetchError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| FetchError::InvalidHeader(format!("invalid header name '{name}': {e}")))?;
        let header_value = HeaderValue::from_str(value).map_err(|e| {
            FetchError::InvalidHeader(format!("invalid header value for '{name}': {e}"))
        })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

/// Streams the body in chunks and aborts as soon as the cap is exceeded,
/// rather than trusting `Content-Length` (which is absent for chunked
/// responses and easy to lie about) — this is the actual defense against
/// decompression bombs and oversized responses.
async fn read_body_capped(mut resp: reqwest::Response, limit: u64) -> Result<Bytes, FetchError> {
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() as u64 + chunk.len() as u64 > limit {
            return Err(FetchError::ResponseTooLarge { limit });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(buf))
}

fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

fn is_retryable_error(err: &FetchError) -> bool {
    matches!(
        err,
        FetchError::Request(e)
            if e.is_timeout() || e.is_connect() || e.is_body() || e.is_request()
    )
}

/// Parses both forms RFC 9110 allows for `Retry-After`: delta-seconds
/// (`Retry-After: 5`) and an HTTP-date (`Retry-After: Wed, 21 Oct 2015
/// 07:28:00 GMT`). A date in the past means "retry now".
fn parse_retry_after(value: &str) -> Option<Duration> {
    parse_retry_after_at(value, SystemTime::now())
}

fn parse_retry_after_at(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = httpdate::parse_http_date(value).ok()?;
    Some(when.duration_since(now).unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(200));
    }

    #[test]
    fn retry_after_parses_seconds() {
        assert_eq!(parse_retry_after("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("not-a-number"), None);
    }

    #[test]
    fn retry_after_parses_http_date_relative_to_now() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_603_238_400);
        assert_eq!(
            parse_retry_after_at("Wed, 21 Oct 2020 00:00:10 GMT", now),
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn retry_after_date_in_past_means_retry_now() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            parse_retry_after_at("Wed, 21 Oct 2020 00:00:10 GMT", now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let policy = RetryPolicy::new(Duration::from_millis(100), Duration::from_millis(500));
        // Factor 1.0 removes jitter so the schedule is exact.
        assert_eq!(policy.delay_with_factor(1, 1.0), Duration::from_millis(100));
        assert_eq!(policy.delay_with_factor(2, 1.0), Duration::from_millis(200));
        assert_eq!(policy.delay_with_factor(3, 1.0), Duration::from_millis(400));
        assert_eq!(policy.delay_with_factor(4, 1.0), Duration::from_millis(500));
        assert_eq!(
            policy.delay_with_factor(100, 1.0),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn backoff_jitter_stays_within_half_to_full_delay() {
        let policy = RetryPolicy::new(Duration::from_millis(1000), Duration::from_secs(30));
        for attempt in 1..8 {
            for _ in 0..100 {
                let d = policy.delay(attempt);
                let full = policy.delay_with_factor(attempt, 1.0);
                let half = policy.delay_with_factor(attempt, 0.5);
                assert!(d >= half, "jitter below 50%: {d:?} < {half:?}");
                assert!(d <= full, "jitter above 100%: {d:?} > {full:?}");
            }
        }
    }

    #[test]
    fn build_header_map_accepts_valid_headers() {
        let map = build_header_map(&[("Cookie".to_string(), "session=abc".to_string())]).unwrap();
        assert_eq!(map.get("cookie").unwrap(), "session=abc");
    }

    #[test]
    fn build_header_map_rejects_invalid_name() {
        let err = build_header_map(&[("bad header".to_string(), "v".to_string())]).unwrap_err();
        assert!(matches!(err, FetchError::InvalidHeader(_)));
    }

    #[test]
    fn build_header_map_rejects_invalid_value() {
        let err =
            build_header_map(&[("X-Test".to_string(), "bad\nvalue".to_string())]).unwrap_err();
        assert!(matches!(err, FetchError::InvalidHeader(_)));
    }

    fn dummy_clients(n: usize) -> Vec<reqwest::Client> {
        (0..n).map(|_| reqwest::Client::new()).collect()
    }

    #[test]
    fn proxy_pool_rotates_after_configured_requests() {
        let pool = ProxyPool::new(dummy_clients(3), 2);
        assert_eq!(pool.index(), 0);
        pool.select();
        assert_eq!(pool.index(), 0, "still on proxy 0 for the 1st request");
        pool.select();
        assert_eq!(pool.index(), 1, "rotated to proxy 1 after 2 requests");
        pool.select();
        pool.select();
        assert_eq!(pool.index(), 2);
        pool.select();
        pool.select();
        assert_eq!(pool.index(), 0, "wraps back around");
    }

    #[test]
    fn proxy_pool_mark_failure_rotates_immediately() {
        let pool = ProxyPool::new(dummy_clients(2), 100);
        assert_eq!(pool.index(), 0);
        pool.select();
        assert_eq!(pool.index(), 0);
        pool.mark_failure();
        assert_eq!(pool.index(), 1, "failure abandons the current proxy");
    }

    #[test]
    fn direct_fetcher_reports_proxy_index_zero() {
        let config = CrawlConfig::default();
        let fetcher = HttpFetcher::new(&config).unwrap();
        assert_eq!(fetcher.current_proxy_index(), 0);
    }

    #[test]
    fn fetcher_builds_a_pool_from_configured_proxies() {
        let config = CrawlConfig {
            proxies: vec![
                "http://127.0.0.1:8080".to_string(),
                "http://127.0.0.1:8081".to_string(),
            ],
            proxy_rotate_after: 1,
            ..CrawlConfig::default()
        };
        let fetcher = HttpFetcher::new(&config).unwrap();
        assert_eq!(fetcher.current_proxy_index(), 0);
        assert!(matches!(fetcher.clients, ClientSet::Pool(_)));
    }

    #[test]
    fn fetcher_rejects_invalid_proxy_url() {
        let config = CrawlConfig {
            proxies: vec!["not a proxy url".to_string()],
            ..CrawlConfig::default()
        };
        let err = match HttpFetcher::new(&config) {
            Ok(_) => panic!("expected invalid proxy to be rejected"),
            Err(e) => e,
        };
        assert!(matches!(err, FetchError::PolicyBlocked(_)));
    }
}
