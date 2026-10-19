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
    /// Base delay for the first retry after a 429/5xx/transport error.
    /// Subsequent retries double the delay (exponential backoff) up to
    /// `retry_max_delay`, with jitter applied so concurrent workers don't
    /// retry in lockstep. A server-provided `Retry-After` always wins over
    /// the computed backoff.
    pub retry_base_delay: Duration,
    /// Ceiling for computed exponential backoff (does not cap an explicit
    /// `Retry-After`, which is honored as sent by the server).
    pub retry_max_delay: Duration,
    pub max_redirects: u32,
    pub max_response_bytes: u64,
    pub max_urls: Option<u64>,
    pub max_duration: Option<Duration>,
    pub user_agent: String,
    pub respect_robots: bool,
    pub request_delay: Duration,
    /// Sustained per-host request rate (requests per second) enforced by a
    /// token bucket. `0` disables rate limiting. Applies on top of
    /// `per_host_concurrency` (in-flight cap) and `request_delay` (minimum
    /// spacing), so a host sees at most `per_host_burst` immediate requests
    /// followed by this rate.
    pub per_host_rate: f64,
    /// Token-bucket burst capacity, i.e. how many requests to one host may
    /// start back-to-back before the sustained `per_host_rate` takes over.
    pub per_host_burst: u32,
    /// Extra headers sent with every request (e.g. `Cookie`/`Authorization`
    /// for crawling pages that require auth). Empty by default, matching
    /// plain anonymous crawling.
    pub default_headers: Vec<(String, String)>,
    pub normalization: NormalizationOptions,
    /// When `false` (the default), resolved IPs in loopback/private/
    /// link-local/multicast space are refused (SSRF protection — see
    /// `mudfish_fetch::security`). Set `true` only for deliberate crawls of
    /// internal infrastructure or local test fixtures; never flip this on
    /// for crawls of arbitrary/untrusted URLs.
    pub allow_private_networks: bool,
    /// When `true`, a page whose static HTML looks JavaScript-dependent (see
    /// `mudfish_parser::static_html_looks_js_dependent`) is re-fetched through
    /// a real headless Chrome instance and parsed from the rendered DOM
    /// instead. Off by default: a browser render is far heavier than a plain
    /// HTTP fetch, and requires a Chrome/Chromium binary on the host.
    pub render_js: bool,
    /// How long a rendered page is allowed to settle (network idle wait)
    /// before its DOM is captured, when `render_js` is enabled.
    pub render_settle: Duration,
    /// Ordered list of proxy URLs (e.g. `http://user:pass@host:port`). Empty
    /// by default, meaning every request goes direct. When non-empty, the
    /// fetcher builds one HTTP client per proxy and rotates through them so a
    /// long crawl isn't pinned to a single egress IP.
    pub proxies: Vec<String>,
    /// Number of requests sent through a proxy before rotating to the next
    /// one. Ignored when `proxies` is empty. A retryable failure also forces
    /// an immediate rotation.
    pub proxy_rotate_after: u32,
    /// Explicit path to the Chrome/Chromium (or `chrome-headless-shell`)
    /// binary to drive when `render_js` is set. `None` (the default) uses
    /// chromiumoxide's host autodetection (including `$CHROME`). Bundled
    /// distributions point this at the binary shipped inside the package.
    pub chrome_path: Option<String>,
    /// Optional SQLite path for a persistent frontier. When set, queued URLs
    /// and the seen-set survive process restarts; in-flight leases from a
    /// crash are requeued on startup. `None` keeps the in-memory frontier.
    pub frontier_state: Option<std::path::PathBuf>,
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
            retry_base_delay: Duration::from_millis(500),
            retry_max_delay: Duration::from_secs(30),
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
            per_host_rate: 2.0,
            per_host_burst: 2,
            default_headers: Vec::new(),
            normalization: NormalizationOptions::default(),
            allow_private_networks: false,
            render_js: false,
            render_settle: Duration::from_secs(2),
            proxies: Vec::new(),
            proxy_rotate_after: 50,
            chrome_path: None,
            frontier_state: None,
        }
    }
}
