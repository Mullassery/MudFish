use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use mudfish_core::{CrawlConfig, CrawlErrorRecord, CrawlResult, CrawlStats, FetchMethod, Page};
use mudfish_fetch::{HttpFetcher, PolitenessManager, RobotsManager};
use mudfish_frontier::{DepthPriority, Frontier};
use mudfish_parser::parse_html;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, warn};
use uuid::Uuid;

/// Updates `lock` to `new_host` if it differs, recovering from poisoning
/// instead of propagating it.
///
/// A single worker panicking while briefly holding this write lock (e.g. a
/// future bug elsewhere in the crawl loop) must not cascade into every
/// other worker panicking on their next `read`/`write` of the resolved
/// scope host — that would take down the whole worker pool over what was,
/// structurally, just a `String` write. The recovered value is safe to
/// keep using: the only mutation this lock ever guards is a whole-`String`
/// replacement, so a panic can't leave it partially written.
fn update_scope_host(lock: &RwLock<String>, new_host: &str) {
    let mut guard = lock
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.as_str() != new_host {
        *guard = new_host.to_string();
    }
}

/// Reads the current scope host, recovering from poisoning instead of
/// propagating it. See `update_scope_host` for why this is safe.
fn read_scope_host(lock: &RwLock<String>) -> String {
    lock.read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

#[derive(Default)]
struct StatsCounter {
    discovered: AtomicU64,
    fetched: AtomicU64,
    skipped: AtomicU64,
    errors: AtomicU64,
    bytes: AtomicU64,
}

/// Runs a crawl to completion and returns the result. This is the shared
/// core consumed by both the CLI (`apps/mudfish`) and the Python bindings
/// (`crates/python`) — neither layer duplicates the orchestration logic.
///
/// Only `config.seeds[0]` is used as the crawl's starting point and as the
/// basis for same-domain scoping; multi-seed crawls across different hosts
/// are not yet supported (both current callers only ever pass one seed).
pub async fn crawl(config: &CrawlConfig) -> anyhow::Result<CrawlResult> {
    let seed = config
        .seeds
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("CrawlConfig must have at least one seed URL"))?;

    // The seed itself may redirect (e.g. `www.example.com` -> `example.com`);
    // same-domain scoping must follow that resolved host, not the literal
    // seed host, or every link on the real page would be misclassified as
    // cross-domain. Safe to update from a single worker with no lock
    // contention concerns: no depth>0 item can exist in the frontier until
    // the seed (depth 0) has been processed once.
    let scope_host = Arc::new(RwLock::new(seed.host_str().unwrap_or_default().to_string()));

    let crawl_id = Uuid::new_v4();
    let started_at = Instant::now();

    let fetcher = Arc::new(HttpFetcher::new(config)?);
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
    let global_sem = Arc::new(Semaphore::new(config.concurrency.max(1)));

    let deadline = config.max_duration.map(|d| started_at + d);
    let max_urls = config.max_urls;

    let mut workers = Vec::with_capacity(config.concurrency.max(1));
    for _ in 0..config.concurrency.max(1) {
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
                                update_scope_host(&scope_host, final_host);
                            }
                        }

                        if item.depth < config.max_depth {
                            let effective_scope_host = read_scope_host(&scope_host);
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

    Ok(CrawlResult {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_host_lock_recovers_from_poisoning_instead_of_panicking() {
        // Simulate a worker panicking while briefly holding the scope_host
        // write lock (e.g. some future bug elsewhere in the crawl loop).
        // Before the fix, `update_scope_host`/`read_scope_host` used
        // `.expect(...)`, so every other worker's next read or write would
        // itself panic on the poisoned `RwLock`, cascading one worker's
        // crash into the whole pool.
        let lock = Arc::new(RwLock::new("example.com".to_string()));

        let lock2 = lock.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = lock2.write().unwrap();
            panic!("simulated worker panic while holding the scope_host lock");
        })
        .join();
        assert!(panicked.is_err(), "the spawned thread should have panicked");

        // These must not panic even though the lock is now poisoned, and
        // must still behave correctly (read-back reflects the write).
        update_scope_host(&lock, "final.example.com");
        assert_eq!(read_scope_host(&lock), "final.example.com");
    }
}
