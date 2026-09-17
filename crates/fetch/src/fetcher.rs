use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use mudfish_core::CrawlConfig;
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

/// The rest of the platform talks to `HttpFetcher` (and, eventually, a
/// browser-backed fetcher) only through this shape — nothing downstream
/// should need to know which one produced a given response.
pub struct HttpFetcher {
    client: reqwest::Client,
    max_response_bytes: u64,
    max_retries: u32,
}

impl HttpFetcher {
    pub fn new(config: &CrawlConfig) -> Result<Self, FetchError> {
        let mut builder = reqwest::Client::builder()
            .user_agent(config.user_agent.clone())
            .redirect(reqwest::redirect::Policy::limited(
                config.max_redirects as usize,
            ))
            .timeout(config.request_timeout);
        if !config.allow_private_networks {
            builder = builder.dns_resolver(Arc::new(SsrfGuardResolver));
        }
        let client = builder.build()?;
        Ok(Self {
            client,
            max_response_bytes: config.max_response_bytes,
            max_retries: config.max_retries,
        })
    }

    /// Fetches `url`, retrying transient failures (connect/timeout errors,
    /// HTTP 429, HTTP 5xx) with exponential backoff and jitter. Honors a
    /// `Retry-After` header when present instead of guessing.
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

            let delay = match &result {
                Ok(resp) => resp
                    .header("retry-after")
                    .and_then(parse_retry_after)
                    .unwrap_or_else(|| backoff_delay(attempt + 1)),
                Err(_) => backoff_delay(attempt + 1),
            };
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }

    async fn fetch_once(&self, url: &Url) -> Result<FetchResponse, FetchError> {
        let resp = self.client.get(url.clone()).send().await?;
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
    matches!(err, FetchError::Request(e) if e.is_timeout() || e.is_connect())
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

fn backoff_delay(attempt: u32) -> Duration {
    let base_ms: u64 = 500u64.saturating_mul(1u64 << attempt.min(6));
    let jitter_ms: u64 = rand::random_range(0..250);
    Duration::from_millis((base_ms + jitter_ms).min(30_000))
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
        assert_eq!(parse_retry_after("not-a-number"), None);
    }

    #[test]
    fn backoff_grows_and_caps() {
        let d1 = backoff_delay(1);
        let d4 = backoff_delay(4);
        let d_huge = backoff_delay(20);
        assert!(d1 < d4);
        assert!(d_huge <= Duration::from_millis(30_250));
    }
}
