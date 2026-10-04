/// A heuristic signal that a response came from an anti-bot mechanism
/// rather than the site's real content. This module only *detects* and
/// *labels* that condition for reporting/backoff purposes -- it never
/// attempts to solve, bypass, or circumvent whatever produced it. The
/// correct response to any of these is to back off (see
/// `crate::health::DomainHealthTracker`), not to retry harder or switch
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeSignal {
    /// HTTP 429, or a body matching a known rate-limit message.
    RateLimited,
    /// HTTP 403, or a body matching a known "access denied" message.
    Blocked,
    /// A 2xx/anything-else response whose body matches a known
    /// interstitial-challenge pattern (e.g. a CAPTCHA or JS challenge
    /// page served with status 200).
    ChallengePage,
}

/// Body substrings (checked case-insensitively, on at most the first 4KB
/// -- these markers always appear near the top of a real challenge page,
/// and bounding the scan keeps this cheap on large legitimate pages)
/// that are reasonably specific to known anti-bot challenge/interstitial
/// pages. This is a heuristic allowlist of observed real-world phrasing,
/// not a comprehensive or guaranteed-accurate detector: a site can phrase
/// its block page differently and go undetected (a false negative,
/// which just means no signal -- safe), or a legitimate page could in
/// principle contain one of these phrases in unrelated prose (a false
/// positive, which only affects reporting/backoff, never correctness of
/// the crawl's own requests).
const CHALLENGE_BODY_MARKERS: &[&str] = &[
    "checking your browser",
    "cf-chl",
    "captcha",
    "just a moment",
    "access denied",
    "unusual traffic",
    "please verify you are a human",
];

const MAX_SCANNED_BYTES: usize = 4096;

/// Classifies a completed HTTP response as a possible anti-bot challenge
/// signal. `None` means no signal was detected -- most responses,
/// including ordinary error pages, return `None`.
pub fn detect_challenge(status: u16, body: &[u8]) -> Option<ChallengeSignal> {
    if status == 429 {
        return Some(ChallengeSignal::RateLimited);
    }
    if status == 403 {
        return Some(ChallengeSignal::Blocked);
    }

    let scan_len = body.len().min(MAX_SCANNED_BYTES);
    let snippet = String::from_utf8_lossy(&body[..scan_len]).to_lowercase();
    if CHALLENGE_BODY_MARKERS
        .iter()
        .any(|marker| snippet.contains(marker))
    {
        return Some(ChallengeSignal::ChallengePage);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_429_is_rate_limited_regardless_of_body() {
        assert_eq!(
            detect_challenge(429, b"anything"),
            Some(ChallengeSignal::RateLimited)
        );
    }

    #[test]
    fn status_403_is_blocked_regardless_of_body() {
        assert_eq!(
            detect_challenge(403, b"anything"),
            Some(ChallengeSignal::Blocked)
        );
    }

    #[test]
    fn status_200_with_challenge_marker_is_flagged() {
        let body = b"<html><body>Checking your browser before accessing...</body></html>";
        assert_eq!(
            detect_challenge(200, body),
            Some(ChallengeSignal::ChallengePage)
        );
    }

    #[test]
    fn ordinary_200_page_is_not_flagged() {
        let body = b"<html><body><h1>Welcome</h1><p>Normal content.</p></body></html>";
        assert_eq!(detect_challenge(200, body), None);
    }

    #[test]
    fn ordinary_404_is_not_flagged() {
        let body = b"<html><body>Page not found</body></html>";
        assert_eq!(detect_challenge(404, body), None);
    }

    #[test]
    fn marker_far_beyond_the_scan_window_is_not_detected() {
        let mut body = vec![b'x'; MAX_SCANNED_BYTES + 100];
        body.extend_from_slice(b"captcha");
        assert_eq!(detect_challenge(200, &body), None);
    }
}
