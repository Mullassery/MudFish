use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Semaphore;

/// Enforces two independent per-host limits: a minimum delay between
/// requests, and a maximum number of requests in flight at once. Both are
/// keyed by host so one slow or strict site never throttles the whole
/// crawl's global concurrency budget.
pub struct PolitenessManager {
    last_request: DashMap<String, Instant>,
    host_semaphores: DashMap<String, Arc<Semaphore>>,
    delay: Duration,
    per_host_concurrency: usize,
}

impl PolitenessManager {
    pub fn new(delay: Duration, per_host_concurrency: usize) -> Self {
        Self {
            last_request: DashMap::new(),
            host_semaphores: DashMap::new(),
            delay,
            per_host_concurrency: per_host_concurrency.max(1),
        }
    }

    pub fn host_semaphore(&self, host: &str) -> Arc<Semaphore> {
        self.host_semaphores
            .entry(host.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_host_concurrency)))
            .clone()
    }

    /// Sleeps as needed so consecutive requests to `host` are spaced at
    /// least `delay` apart, then reserves the next slot.
    pub async fn wait_for_slot(&self, host: &str) {
        if self.delay.is_zero() {
            return;
        }
        let now = Instant::now();
        let wait_until = {
            let mut entry = self
                .last_request
                .entry(host.to_string())
                .or_insert(now - self.delay);
            let next_allowed = (*entry + self.delay).max(now);
            *entry = next_allowed;
            next_allowed
        };
        if wait_until > now {
            tokio::time::sleep(wait_until - now).await;
        }
    }
}
