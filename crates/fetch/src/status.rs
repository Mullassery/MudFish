use crate::challenge::ChallengeSignal;

/// A coarse, user-facing classification of how a crawled page's fetch
/// went. Built for callers (PyTagManager's reporting, Mudfish's own CLI
/// summary) that want to show something more meaningful than a raw HTTP
/// status code, without needing to know about `ChallengeSignal`/
/// `DomainHealthTracker` internals themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrawlStatus {
    Success,
    Throttled,
    Blocked,
    Challenged,
    Failed,
}

impl std::fmt::Display for CrawlStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CrawlStatus::Success => "success",
            CrawlStatus::Throttled => "throttled",
            CrawlStatus::Blocked => "blocked",
            CrawlStatus::Challenged => "challenged",
            CrawlStatus::Failed => "failed",
        };
        f.write_str(s)
    }
}

/// Classifies a *completed* fetch (one that got an HTTP response at all)
/// into a `CrawlStatus`. `fetch_failed` (network/timeout/DNS error, never
/// got a response) always wins -- call this with `fetch_failed: true` and
/// no further status/challenge args in that case instead of trying to
/// force a status code through.
pub fn classify_status(status: u16, challenge: Option<ChallengeSignal>) -> CrawlStatus {
    match challenge {
        Some(ChallengeSignal::RateLimited) => CrawlStatus::Throttled,
        Some(ChallengeSignal::Blocked) => CrawlStatus::Blocked,
        Some(ChallengeSignal::ChallengePage) => CrawlStatus::Challenged,
        None if (200..400).contains(&status) => CrawlStatus::Success,
        None => CrawlStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_status_with_no_challenge_signal() {
        assert_eq!(classify_status(200, None), CrawlStatus::Success);
    }

    #[test]
    fn redirect_status_counts_as_success() {
        assert_eq!(classify_status(301, None), CrawlStatus::Success);
    }

    #[test]
    fn rate_limited_signal_wins_over_status() {
        assert_eq!(
            classify_status(200, Some(ChallengeSignal::RateLimited)),
            CrawlStatus::Throttled
        );
    }

    #[test]
    fn plain_server_error_with_no_challenge_signal_is_failed() {
        assert_eq!(classify_status(500, None), CrawlStatus::Failed);
    }

    #[test]
    fn plain_404_with_no_challenge_signal_is_failed() {
        // Not "blocked"/"challenged" -- just an ordinary fetch failure
        // from PyTagManager's/the CLI's reporting point of view.
        assert_eq!(classify_status(404, None), CrawlStatus::Failed);
    }
}
