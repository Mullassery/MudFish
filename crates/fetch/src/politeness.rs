use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Semaphore;

/// Largest single rate-limit wait we'll schedule. This only guards against
/// pathological configured rates (e.g. `1e-9` requests/sec); normal rates
/// produce waits far below it.
const MAX_RATE_WAIT: Duration = Duration::from_secs(86_400);

/// A per-host token bucket. Tokens accrue at `1 / interval` per second up to
/// `burst` at once. We implement it with the standard virtual-time schedule
/// (`next_available`) rather than floating-point token counts, which makes
/// reservations exact and deterministic: each caller is handed the slot it
/// will occupy and only sleeps for the difference.
#[derive(Debug, Clone, Copy)]
struct TokenBucket {
    interval: Duration,
    burst: u32,
    next_available: Instant,
}

impl TokenBucket {
    fn new(interval: Duration, burst: u32, now: Instant) -> Self {
        let burst = burst.max(1);
        // Start with a full bucket: `burst` requests may go immediately.
        let initial_lag = interval.saturating_mul(burst.saturating_sub(1));
        Self {
            interval,
            burst,
            next_available: now.checked_sub(initial_lag).unwrap_or(now),
        }
    }

    /// Reserves the next token and returns how long the caller must wait.
    /// Idle time accrues at most `burst` tokens, so a host that has been
    /// quiet isn't hit by an unbounded burst when work resumes.
    fn reserve(&mut self, now: Instant) -> Duration {
        let max_idle_credit = self.interval.saturating_mul(self.burst.saturating_sub(1));
        if now.saturating_duration_since(self.next_available) > max_idle_credit {
            self.next_available = now.checked_sub(max_idle_credit).unwrap_or(now);
        }
        let slot = self.next_available;
        self.next_available = slot.checked_add(self.interval).unwrap_or(slot);
        slot.saturating_duration_since(now)
    }
}

/// Enforces three independent per-host limits: a minimum delay between
/// requests, a sustained token-bucket rate (with a small burst allowance),
/// and a maximum number of requests in flight at once. All are keyed by host
/// so one slow or strict site never throttles the whole crawl's global
/// concurrency budget.
pub struct PolitenessManager {
    last_request: DashMap<String, Instant>,
    host_semaphores: DashMap<String, Arc<Semaphore>>,
    coalesce: DashMap<String, TokenBucket>,
    delay: Duration,
    per_host_concurrency: usize,
    rate: Option<RateLimit>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimit {
    interval: Duration,
    burst: u32,
}

impl PolitenessManager {
    /// Builds a manager with only the legacy spacing + concurrency limits.
    /// Rate limiting is opt-in through [`Self::with_rate_limit`].
    pub fn new(delay: Duration, per_host_concurrency: usize) -> Self {
        Self {
            last_request: DashMap::new(),
            host_semaphores: DashMap::new(),
            coalesce: DashMap::new(),
            delay,
            per_host_concurrency: per_host_concurrency.max(1),
            rate: None,
        }
    }

    /// Builds a manager with a token-bucket rate limit in addition to the
    /// minimum delay and per-host concurrency cap. `requests_per_sec <= 0`
    /// (or a non-finite value) disables rate limiting; `burst` is clamped to
    /// at least 1.
    pub fn with_rate_limit(
        delay: Duration,
        per_host_concurrency: usize,
        requests_per_sec: f64,
        burst: u32,
    ) -> Self {
        let mut manager = Self::new(delay, per_host_concurrency);
        if requests_per_sec.is_finite() && requests_per_sec > 0.0 {
            let interval_secs = 1.0 / requests_per_sec;
            if interval_secs.is_finite() && interval_secs <= MAX_RATE_WAIT.as_secs_f64() {
                manager.rate = Some(RateLimit {
                    interval: Duration::from_secs_f64(interval_secs),
                    burst: burst.max(1),
                });
            }
        }
        manager
    }

    pub fn host_semaphore(&self, host: &str) -> Arc<Semaphore> {
        self.host_semaphores
            .entry(host.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_host_concurrency)))
            .clone()
    }

    /// Sleeps as needed so consecutive requests to `host` are spaced at
    /// least `delay` apart and conform to the configured token-bucket rate,
    /// then reserves the next slot. Both limits are reserved before sleeping
    /// so concurrent callers queue deterministically instead of all waking
    /// up and racing for the same token.
    pub async fn wait_for_slot(&self, host: &str) {
        if !self.delay.is_zero() {
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

        if let Some(rate) = self.rate {
            let wait = {
                let now = Instant::now();
                let mut bucket = self
                    .coalesce
                    .entry(host.to_string())
                    .or_insert_with(|| TokenBucket::new(rate.interval, rate.burst, now));
                bucket.reserve(now)
            };
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_allows_burst_then_spaces_at_rate() {
        let start = Instant::now();
        let interval = Duration::from_millis(100);
        let mut bucket = TokenBucket::new(interval, 3, start);

        // Full bucket: the first three reservations are immediate.
        assert_eq!(bucket.reserve(start), Duration::ZERO);
        assert_eq!(bucket.reserve(start), Duration::ZERO);
        assert_eq!(bucket.reserve(start), Duration::ZERO);

        // The fourth must wait one interval, the fifth two, and so on.
        assert_eq!(bucket.reserve(start), interval);
        assert_eq!(bucket.reserve(start), interval * 2);
    }

    #[test]
    fn token_bucket_caps_idle_credit_at_burst() {
        let start = Instant::now();
        let interval = Duration::from_millis(100);
        let mut bucket = TokenBucket::new(interval, 2, start);

        // Idle for a long time: only `burst` (2) tokens may be spent at once,
        // not one token per interval of idle time.
        let later = start + Duration::from_secs(10);
        assert_eq!(bucket.reserve(later), Duration::ZERO);
        assert_eq!(bucket.reserve(later), Duration::ZERO);
        assert_eq!(bucket.reserve(later), interval);
    }

    #[test]
    fn token_bucket_reservations_are_ordered_under_concurrency() {
        // Two callers arriving "at the same time" are handed distinct,
        // monotonically increasing slots instead of both sleeping the same
        // interval and then racing.
        let start = Instant::now();
        let interval = Duration::from_millis(50);
        let mut bucket = TokenBucket::new(interval, 1, start);

        assert_eq!(bucket.reserve(start), Duration::ZERO);
        assert_eq!(bucket.reserve(start), interval);
        assert_eq!(bucket.reserve(start), interval * 2);
    }

    #[test]
    fn rate_limit_disabled_for_zero_or_invalid_rates() {
        assert!(
            PolitenessManager::with_rate_limit(Duration::ZERO, 4, 0.0, 2)
                .rate
                .is_none()
        );
        assert!(
            PolitenessManager::with_rate_limit(Duration::ZERO, 4, f64::NAN, 2)
                .rate
                .is_none()
        );
        assert!(
            PolitenessManager::with_rate_limit(Duration::ZERO, 4, -1.0, 2)
                .rate
                .is_none()
        );
    }

    #[test]
    fn per_host_concurrency_cap_is_bounded() {
        let manager = PolitenessManager::new(Duration::ZERO, 2);
        let sem = manager.host_semaphore("example.com");
        let first = sem.clone().try_acquire_owned().unwrap();
        let second = sem.clone().try_acquire_owned().unwrap();
        assert!(
            sem.clone().try_acquire_owned().is_err(),
            "third in-flight request must be refused while the cap is 2"
        );
        drop(first);
        assert!(sem.try_acquire_owned().is_ok());
        drop(second);
    }

    #[test]
    fn hosts_have_independent_concurrency_caps() {
        let manager = PolitenessManager::new(Duration::ZERO, 1);
        let a = manager
            .host_semaphore("a.example")
            .try_acquire_owned()
            .unwrap();
        let b = manager.host_semaphore("b.example");
        assert!(
            b.try_acquire_owned().is_ok(),
            "one host at its cap must not block a different host"
        );
        drop(a);
    }
}
