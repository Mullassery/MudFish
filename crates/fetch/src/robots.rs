use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use texting_robots::Robot;
use url::Url;

use crate::fetcher::HttpFetcher;

struct CachedRobots {
    /// `None` means "no restrictions apply" (robots.txt absent or 4xx).
    robot: Option<Arc<Robot>>,
    fetched_at: Instant,
}

/// Fetches, parses, and caches `robots.txt` per host, applying Google's
/// documented status-code handling: 2xx parses the file, 4xx means no
/// restrictions, and unreachable/5xx conservatively assumes deny-all until
/// the next TTL-driven refetch (rather than crawling a site whose crawl
/// policy we couldn't actually read).
pub struct RobotsManager {
    cache: DashMap<String, CachedRobots>,
    ttl: Duration,
    user_agent: String,
}

impl RobotsManager {
    pub fn new(user_agent: impl Into<String>) -> Self {
        Self {
            cache: DashMap::new(),
            ttl: Duration::from_secs(3600),
            user_agent: user_agent.into(),
        }
    }

    pub async fn is_allowed(&self, fetcher: &HttpFetcher, url: &Url) -> bool {
        let host_key = url.origin().ascii_serialization();

        if let Some(entry) = self.cache.get(&host_key) {
            if entry.fetched_at.elapsed() < self.ttl {
                return entry
                    .robot
                    .as_ref()
                    .map(|r| r.allowed(url.as_str()))
                    .unwrap_or(true);
            }
        }

        let robot = self.fetch_and_parse(fetcher, &host_key).await;
        let allowed = robot
            .as_ref()
            .map(|r| r.allowed(url.as_str()))
            .unwrap_or(true);
        self.cache.insert(
            host_key,
            CachedRobots {
                robot,
                fetched_at: Instant::now(),
            },
        );
        allowed
    }

    async fn fetch_and_parse(&self, fetcher: &HttpFetcher, host_key: &str) -> Option<Arc<Robot>> {
        let robots_url = Url::parse(&format!("{host_key}/robots.txt")).ok()?;
        match fetcher.fetch(&robots_url).await {
            Ok(resp) if (200..300).contains(&resp.status) => {
                Robot::new(&self.user_agent, &resp.body).ok().map(Arc::new)
            }
            Ok(resp) if (400..500).contains(&resp.status) => None,
            _ => Robot::new(&self.user_agent, b"User-agent: *\nDisallow: /")
                .ok()
                .map(Arc::new),
        }
    }
}
