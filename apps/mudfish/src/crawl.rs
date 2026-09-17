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
        request_timeout: Duration::from_secs(args.timeout_secs),
        max_response_bytes: args.max_response_bytes,
        allow_private_networks: args.allow_private_networks,
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
