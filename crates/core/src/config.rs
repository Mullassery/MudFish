use std::time::Duration;

use url::Url;

use crate::normalize::NormalizationOptions;

/// Every crawl runs under an explicit budget. There is no unbounded mode:
/// unset optional limits (e.g. `max_urls: None`) mean "no limit", chosen
/// deliberately by the caller, not an accidental default.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    pub seeds: Vec<Url>,
    pub max_depth: usize,
    pub concurrency: usize,
    pub per_host_concurrency: usize,
    pub same_domain: bool,
    pub allow_hosts: Vec<String>,
    pub deny_hosts: Vec<String>,
    pub allow_path_patterns: Vec<String>,
    pub deny_path_patterns: Vec<String>,
    pub request_timeout: Duration,
    pub max_retries: u32,
    pub max_redirects: u32,
    pub max_response_bytes: u64,
    pub max_urls: Option<u64>,
    pub max_duration: Option<Duration>,
    pub user_agent: String,
    pub respect_robots: bool,
    pub request_delay: Duration,
    pub normalization: NormalizationOptions,
    /// When `false` (the default), resolved IPs in loopback/private/
    /// link-local/multicast space are refused (SSRF protection — see
    /// `mudfish_fetch::security`). Set `true` only for deliberate crawls of
    /// internal infrastructure or local test fixtures; never flip this on
    /// for crawls of arbitrary/untrusted URLs.
    pub allow_private_networks: bool,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            seeds: Vec::new(),
            max_depth: 3,
            concurrency: 50,
            per_host_concurrency: 4,
            same_domain: true,
            allow_hosts: Vec::new(),
            deny_hosts: Vec::new(),
            allow_path_patterns: Vec::new(),
            deny_path_patterns: Vec::new(),
            request_timeout: Duration::from_secs(30),
            max_retries: 2,
            max_redirects: 10,
            max_response_bytes: 20 * 1024 * 1024,
            max_urls: Some(10_000),
            max_duration: None,
            user_agent: format!(
                "MudfishCrawler/{} (+https://github.com/Mullassery/MudFish)",
                env!("CARGO_PKG_VERSION")
            ),
            respect_robots: true,
            request_delay: Duration::from_millis(0),
            normalization: NormalizationOptions::default(),
            allow_private_networks: false,
        }
    }
}
