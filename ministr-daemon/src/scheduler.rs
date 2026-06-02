//! Ingestion scheduler — bounded concurrency + per-corpus exclusion.
//!
//! Replaces the daemon-wide `INDEXING_SEMAPHORE(1)` band-aid (which serialized
//! *all* indexing to one corpus at a time). That band-aid existed because
//! embedding ran synchronously on Tokio workers and N concurrent corpora could
//! starve the runtime. With the dedicated [`EmbeddingService`] (ADR 0001 D1)
//! embedding is off-runtime, `SQLite` already uses `spawn_blocking` + a connection
//! pool, and parse runs on a rayon pool — so the runtime-starvation root cause
//! is gone and indexing can safely run N corpora concurrently.
//!
//! Raising concurrency above 1 introduces one new hazard the serial band-aid
//! masked: a corpus could be indexed **concurrently with itself** (e.g. a
//! `register` racing its own file-watcher, or two watcher bursts). That would
//! tear `SQLite` + vector state. [`IngestionScheduler`] prevents it with a
//! **per-corpus async mutex** (the same lock-of-locks idiom the registry already
//! uses for restore), so same-corpus indexing serializes (and is never dropped —
//! a queued reindex still runs after the in-flight one), while **different**
//! corpora run concurrently up to a global bound.
//!
//! [`EmbeddingService`]: ministr_core::embedding::EmbeddingService
//!
//! Out of scope (a follow-up): an explicit job queue with priority ordering
//! (small repos before huge vendored trees) and a distinct Queued-vs-Indexing
//! status surfaced to the UI. This module delivers the safety-critical core:
//! bounded concurrency + per-corpus exclusion.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

/// Owns the daemon-wide indexing concurrency policy.
pub struct IngestionScheduler {
    /// Global bound on how many corpora index at once.
    permits: Arc<Semaphore>,
    /// Per-corpus locks (lock-of-locks): the outer mutex guards the map; each
    /// value is the corpus's own lock that serializes its indexing.
    per_corpus: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    max_concurrency: usize,
}

/// An acquired indexing slot. Holds the corpus's exclusive lock **and** a global
/// concurrency permit until dropped — so the corpus can't index concurrently
/// with itself and the global bound is respected. Must be held for the whole
/// ingest.
#[must_use = "dropping the slot immediately releases the indexing permit + corpus lock"]
pub struct IndexSlot {
    // Field order matters for drop order: the global permit is released first,
    // then the per-corpus lock — so a waiter for the same corpus only proceeds
    // once the slot is fully released.
    _global: OwnedSemaphorePermit,
    _corpus: OwnedMutexGuard<()>,
}

impl IngestionScheduler {
    /// Create a scheduler with an explicit global concurrency bound (clamped to
    /// at least 1).
    #[must_use]
    pub fn new(max_concurrency: usize) -> Self {
        let n = max_concurrency.max(1);
        Self {
            permits: Arc::new(Semaphore::new(n)),
            per_corpus: Mutex::new(HashMap::new()),
            max_concurrency: n,
        }
    }

    /// Create a scheduler with the default bound: `available_parallelism`
    /// clamped to `2..=4`. The GPU is serialized by the single shared
    /// [`EmbeddingService`](ministr_core::embedding::EmbeddingService) regardless
    /// of `N`, so this bound governs how many corpora overlap their parse / I/O /
    /// `SQLite` work; it is kept conservative because in-flight parse trees are the
    /// memory-pressure source (a hard memory ceiling is the governance chunk's
    /// job).
    #[must_use]
    pub fn with_default_concurrency() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        Self::new(cores.clamp(2, 4))
    }

    /// The global concurrency bound.
    #[must_use]
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Try to claim a global indexing permit **without** waiting. Returns
    /// `None` immediately when the bound is already saturated.
    ///
    /// This is the non-blocking counterpart to [`Self::acquire`]'s global
    /// half, used by the [`IngestionCoordinator`](crate::coordinator::IngestionCoordinator)
    /// to drive event-driven dispatch: it owns the per-corpus exclusion itself
    /// (a busy-set, so it never blocks on a busy corpus) and only needs the
    /// scheduler to enforce the global bound. The returned permit is held by
    /// the spawned job and released on drop, exactly as the global half of an
    /// [`IndexSlot`] is.
    #[must_use]
    pub fn try_acquire_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.permits.clone().try_acquire_owned().ok()
    }

    /// Look up (or create) the per-corpus lock. Brief acquisition of the outer
    /// map mutex to get/insert; returns the inner `Arc<Mutex<()>>` so the caller
    /// holds the per-corpus lock — not the map — across the ingest.
    async fn corpus_lock(&self, corpus_id: &str) -> Arc<Mutex<()>> {
        let mut map = self.per_corpus.lock().await;
        map.entry(corpus_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Acquire an indexing slot for `corpus_id`: first the corpus's exclusive
    /// lock (serializing same-corpus indexing), then a global permit (bounding
    /// total concurrency). Both are held until the returned [`IndexSlot`] drops.
    ///
    /// Always eventually resolves — a same-corpus request waits for the in-flight
    /// one rather than being dropped, so no reindex is ever lost.
    ///
    /// # Panics
    ///
    /// Never in practice: the internal semaphore is owned by `self` and is never
    /// closed, so `acquire_owned` cannot return an error.
    pub async fn acquire(&self, corpus_id: &str) -> IndexSlot {
        let lock = self.corpus_lock(corpus_id).await;
        // Serialize same-corpus indexing. No permit is held while waiting here,
        // so a busy corpus never consumes a global slot just to queue.
        let corpus_guard = lock.lock_owned().await;
        // Then bound total concurrency. The semaphore is owned by `self` and
        // never closed, so acquisition cannot fail.
        let global = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("indexing semaphore is never closed");
        IndexSlot {
            _global: global,
            _corpus: corpus_guard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn different_corpora_run_concurrently_up_to_the_bound() {
        let sched = IngestionScheduler::new(2);
        let a = sched.acquire("a").await;
        let _b = sched.acquire("b").await;

        // A third *distinct* corpus must block — the global bound is 2.
        let third = timeout(Duration::from_millis(50), sched.acquire("c")).await;
        assert!(
            third.is_err(),
            "third corpus should block at the bound of 2"
        );

        // Releasing one frees a slot.
        drop(a);
        let third = timeout(Duration::from_millis(500), sched.acquire("c")).await;
        assert!(
            third.is_ok(),
            "third corpus should proceed once a slot frees"
        );
    }

    #[tokio::test]
    async fn same_corpus_indexing_is_serialized() {
        // A generous bound proves the serialization comes from the per-corpus
        // lock, not the global permit.
        let sched = IngestionScheduler::new(8);
        let first = sched.acquire("dup").await;

        let second = timeout(Duration::from_millis(50), sched.acquire("dup")).await;
        assert!(
            second.is_err(),
            "the same corpus must not index concurrently with itself"
        );

        drop(first);
        let second = timeout(Duration::from_millis(500), sched.acquire("dup")).await;
        assert!(
            second.is_ok(),
            "a queued same-corpus reindex must run after the in-flight one (never dropped)"
        );
    }

    #[tokio::test]
    async fn default_concurrency_is_bounded_and_positive() {
        let n = IngestionScheduler::with_default_concurrency().max_concurrency();
        assert!((2..=4).contains(&n), "default bound {n} out of 2..=4");
    }

    #[tokio::test]
    async fn min_concurrency_is_at_least_one() {
        assert_eq!(IngestionScheduler::new(0).max_concurrency(), 1);
    }
}
