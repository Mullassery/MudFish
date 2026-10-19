use std::time::Duration;

use mudfish_core::CrawlConfig;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use url::Url;

/// Runs a crawl to completion and returns a Python dict shaped like
/// `mudfish_core::CrawlResult` (crawl_id, pages, errors, stats).
///
/// This is synchronous/blocking from Python's perspective: it spins up its
/// own single-process tokio runtime internally and blocks on it. The GIL is
/// released for the duration (via `Python::detach`) so other Python threads
/// keep running. There is no asyncio integration in this version.
#[pyfunction]
#[pyo3(signature = (
    url,
    depth = 3,
    concurrency = 50,
    per_host_concurrency = 4,
    same_domain = true,
    max_urls = 10_000,
    max_duration_secs = None,
    timeout_secs = 30,
    request_delay_ms = 0,
    per_host_rate = 2.0,
    per_host_burst = 2,
    max_retries = 2,
    retry_base_ms = 500,
    retry_max_ms = 30_000,
    max_response_bytes = 20 * 1024 * 1024,
    respect_robots = true,
    allow_private_networks = false,
    render_js = false,
    render_settle_ms = 2000,
    proxies = None,
    proxy_rotate_after = 50,
    chrome_path = None,
))]
#[allow(clippy::too_many_arguments)]
fn crawl<'py>(
    py: Python<'py>,
    url: String,
    depth: usize,
    concurrency: usize,
    per_host_concurrency: usize,
    same_domain: bool,
    max_urls: u64,
    max_duration_secs: Option<u64>,
    timeout_secs: u64,
    request_delay_ms: u64,
    per_host_rate: f64,
    per_host_burst: u32,
    max_retries: u32,
    retry_base_ms: u64,
    retry_max_ms: u64,
    max_response_bytes: u64,
    respect_robots: bool,
    allow_private_networks: bool,
    render_js: bool,
    render_settle_ms: u64,
    proxies: Option<Vec<String>>,
    proxy_rotate_after: u32,
    chrome_path: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    let seed = Url::parse(&url).map_err(|e| PyValueError::new_err(format!("invalid url: {e}")))?;

    let config = CrawlConfig {
        seeds: vec![seed],
        max_depth: depth,
        concurrency: concurrency.max(1),
        per_host_concurrency: per_host_concurrency.max(1),
        same_domain,
        respect_robots,
        request_delay: Duration::from_millis(request_delay_ms),
        per_host_rate,
        per_host_burst,
        max_retries,
        retry_base_delay: Duration::from_millis(retry_base_ms),
        retry_max_delay: Duration::from_millis(retry_max_ms),
        request_timeout: Duration::from_secs(timeout_secs),
        max_response_bytes,
        allow_private_networks,
        render_js,
        render_settle: Duration::from_millis(render_settle_ms),
        proxies: proxies.unwrap_or_default(),
        proxy_rotate_after,
        chrome_path,
        max_urls: Some(max_urls),
        max_duration: max_duration_secs.map(Duration::from_secs),
        ..CrawlConfig::default()
    };

    let result = py
        .detach(|| {
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            rt.block_on(mudfish_engine::crawl(&config))
                .map_err(|e| e.to_string())
        })
        .map_err(PyRuntimeError::new_err)?;

    Ok(pythonize::pythonize(py, &result)?)
}

/// Extract named fields from HTML using CSS selectors.
///
/// `schema` is a dict of field name -> CSS selector. Returns field name ->
/// list of trimmed text matches.
#[pyfunction]
fn extract_fields(
    html: &str,
    schema: std::collections::BTreeMap<String, String>,
) -> PyResult<std::collections::BTreeMap<String, Vec<String>>> {
    Ok(mudfish_parser::extract_fields(html, &schema))
}

#[pymodule]
fn _mudfish(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(crawl, m)?)?;
    m.add_function(wrap_pyfunction!(extract_fields, m)?)?;
    Ok(())
}
