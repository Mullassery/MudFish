use url::Url;

/// Scoring is pluggable so callers can bias crawl order by anything they can
/// compute from the URL and depth alone (path shape, sitemap priority,
/// user-supplied rules, ...). The frontier itself never hard-codes FIFO.
pub trait PriorityScorer: Send + Sync {
    fn score(&self, url: &Url, depth: usize) -> i64;
}

/// Default scorer: prefer shallower pages, approximating breadth-first
/// traversal. `BinaryHeap` pops the highest score first, so depth is
/// negated — depth 0 (seeds) outranks depth 5.
#[derive(Debug, Default, Clone, Copy)]
pub struct DepthPriority;

impl PriorityScorer for DepthPriority {
    fn score(&self, _url: &Url, depth: usize) -> i64 {
        -(depth as i64)
    }
}
