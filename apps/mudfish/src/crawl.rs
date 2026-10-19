use std::time::Duration;

use mudfish_core::{CrawlConfig, CrawlResult};
use url::Url;

use crate::cli::CrawlArgs;

fn build_config(args: &CrawlArgs) -> anyhow::Result<CrawlConfig> {
    let seed = Url::parse(&args.url)?;
    Ok(CrawlConfig {
        seeds: vec![seed],
        max_depth: args.depth,
        concurrency: args.concurrency.max(1),
        per_host_concurrency: args.per_host_concurrency.max(1),
        same_domain: !args.allow_cross_domain,
        respect_robots: !args.no_robots,
        request_delay: Duration::from_millis(args.request_delay_ms),
        per_host_rate: args.per_host_rate,
        per_host_burst: args.per_host_burst,
        max_retries: args.max_retries,
        retry_base_delay: Duration::from_millis(args.retry_base_ms),
        retry_max_delay: Duration::from_millis(args.retry_max_ms),
        request_timeout: Duration::from_secs(args.timeout_secs),
        max_response_bytes: args.max_response_bytes,
        allow_private_networks: args.allow_private_networks,
        render_js: args.render_js,
        render_settle: Duration::from_millis(args.render_settle_ms),
        proxies: args.proxy.clone(),
        proxy_rotate_after: args.proxy_rotate_after,
        chrome_path: args
            .chrome_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        frontier_state: args.frontier_state.clone(),
        max_urls: Some(args.max_urls),
        max_duration: args.max_duration_secs.map(Duration::from_secs),
        ..CrawlConfig::default()
    })
}

/// Runs a crawl to completion and returns the result without emitting any
/// output — the testable core. `run` (below) is the CLI-facing wrapper that
/// also writes the result out in the requested format.
pub async fn execute(args: &CrawlArgs) -> anyhow::Result<CrawlResult> {
    let config = build_config(args)?;
    mudfish_engine::crawl(&config).await
}

pub async fn run(args: CrawlArgs) -> anyhow::Result<()> {
    let result = execute(&args).await?;
    crate::output::emit(&result, args.output, args.out_file.as_deref())?;
    Ok(())
}
