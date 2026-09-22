use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

use dashmap::DashSet;
use mudfish_core::{fingerprint, normalize_url, NormalizationOptions};
use tokio::sync::Notify;
use url::Url;

use crate::item::FrontierItem;
use crate::scheduler::PriorityScorer;

/// The URL frontier: a priority queue plus URL-level dedup and a termination
/// signal for a pool of concurrent workers.
///
/// # Termination protocol
/// `pending` counts URLs that are enqueued but not yet fully processed.
/// `try_push` increments it; `complete` decrements it. A caller MUST push
/// any children discovered while processing an item *before* calling
/// `complete` for that item — otherwise a worker could observe
/// `pending == 0` and shut down while a sibling worker is still about to
/// enqueue more work.
pub struct Frontier {
    queue: Mutex<BinaryHeap<FrontierItem>>,
    visited: DashSet<u64>,
    pending: AtomicI64,
    seq: AtomicU64,
    notify: Notify,
    stopped: AtomicBool,
    scorer: Box<dyn PriorityScorer>,
    normalization: NormalizationOptions,
}

impl Frontier {
    pub fn new(scorer: Box<dyn PriorityScorer>, normalization: NormalizationOptions) -> Self {
        Self {
            queue: Mutex::new(BinaryHeap::new()),
            visited: DashSet::new(),
            pending: AtomicI64::new(0),
            seq: AtomicU64::new(0),
            notify: Notify::new(),
            stopped: AtomicBool::new(false),
            scorer,
            normalization,
        }
    }

    /// Normalizes and fingerprints `url`; if it has not been seen before,
    /// enqueues it and returns `true`. Returns `false` for duplicates
    /// (already seen, whether or not it was ever successfully fetched).
    pub fn try_push(&self, url: Url, depth: usize, discovered_from: Option<Url>) -> bool {
        let normalized = normalize_url(&url, &self.normalization);
        let fp = fingerprint(&normalized);
        if !self.visited.insert(fp) {
            return false;
        }

        let priority = self.scorer.score(&normalized, depth);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let item = FrontierItem {
            url: normalized,
            depth,
            discovered_from,
            priority,
            seq,
        };

        self.pending.fetch_add(1, Ordering::SeqCst);
        // Recover from poisoning rather than propagate it: a worker
        // panicking while briefly holding this lock (e.g. a future bug in
        // `PriorityScorer`) must not cascade into every other worker
        // panicking on their next queue access. `BinaryHeap::push`/`pop`
        // can't leave the heap invariant broken by a panic mid-call (the
        // panic would have to originate in `Ord`, which `FrontierItem`
        // doesn't implement in a way that can panic), so the recovered
        // data is safe to keep using as-is.
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(item);
        self.notify.notify_one();
        true
    }

    /// Waits for and returns the next item to process, or `None` once the
    /// frontier is fully drained (empty queue, zero pending work) or
    /// `stop()` has been called — either is the signal for a worker to exit.
    pub async fn pop(&self) -> Option<FrontierItem> {
        loop {
            // Register interest before checking state, per tokio::sync::Notify's
            // documented safe pattern, so a push racing with this check can't
            // produce a missed wakeup.
            let notified = self.notify.notified();

            if self.stopped.load(Ordering::SeqCst) {
                return None;
            }

            if let Some(item) = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop()
            {
                return Some(item);
            }

            if self.pending.load(Ordering::SeqCst) == 0 {
                return None;
            }

            notified.await;
        }
    }

    /// Hard-stops the frontier: every worker currently blocked in `pop`
    /// wakes immediately and receives `None`, and all future `pop` calls
    /// do too. Used to enforce budgets (`max_urls`, `max_duration`) where
    /// waiting for organic drain would otherwise hang workers that have
    /// nothing left to do but whose siblings are still marked pending.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Marks a previously popped item as fully processed. Must be called
    /// exactly once per item returned by `pop`, and only after any children
    /// it discovered have already been pushed.
    pub fn complete(&self, _item: &FrontierItem) {
        let remaining = self.pending.fetch_sub(1, Ordering::SeqCst) - 1;
        if remaining == 0 {
            self.notify.notify_waiters();
        }
    }

    pub fn queued_len(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn pending(&self) -> i64 {
        self.pending.load(Ordering::SeqCst)
    }

    pub fn visited_count(&self) -> usize {
        self.visited.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::DepthPriority;
    use std::sync::Arc;
    use std::time::Duration;

    fn frontier() -> Frontier {
        Frontier::new(Box::new(DepthPriority), NormalizationOptions::default())
    }

    #[test]
    fn duplicate_urls_are_rejected() {
        let f = frontier();
        let u = Url::parse("https://example.com/a").unwrap();
        assert!(f.try_push(u.clone(), 0, None));
        assert!(!f.try_push(u, 0, None));
        assert_eq!(f.queued_len(), 1);
    }

    #[test]
    fn normalization_collapses_dedup_key() {
        let f = frontier();
        let a = Url::parse("https://example.com/a?utm_source=x").unwrap();
        let b = Url::parse("https://example.com/a").unwrap();
        assert!(f.try_push(a, 0, None));
        assert!(!f.try_push(b, 0, None));
    }

    #[test]
    fn shallower_depth_pops_first() {
        let f = frontier();
        f.try_push(Url::parse("https://example.com/deep").unwrap(), 3, None);
        f.try_push(Url::parse("https://example.com/shallow").unwrap(), 0, None);
        // both counted as pending; pop synchronously via block_on-free path
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let first = rt.block_on(f.pop()).unwrap();
        assert_eq!(first.url.path(), "/shallow");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn workers_terminate_when_drained() {
        let f = Arc::new(frontier());
        f.try_push(Url::parse("https://example.com/").unwrap(), 0, None);

        let mut handles = Vec::new();
        for _ in 0..4 {
            let f = f.clone();
            handles.push(tokio::spawn(async move {
                let mut processed = 0;
                while let Some(item) = f.pop().await {
                    // simulate discovering one child, then completing.
                    if item.depth < 2 {
                        // Named per-depth so relative-resolution semantics
                        // (join replaces the last path segment) can't
                        // accidentally collide with an ancestor's URL.
                        let child = item.url.join(&format!("child-{}", item.depth + 1)).unwrap();
                        f.try_push(child, item.depth + 1, Some(item.url.clone()));
                    }
                    processed += 1;
                    f.complete(&item);
                }
                processed
            }));
        }

        let results = futures_join_all(handles).await;
        let total: i32 = results.into_iter().sum();
        assert_eq!(total, 3); // depth 0 -> 1 -> 2, then stops
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stop_wakes_blocked_workers_immediately() {
        // One item pending forever (never completed) means `pending` never
        // reaches zero, so organic drain would never happen. Workers must
        // rely on `stop()` to wake up instead of hanging.
        let f = Arc::new(frontier());
        f.try_push(
            Url::parse("https://example.com/never-completed").unwrap(),
            0,
            None,
        );
        let _first = f.pop().await.unwrap(); // leave it in-flight, uncompleted

        let mut handles = Vec::new();
        for _ in 0..3 {
            let f = f.clone();
            handles.push(tokio::spawn(async move { f.pop().await }));
        }

        // Give the blocked workers a moment to actually reach the await point.
        tokio::time::sleep(Duration::from_millis(20)).await;
        f.stop();

        for h in handles {
            assert!(h.await.unwrap().is_none());
        }
        assert!(f.is_stopped());
    }

    #[test]
    fn poisoned_queue_lock_is_recovered_not_propagated() {
        // Simulate a worker panicking while holding the frontier's internal
        // queue lock (e.g. some future bug elsewhere in the crawl loop).
        // Before the fix, every subsequent `try_push`/`pop`/`queued_len`
        // call would itself panic on the poisoned `Mutex`, cascading one
        // worker's crash into the whole pool. After the fix, the lock is
        // recovered via `into_inner()` and the frontier keeps working.
        let f = Arc::new(frontier());
        f.try_push(Url::parse("https://example.com/a").unwrap(), 0, None);

        let f2 = f.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = f2.queue.lock().unwrap();
            panic!("simulated worker panic while holding the frontier queue lock");
        })
        .join();
        assert!(panicked.is_err(), "the spawned thread should have panicked");

        // These must not panic even though the lock is now poisoned.
        assert_eq!(f.queued_len(), 1);
        assert!(f.try_push(Url::parse("https://example.com/b").unwrap(), 0, None));
        assert_eq!(f.queued_len(), 2);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let popped = rt.block_on(f.pop());
        assert!(popped.is_some());
    }

    // Minimal join_all so we don't pull in futures crate just for tests.
    async fn futures_join_all(handles: Vec<tokio::task::JoinHandle<i32>>) -> Vec<i32> {
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            out.push(h.await.unwrap());
        }
        out
    }
}
