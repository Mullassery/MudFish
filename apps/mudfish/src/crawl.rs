use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mudfish_core::{CrawlConfig, CrawlErrorRecord, CrawlResult, CrawlStats, FetchMethod, Page};
use mudfish_fetch::{HttpFetcher, PolitenessManager, RobotsManager};
use mudfish_frontier::{DepthPriority, Frontier};
use mudfish_parser::parse_html;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use crate::cli::CrawlArgs;

#[derive(Default)]
struct StatsCounter {
    discovered: AtomicU64,
    fetched: AtomicU64,
    skipped: AtomicU64,
    errors: AtomicU64,
    bytes: AtomicU64,
}

/// Runs a crawl to completion and returns the result without emitting any
/// output — the testable core. `run` (below) is the CLI-facing wrapper that
/// also writes the result out in the requested format.
pub async fn execute(args: &CrawlArgs) -> anyhow::Result<CrawlResult> {
    let seed = Url::parse(&args.url)?;
    // The seed itself may redirect (e.g. `www.example.com` -> `example.com`);
    // same-domain scoping must follow that resolved host, not the literal
    // host typed on the command line, or every link on the real page would
    // be misclassified as cross-domain. Safe to update from a single
    // worker with no lock contention concerns: no depth>0 item can exist
    // in the frontier until the seed (depth 0) has been processed once.
    let scope_host = Arc::new(std::sync::RwLock::new(
        seed.host_str().unwrap_or_default().to_string(),
    ));

    let config = CrawlConfig {
        seeds: vec![seed.clone()],
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
    };

    let crawl_id = Uuid::new_v4();
    let started_at = Instant::now();

    let fetcher = Arc::new(HttpFetcher::new(&config)?);
    let robots = Arc::new(RobotsManager::new(config.user_agent.clone()));
    let politeness = Arc::new(PolitenessManager::new(
        config.request_delay,
        config.per_host_concurrency,
    ));
    let frontier = Arc::new(Frontier::new(
        Box::new(DepthPriority),
        config.normalization.clone(),
    ));

    frontier.try_push(seed.clone(), 0, None);

    let pages = Arc::new(Mutex::new(Vec::<Page>::new()));
    let errors = Arc::new(Mutex::new(Vec::<CrawlErrorRecord>::new()));
    let stats = Arc::new(StatsCounter::default());
    let global_sem = Arc::new(Semaphore::new(config.concurrency));

    let deadline = config.max_duration.map(|d| started_at + d);
    let max_urls = config.max_urls;

    let mut workers = Vec::with_capacity(config.concurrency);
    for _ in 0..config.concurrency {
        let frontier = frontier.clone();
        let fetcher = fetcher.clone();
        let robots = robots.clone();
        let politeness = politeness.clone();
        let pages = pages.clone();
        let errors = errors.clone();
        let stats = stats.clone();
        let config = config.clone();
        let scope_host = scope_host.clone();
        let global_sem = global_sem.clone();

        workers.push(tokio::spawn(async move {
            loop {
                if let Some(deadline) = deadline {
                    if Instant::now() >= deadline {
                        frontier.stop();
                        break;
                    }
                }
                if let Some(max) = max_urls {
                    if stats.fetched.load(Ordering::Relaxed) >= max {
                        frontier.stop();
                        break;
                    }
                }

                let item = match frontier.pop().await {
                    Some(item) => item,
                    None => break,
                };

                let _global_permit = global_sem.acquire().await.expect("semaphore not closed");
                let host = item.url.host_str().unwrap_or("").to_string();

                if config.respect_robots && !robots.is_allowed(&fetcher, &item.url).await {
                    stats.skipped.fetch_add(1, Ordering::Relaxed);
                    frontier.complete(&item);
                    continue;
                }

                let host_sem = politeness.host_semaphore(&host);
                let _host_permit = host_sem.acquire().await.expect("semaphore not closed");
                politeness.wait_for_slot(&host).await;

                match fetcher.fetch(&item.url).await {
                    Ok(resp) => {
                        stats.fetched.fetch_add(1, Ordering::Relaxed);
                        stats
                            .bytes
                            .fetch_add(resp.body.len() as u64, Ordering::Relaxed);

                        let is_html = resp
                            .content_type
                            .as_deref()
                            .map(|ct| ct.contains("text/html"))
                            .unwrap_or(false);

                        let (metadata, links) = if is_html {
                            let body_str = String::from_utf8_lossy(&resp.body).into_owned();
                            let parsed = parse_html(&body_str, &resp.final_url);
                            (parsed.metadata, parsed.links)
                        } else {
                            Default::default()
                        };

                        if item.depth == 0 {
                            if let Some(final_host) = resp.final_url.host_str() {
                                let mut guard =
                                    scope_host.write().expect("scope_host lock not poisoned");
                                if guard.as_str() != final_host {
                                    *guard = final_host.to_string();
                                }
                            }
                        }

                        if item.depth < config.max_depth {
                            let effective_scope_host = scope_host
                                .read()
                                .expect("scope_host lock not poisoned")
                                .clone();
                            for link in &links {
                                if config.same_domain
                                    && link.url.host_str() != Some(effective_scope_host.as_str())
                                {
                                    continue;
                                }
                                if frontier.try_push(
                                    link.url.clone(),
                                    item.depth + 1,
                                    Some(item.url.clone()),
                                ) {
                                    stats.discovered.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }

                        debug!(url = %item.url, status = resp.status, "fetched");

                        let page = Page {
                            url: item.url.clone(),
                            final_url: resp.final_url.clone(),
                            status_code: resp.status,
                            content_type: resp.content_type.clone(),
                            depth: item.depth,
                            metadata,
                            links,
                            body_bytes: resp.body.len() as u64,
                            fetched_via: FetchMethod::Http,
                        };
                        pages.lock().await.push(page);
                    }
                    Err(err) => {
                        warn!(url = %item.url, error = %err, "fetch failed");
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        errors.lock().await.push(CrawlErrorRecord {
                            url: item.url.clone(),
                            message: err.to_string(),
                        });
                    }
                }

                frontier.complete(&item);
            }
        }));
    }

    for w in workers {
        let _ = w.await;
    }

    let duration = started_at.elapsed();
    let pages = Arc::try_unwrap(pages)
        .expect("all workers joined")
        .into_inner();
    let errors = Arc::try_unwrap(errors)
        .expect("all workers joined")
        .into_inner();

    let result = CrawlResult {
        crawl_id,
        stats: CrawlStats {
            urls_discovered: stats.discovered.load(Ordering::Relaxed) + 1, // +1 for the seed
            urls_fetched: stats.fetched.load(Ordering::Relaxed),
            urls_skipped: stats.skipped.load(Ordering::Relaxed),
            errors: stats.errors.load(Ordering::Relaxed),
            bytes_downloaded: stats.bytes.load(Ordering::Relaxed),
            duration_ms: duration.as_millis() as u64,
        },
        pages,
        errors,
    };

    Ok(result)
}

pub async fn run(args: CrawlArgs) -> anyhow::Result<()> {
    let result = execute(&args).await?;
    crate::output::emit(&result, args.output, args.out_file.as_deref())?;
    Ok(())
}
