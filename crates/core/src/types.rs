use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchMethod {
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UrlStatus {
    Pending,
    Fetching,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub url: Url,
    pub anchor_text: Option<String>,
    pub rel: Option<String>,
}

/// A URL awaiting or undergoing processing in the frontier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlRecord {
    pub url: Url,
    pub depth: usize,
    pub discovered_from: Option<Url>,
    pub priority: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageMetadata {
    pub title: Option<String>,
    pub meta_description: Option<String>,
    pub canonical: Option<Url>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub url: Url,
    pub final_url: Url,
    pub status_code: u16,
    pub content_type: Option<String>,
    pub depth: usize,
    pub metadata: PageMetadata,
    pub links: Vec<Link>,
    pub body_bytes: u64,
    pub fetched_via: FetchMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlErrorRecord {
    pub url: Url,
    pub message: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CrawlStats {
    pub urls_discovered: u64,
    pub urls_fetched: u64,
    pub urls_skipped: u64,
    pub errors: u64,
    pub bytes_downloaded: u64,
    pub duration_ms: u64,
}

impl CrawlStats {
    pub fn throughput_per_sec(&self) -> f64 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        self.urls_fetched as f64 / (self.duration_ms as f64 / 1000.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    pub crawl_id: Uuid,
    pub pages: Vec<Page>,
    pub errors: Vec<CrawlErrorRecord>,
    pub stats: CrawlStats,
}
