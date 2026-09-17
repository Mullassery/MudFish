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
    max_response_bytes = 20 * 1024 * 1024,
    respect_robots = true,
    allow_private_networks = false,
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
    max_response_bytes: u64,
    respect_robots: bool,
    allow_private_networks: bool,
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
        request_timeout: Duration::from_secs(timeout_secs),
        max_response_bytes,
        allow_private_networks,
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

#[pymodule]
fn _mudfish(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(crawl, m)?)?;
    Ok(())
}
