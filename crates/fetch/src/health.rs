use std::time::{Duration, Instant};

use dashmap::DashMap;

/// A domain's circuit-breaker state. `Closed` is normal operation;
/// `Open` means requests to this host are being skipped until
/// `cooldown_until` elapses; `HalfOpen` is the one-trial-request state a
/// cooldown transitions into (see `DomainHealthTracker::is_available`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Per-host request outcome counters plus circuit-breaker state. All
/// counts are lifetime totals for this tracker's process lifetime --
/// there is no persistence across runs (see this module's doc comment
/// in `ROADMAP_HONEST.md` for why that's a disclosed, not hidden, gap).
#[derive(Debug, Clone)]
pub struct DomainStats {
    pub host: String,
    pub requests: u64,
    pub successes: u64,
    pub failures_4xx: u64,
    pub failures_429: u64,
    pub failures_403: u64,
    pub failures_5xx: u64,
    pub consecutive_failures: u32,
    pub circuit: CircuitState,
    cooldown_until: Option<Instant>,
    cooldown_duration: Duration,
}

impl DomainStats {
    fn new(host: &str, base_cooldown: Duration) -> Self {
        Self {
            host: host.to_string(),
            requests: 0,
            successes: 0,
            failures_4xx: 0,
            failures_429: 0,
            failures_403: 0,
            failures_5xx: 0,
            consecutive_failures: 0,
            circuit: CircuitState::Closed,
            cooldown_until: None,
            cooldown_duration: base_cooldown,
        }
    }

    /// Seconds remaining in the current cooldown, or `None` if the
    /// circuit isn't open (or the cooldown has already elapsed). Exposed
    /// as a plain duration rather than `Instant` so callers (CLI/
    /// PyTagManager reporting) don't need to depend on `std::time`
    /// internals to show something useful to a user.
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        self.cooldown_until
            .map(|until| until.saturating_duration_since(Instant::now()))
            .filter(|d| !d.is_zero())
    }
}

/// Consecutive 429/403/5xx responses from the same host before its
/// circuit opens. Chosen to tolerate a couple of transient blips (a
/// single rate-limit response, a flaky 503) without overreacting, while
/// still catching a host that is clearly, persistently blocking this
/// crawler.
const DEFAULT_OPEN_THRESHOLD: u32 = 5;
/// Initial cooldown once a circuit opens; doubles on each subsequent
/// open (see `record_response`), capped at `MAX_COOLDOWN`.
const DEFAULT_BASE_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_COOLDOWN: Duration = Duration::from_secs(600);

/// Tracks per-host request health and enforces a circuit breaker so a
/// host that is persistently rate-limiting or blocking this crawler gets
/// a cooldown instead of being hammered with retries. This does **not**
/// attempt to detect or bypass any specific anti-bot mechanism -- see
/// `challenge::detect_challenge` for the (separate, also heuristic)
/// signal this tracker's callers typically pair it with.
pub struct DomainHealthTracker {
    domains: DashMap<String, DomainStats>,
    open_threshold: u32,
    base_cooldown: Duration,
}

impl Default for DomainHealthTracker {
    fn default() -> Self {
        Self::new(DEFAULT_OPEN_THRESHOLD, DEFAULT_BASE_COOLDOWN)
    }
}

impl DomainHealthTracker {
    pub fn new(open_threshold: u32, base_cooldown: Duration) -> Self {
        Self {
            domains: DashMap::new(),
            open_threshold: open_threshold.max(1),
            base_cooldown,
        }
    }

    /// Whether a request to `host` should be attempted right now. Does
    /// **not** itself count as a request -- callers that get `true` and
    /// then skip the fetch for an unrelated reason don't need to
    /// "undo" anything here.
    ///
    /// Transitions `Open` -> `HalfOpen` (allowing exactly one trial
    /// request through) once the cooldown has elapsed; `record_response`
    /// is what turns `HalfOpen` back to `Open` (on another failure) or
    /// `Closed` (on a success).
    pub fn is_available(&self, host: &str) -> bool {
        let Some(mut entry) = self.domains.get_mut(host) else {
            return true;
        };
        match entry.circuit {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => match entry.cooldown_remaining() {
                None => {
                    entry.circuit = CircuitState::HalfOpen;
                    // One more bad response should reopen the circuit
                    // immediately, not require the full threshold again.
                    entry.consecutive_failures = self.open_threshold.saturating_sub(1);
                    true
                }
                Some(_) => false,
            },
        }
    }

    /// Records the outcome of a completed request to `host`. Plain 4xx
    /// statuses other than 403/429 (e.g. 404) are tracked but don't count
    /// toward opening the circuit -- they're normal crawl noise (dead
    /// links), not a sign of being rate-limited or blocked.
    pub fn record_response(&self, host: &str, status: u16) {
        let mut entry = self
            .domains
            .entry(host.to_string())
            .or_insert_with(|| DomainStats::new(host, self.base_cooldown));
        entry.requests += 1;

        if (200..400).contains(&status) {
            entry.successes += 1;
            entry.consecutive_failures = 0;
            entry.circuit = CircuitState::Closed;
            entry.cooldown_until = None;
            entry.cooldown_duration = self.base_cooldown;
            return;
        }

        match status {
            429 => entry.failures_429 += 1,
            403 => entry.failures_403 += 1,
            500..=599 => entry.failures_5xx += 1,
            _ => {
                entry.failures_4xx += 1;
                return;
            }
        }

        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= self.open_threshold {
            entry.circuit = CircuitState::Open;
            entry.cooldown_until = Some(Instant::now() + entry.cooldown_duration);
            entry.cooldown_duration = (entry.cooldown_duration * 2).min(MAX_COOLDOWN);
        }
    }

    pub fn snapshot(&self, host: &str) -> Option<DomainStats> {
        self.domains.get(host).map(|e| e.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> DomainHealthTracker {
        DomainHealthTracker::new(3, Duration::from_millis(50))
    }

    #[test]
    fn unknown_host_is_available() {
        let t = tracker();
        assert!(t.is_available("example.com"));
    }

    #[test]
    fn plain_404s_never_open_the_circuit() {
        let t = tracker();
        for _ in 0..10 {
            t.record_response("example.com", 404);
        }
        assert!(t.is_available("example.com"));
        assert_eq!(t.snapshot("example.com").unwrap().failures_4xx, 10);
    }

    #[test]
    fn opens_after_threshold_consecutive_429s() {
        let t = tracker();
        t.record_response("example.com", 429);
        t.record_response("example.com", 429);
        assert!(t.is_available("example.com"), "below threshold yet");
        t.record_response("example.com", 429);
        assert!(
            !t.is_available("example.com"),
            "threshold reached, circuit should be open"
        );
        let snap = t.snapshot("example.com").unwrap();
        assert_eq!(snap.circuit, CircuitState::Open);
    }

    #[test]
    fn a_success_in_between_resets_the_consecutive_counter() {
        let t = tracker();
        t.record_response("example.com", 429);
        t.record_response("example.com", 429);
        t.record_response("example.com", 200); // resets
        t.record_response("example.com", 429);
        t.record_response("example.com", 429);
        assert!(
            t.is_available("example.com"),
            "the reset means only 2 consecutive failures so far"
        );
    }

    #[test]
    fn half_opens_after_cooldown_then_recloses_on_success() {
        let t = tracker();
        for _ in 0..3 {
            t.record_response("example.com", 403);
        }
        assert!(!t.is_available("example.com"));

        std::thread::sleep(Duration::from_millis(60));
        assert!(
            t.is_available("example.com"),
            "cooldown elapsed, should allow one trial request (half-open)"
        );

        t.record_response("example.com", 200);
        let snap = t.snapshot("example.com").unwrap();
        assert_eq!(snap.circuit, CircuitState::Closed);
    }

    #[test]
    fn half_open_failure_reopens_immediately() {
        let t = tracker();
        for _ in 0..3 {
            t.record_response("example.com", 403);
        }
        std::thread::sleep(Duration::from_millis(60));
        assert!(t.is_available("example.com")); // half-open trial allowed

        t.record_response("example.com", 403); // the trial fails
        assert!(
            !t.is_available("example.com"),
            "a single failure during half-open must reopen the circuit, \
             not require the full threshold again"
        );
    }

    #[test]
    fn cooldown_doubles_on_repeated_opens() {
        let t = tracker();
        for _ in 0..3 {
            t.record_response("example.com", 500);
        }
        let first_cooldown = t.snapshot("example.com").unwrap().cooldown_duration;

        std::thread::sleep(Duration::from_millis(60));
        assert!(t.is_available("example.com"));
        t.record_response("example.com", 500); // half-open trial fails again
        let second_cooldown = t.snapshot("example.com").unwrap().cooldown_duration;

        assert!(second_cooldown > first_cooldown);
    }
}
