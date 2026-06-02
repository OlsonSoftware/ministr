//! Ingestion coordinator — the single owner of the indexing **job queue**.
//!
//! Before this module, every ingest entry point (`register`,
//! `update_corpus_paths`, and the file watcher) independently
//! `tokio::spawn`ed [`indexer::run`](crate::indexer::run), which then blocked
//! internally on [`IngestionScheduler::acquire`](crate::scheduler::IngestionScheduler::acquire).
//! That left the "queue" implicit — a pile of independently-blocked tasks with
//! no observable depth and nothing to coalesce or prioritize.
//!
//! [`IngestionCoordinator`] makes the queue explicit and owns it (SRP): corpora
//! **enqueue** jobs; a bounded set of jobs drains the queue. It separates the
//! two concurrency concerns cleanly:
//!
//! * **Global bound** — delegated to the existing [`IngestionScheduler`]: each
//!   dispatched job holds an [`OwnedSemaphorePermit`](tokio::sync::OwnedSemaphorePermit)
//!   for its whole run, so at most `max_concurrency` corpora index at once.
//! * **Per-corpus exclusion** — a `busy` set: a corpus already indexing is
//!   never selected again until its job completes, so a corpus never indexes
//!   concurrently with itself. Crucially, dispatch **skips** busy corpora
//!   rather than blocking on them, so one slow corpus can't head-of-line-block
//!   the others.
//! * **Priority (cq-priority)** — dispatch is shortest-job-first: among
//!   non-busy queued corpora the one with the smallest estimated work (its
//!   last-known indexed file count) goes next, ties broken FIFO. Small user
//!   code repos are indexed ahead of huge vendored trees. A never-indexed
//!   corpus estimates as 0 (treated as small) so first-time indexing is prompt;
//!   a cold start where every size is still unknown degrades to FIFO (size is
//!   unknowable without walking the tree, which the enqueue path must not do).
//!
//! Dispatch is **event-driven**, not a polling loop: [`enqueue`](IngestionCoordinator::enqueue)
//! and every job completion call [`try_dispatch`](IngestionCoordinator::try_dispatch),
//! which launches as many ready jobs as free permits allow and then parks. A
//! freed permit is always followed by a `try_dispatch` from the completing
//! job, so there is no lost-wakeup.
//!
//! Status transitions straddle the wait exactly as cq-status intends: a job is
//! marked [`Queued`](IndexingStatus::Queued) at enqueue (while it waits its
//! turn) and [`Indexing`](IndexingStatus::Indexing) at dispatch (when a permit
//! is in hand and work actually starts). The terminal transitions
//! (`Idle`/`Error`/stats) stay in [`indexer::run_body`](crate::indexer::run_body).
//!
//! Teardown is unchanged: each dispatched job's `JoinHandle` is pushed into its
//! corpus's [`CorpusHandle::tasks`](crate::registry::CorpusHandle::tasks), so
//! `unregister` still awaits in-flight indexing (after cancelling) before
//! deleting the corpus directory. A job that is dispatched *after* its corpus
//! was unregistered no-ops via `run_body`'s registry guard (it opens no files),
//! so `remove_dir_all` stays safe.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use ministr_api::corpus::IndexingStatus;

use crate::registry::CorpusRegistry;

/// A single dispatched indexing job: which corpus, and the paths to ingest.
struct Job {
    corpus_id: String,
    paths: Vec<String>,
}

/// A corpus waiting in the queue.
struct PendingJob {
    /// The latest paths to ingest for this corpus.
    paths: Vec<String>,
    /// Dispatch priority — **smaller is sooner**. An estimate of indexing work
    /// (the corpus's last-known indexed file count), so small user code repos
    /// are dispatched ahead of huge vendored trees (shortest-job-first).
    priority: usize,
    /// Monotonic enqueue order, used as a stable FIFO tiebreak among equal
    /// priorities (and so a never-indexed corpus keeps its arrival order).
    seq: u64,
}

/// The mutable queue state, guarded by a brief `std::sync::Mutex` that is
/// **never** held across an `.await`.
#[derive(Default)]
struct QueueState {
    /// Pending corpora keyed by id — at most one entry each, so repeated
    /// enqueues for the same corpus collapse onto one slot (their paths
    /// unioned, original FIFO position preserved). Selection is
    /// shortest-job-first by `(priority, seq)`.
    pending: HashMap<String, PendingJob>,
    /// Corpora with a job currently in flight (holding a permit). Selection
    /// skips these, giving per-corpus exclusion without head-of-line blocking.
    busy: HashSet<String>,
    /// Monotonic enqueue counter feeding each job's `seq` (FIFO tiebreak).
    next_seq: u64,
}

impl QueueState {
    /// Record (or refresh) a pending job for `corpus_id` at `priority`. If the
    /// corpus is already queued, the request coalesces onto the existing slot:
    /// the requested paths are **unioned** into it (lossless coalescing — no
    /// concurrently-requested path is dropped), the priority is refreshed to the
    /// latest estimate, and the original FIFO position (`seq`) is preserved — it
    /// is not re-queued.
    ///
    /// The union (rather than latest-wins-replace) matters when two different
    /// path sets land on one corpus before it dispatches — e.g. an
    /// `update_corpus_paths` "added" set collapsing with the file watcher's
    /// full-set enqueue, or vice versa. Re-ingesting the union is always safe
    /// because ingestion is incremental (unchanged files are skipped).
    fn upsert(&mut self, corpus_id: String, paths: Vec<String>, priority: usize) {
        if let Some(job) = self.pending.get_mut(&corpus_id) {
            for p in paths {
                if !job.paths.contains(&p) {
                    job.paths.push(p);
                }
            }
            job.priority = priority;
        } else {
            let seq = self.next_seq;
            self.next_seq += 1;
            self.pending.insert(
                corpus_id,
                PendingJob {
                    paths,
                    priority,
                    seq,
                },
            );
        }
    }

    /// Remove and return the most-eligible queued job whose corpus is **not**
    /// busy — lowest `priority` (shortest job first), ties broken by lowest
    /// `seq` (FIFO). Marks the chosen corpus busy. Returns `None` when nothing
    /// is dispatchable (queue empty, or every queued corpus is already indexing).
    fn take_next_dispatchable(&mut self) -> Option<Job> {
        let busy = &self.busy;
        let corpus_id = self
            .pending
            .iter()
            .filter(|(cid, _)| !busy.contains(cid.as_str()))
            .min_by(|(_, a), (_, b)| (a.priority, a.seq).cmp(&(b.priority, b.seq)))
            .map(|(cid, _)| cid.clone())?;
        let job = self
            .pending
            .remove(&corpus_id)
            .expect("corpus_id came from this map");
        self.busy.insert(corpus_id.clone());
        Some(Job {
            corpus_id,
            paths: job.paths,
        })
    }
}

/// Owns the explicit ingestion job queue and drives bounded, fair dispatch.
///
/// Held by [`CorpusRegistry`]; reached from a running job via
/// [`CorpusRegistry::coordinator`] so completions can re-drive dispatch without
/// a back-reference cycle.
#[derive(Default)]
pub struct IngestionCoordinator {
    state: Mutex<QueueState>,
}

impl IngestionCoordinator {
    /// A fresh, empty coordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Enqueue an indexing job for `corpus_id` over `paths` at `priority`
    /// (smaller = dispatched sooner), mark the corpus
    /// [`Queued`](IndexingStatus::Queued), and attempt immediate dispatch.
    ///
    /// Returns once the job is queued and dispatch has been attempted — **not**
    /// when indexing finishes. The actual ingest runs in a spawned task tracked
    /// for teardown.
    pub async fn enqueue(
        &self,
        registry: &Arc<CorpusRegistry>,
        corpus_id: String,
        paths: Vec<String>,
        priority: usize,
    ) {
        self.lock().upsert(corpus_id.clone(), paths, priority);
        // Distinct "queued" state while the job waits its turn (cq-status):
        // a not-yet-started corpus must not keep its prior Idle/indexed status.
        registry
            .set_status(&corpus_id, IndexingStatus::Queued)
            .await;
        self.try_dispatch(registry).await;
    }

    /// Mark a finished corpus as no longer busy so it can be selected again.
    fn mark_done(&self, corpus_id: &str) {
        self.lock().busy.remove(corpus_id);
    }

    /// Launch as many ready jobs as free permits allow, then park.
    ///
    /// Each dispatched job holds a global permit for its whole run (the bound)
    /// and, on completion, releases the permit and re-drives dispatch — so a
    /// freed slot is always reconsidered.
    ///
    /// Returns a boxed future because dispatch is mutually recursive through the
    /// spawned job's completion (which re-enters `try_dispatch`); the explicit
    /// `+ Send` bound breaks the otherwise-cyclic auto-`Send` inference so the
    /// completion task can be `tokio::spawn`ed.
    pub fn try_dispatch<'a>(
        &'a self,
        registry: &'a Arc<CorpusRegistry>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            loop {
                // Claim a global slot without waiting; saturated → nothing to do.
                let Some(permit) = registry.scheduler().try_acquire_permit() else {
                    return;
                };

                let next = {
                    let mut state = self.lock();
                    state.take_next_dispatchable()
                };
                let Some(Job { corpus_id, paths }) = next else {
                    // Permit free but no dispatchable corpus (queue empty, or all
                    // queued corpora busy). Release it; a later completion will
                    // re-drive dispatch.
                    drop(permit);
                    return;
                };

                // A permit is in hand and work is starting now: Queued -> Indexing.
                registry
                    .set_status(
                        &corpus_id,
                        IndexingStatus::Indexing {
                            files_done: 0,
                            files_total: 0,
                        },
                    )
                    .await;

                let reg = Arc::clone(registry);
                let lookup_id = corpus_id.clone();
                let handle = tokio::spawn(async move {
                    crate::indexer::run_body(&reg, &corpus_id, &paths).await;
                    // Release the slot, free the corpus, and reconsider the queue.
                    drop(permit);
                    let coordinator = reg.coordinator();
                    coordinator.mark_done(&corpus_id);
                    coordinator.try_dispatch(&reg).await;
                });

                // Track the job for teardown: `unregister` awaits these (after
                // cancelling) before deleting the corpus dir. If the corpus was
                // already removed, drop the handle — the job no-ops in
                // `run_body`'s registry guard, opening no files.
                if let Some(corpus) = registry.corpora().read().await.get(&lookup_id)
                    && let Ok(mut tasks) = corpus.tasks.lock()
                {
                    tasks.push(handle);
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(p: &str) -> Vec<String> {
        vec![p.to_string()]
    }

    // Equal priority everywhere → seq drives order (pure FIFO).
    const EQ: usize = 0;

    #[test]
    fn distinct_corpora_dispatch_in_fifo_order() {
        let mut q = QueueState::default();
        q.upsert("a".into(), paths("a1"), EQ);
        q.upsert("b".into(), paths("b1"), EQ);

        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "a");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "b");
        assert!(q.take_next_dispatchable().is_none());
    }

    #[test]
    fn same_corpus_enqueues_coalesce_to_one_slot_unioning_paths() {
        let mut q = QueueState::default();
        // Two different path sets for one queued corpus (e.g. an
        // update_corpus_paths "added" set + a watcher full-set enqueue).
        q.upsert("a".into(), vec!["p1".into()], EQ);
        q.upsert("a".into(), vec!["p2".into(), "p1".into()], EQ);

        let job = q.take_next_dispatchable().unwrap();
        assert_eq!(job.corpus_id, "a");
        assert_eq!(
            job.paths,
            vec!["p1".to_string(), "p2".to_string()],
            "the collapsed job ingests the union of all requested paths, deduped"
        );
        assert!(
            q.take_next_dispatchable().is_none(),
            "the two enqueues collapsed onto a single queued job"
        );
    }

    #[test]
    fn coalescing_while_running_unions_all_requests_into_one_followup() {
        let mut q = QueueState::default();
        // `a` starts indexing.
        q.upsert("a".into(), vec!["p1".into()], EQ);
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "a"); // busy

        // While `a` runs, two more requests arrive with distinct paths.
        q.upsert("a".into(), vec!["p2".into()], EQ);
        q.upsert("a".into(), vec!["p3".into(), "p2".into()], EQ);
        assert!(
            q.take_next_dispatchable().is_none(),
            "a is still busy — nothing else dispatchable"
        );

        // When `a` finishes, the two follow-up requests have collapsed into a
        // single job covering the union of their paths.
        q.busy.remove("a");
        let followup = q.take_next_dispatchable().unwrap();
        assert_eq!(followup.corpus_id, "a");
        assert_eq!(
            followup.paths,
            vec!["p2".to_string(), "p3".to_string()],
            "one follow-up job ingests the union of every request made while running"
        );
        assert!(q.take_next_dispatchable().is_none());
    }

    #[test]
    fn a_busy_corpus_is_skipped_so_it_never_head_of_line_blocks_others() {
        let mut q = QueueState::default();
        // `a` is already indexing; `b` is freshly queued.
        q.upsert("a".into(), paths("a1"), EQ);
        let busy_a = q.take_next_dispatchable().unwrap();
        assert_eq!(busy_a.corpus_id, "a"); // now marked busy

        // A new request for the busy corpus AND a request for a different one.
        q.upsert("a".into(), paths("a2"), EQ);
        q.upsert("b".into(), paths("b1"), EQ);

        // Selection skips the busy `a` and dispatches `b` — no head-of-line block.
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "b");
        // `a` stays queued until it is marked done.
        assert!(q.take_next_dispatchable().is_none());

        q.busy.remove("a");
        let redo_a = q.take_next_dispatchable().unwrap();
        assert_eq!(redo_a.corpus_id, "a");
        assert_eq!(
            redo_a.paths,
            paths("a2"),
            "the requeued job carries the latest paths"
        );
    }

    #[test]
    fn dispatch_is_shortest_job_first_then_fifo() {
        let mut q = QueueState::default();
        // Enqueue out of size order; a "huge vendored tree" enqueued first.
        q.upsert("huge".into(), paths("h"), 100_000);
        q.upsert("small".into(), paths("s"), 50);
        q.upsert("medium".into(), paths("m"), 5_000);

        // Smallest estimated work first, regardless of enqueue order.
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "small");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "medium");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "huge");
        assert!(q.take_next_dispatchable().is_none());
    }

    #[test]
    fn equal_priority_breaks_ties_fifo_by_enqueue_order() {
        let mut q = QueueState::default();
        q.upsert("first".into(), paths("1"), 500);
        q.upsert("second".into(), paths("2"), 500);
        q.upsert("third".into(), paths("3"), 500);

        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "first");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "second");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "third");
    }

    #[test]
    fn re_enqueue_updates_priority_but_keeps_fifo_position() {
        let mut q = QueueState::default();
        q.upsert("a".into(), paths("a"), 100);
        q.upsert("b".into(), paths("b"), 100);
        // `a` grows (re-enqueued at a higher cost) but was queued first; with
        // equal-or-higher priority it must NOT jump ahead of `b` on a tie, and
        // its seq is preserved so re-enqueue can't starve later arrivals.
        q.upsert("a".into(), paths("a2"), 100);

        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "a");
        assert_eq!(q.take_next_dispatchable().unwrap().corpus_id, "b");
    }
}
