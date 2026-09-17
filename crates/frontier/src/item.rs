use std::cmp::Ordering;

use url::Url;

/// A URL queued for fetching. Carries enough context (depth, discoverer,
/// priority) that a `PriorityScorer` or downstream consumer can make
/// scheduling decisions without re-deriving it.
#[derive(Debug, Clone)]
pub struct FrontierItem {
    pub url: Url,
    pub depth: usize,
    pub discovered_from: Option<Url>,
    pub priority: i64,
    pub(crate) seq: u64,
}

impl PartialEq for FrontierItem {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}

impl Eq for FrontierItem {}

impl PartialOrd for FrontierItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `BinaryHeap` is a max-heap, so higher `priority` pops first. Among equal
/// priorities, the item with the smaller (earlier) `seq` pops first — this
/// keeps scheduling FIFO within a priority band instead of arbitrary heap
/// order, which would otherwise cause starvation-prone jitter.
impl Ord for FrontierItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}
