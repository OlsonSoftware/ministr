//! Dedicated, dynamically-batched embedding service (ADR 0001, decision D1).
//!
//! # Why this exists
//!
//! [`Embedder::embed`](crate::embedding::Embedder::embed) is a synchronous,
//! GPU-bound call. The legacy ingestion path invoked it *inline on Tokio worker
//! threads*; under concurrent indexing every worker could block inside `embed`
//! and the async runtime would starve (the hang previously band-aided with a
//! global `INDEXING_SEMAPHORE(1)`).
//!
//! [`EmbeddingService`] fixes that at the root: it owns the model on a single
//! **dedicated OS thread** (long-lived blocking work belongs on a dedicated
//! thread, not `spawn_blocking`, per the Tokio docs) and exposes an **async**
//! [`EmbeddingService::embed`] that callers `await` without ever blocking a
//! runtime worker. Requests flow over a **bounded** channel (so a flooded queue
//! applies backpressure) and the worker **dynamically batches** — it drains all
//! currently-queued requests (up to `max_batch`, within a short `max_latency`
//! window) into a *single* model forward, then scatters the results back over
//! per-request [`oneshot`] channels. Because every corpus feeds the one queue,
//! the GPU is never contended and batch sizes are maximised across corpora.
//!
//! # Async, not the `Embedder` trait
//!
//! The service deliberately exposes an *async* `embed`, not the synchronous
//! [`Embedder`](crate::embedding::Embedder) trait: implementing the sync trait
//! would force callers to block a thread waiting on the reply, defeating the
//! whole purpose. Async-aware callers (the ingestion embed stage) await it.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use crate::embedding::Embedder;
use crate::error::IndexError;

/// Tuning knobs for [`EmbeddingService`].
///
/// The defaults favour throughput for an ingestion workload (a generous batch
/// and a small accumulation window) while keeping single-request latency low.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddingServiceConfig {
    /// Maximum number of *requests* coalesced into one model forward.
    pub max_batch: usize,
    /// Maximum time the worker waits to accumulate more requests after the
    /// first one arrives. `Duration::ZERO` means "drain whatever is already
    /// queued, then run immediately".
    pub max_latency: Duration,
    /// Capacity of the bounded request queue. When full, producers `await`
    /// (backpressure) rather than allocating unboundedly.
    pub queue_capacity: usize,
}

impl Default for EmbeddingServiceConfig {
    fn default() -> Self {
        Self {
            max_batch: 64,
            max_latency: Duration::from_millis(5),
            queue_capacity: 256,
        }
    }
}

/// The reply delivered back to a caller: one vector per input text, or an error.
type EmbedReply = Result<Vec<Vec<f32>>, IndexError>;

/// A unit of work handed to the worker thread.
struct EmbedRequest {
    texts: Vec<String>,
    reply: oneshot::Sender<EmbedReply>,
}

/// A dedicated-thread, dynamically-batched front end over an [`Embedder`].
///
/// Construct with [`EmbeddingService::spawn`]; clone-free and cheap to share
/// behind an `Arc`. Dropping the service closes the queue and joins the worker.
pub struct EmbeddingService {
    /// `Option` so [`Drop`] can take and drop it, signalling the worker to exit.
    tx: Option<mpsc::Sender<EmbedRequest>>,
    worker: Option<JoinHandle<()>>,
    dimension: usize,
}

impl EmbeddingService {
    /// Spawn the service around `model` with the given `config`.
    ///
    /// The model is moved onto a dedicated OS thread named `ministr-embed` and
    /// is never touched from the async runtime again.
    ///
    /// # Panics
    ///
    /// Panics if the operating system refuses to spawn the worker thread.
    #[must_use]
    pub fn spawn(model: Arc<dyn Embedder>, config: EmbeddingServiceConfig) -> Self {
        let dimension = model.dimension();
        let (tx, rx) = mpsc::channel::<EmbedRequest>(config.queue_capacity.max(1));
        let worker = std::thread::Builder::new()
            .name("ministr-embed".to_owned())
            .spawn(move || worker_loop(&model, rx, config))
            .expect("failed to spawn ministr-embed worker thread");
        Self {
            tx: Some(tx),
            worker: Some(worker),
            dimension,
        }
    }

    /// Spawn with [`EmbeddingServiceConfig::default`].
    #[must_use]
    pub fn with_model(model: Arc<dyn Embedder>) -> Self {
        Self::spawn(model, EmbeddingServiceConfig::default())
    }

    /// The dimensionality of vectors produced by the underlying model.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Embed a batch of texts, returning one vector per input in order.
    ///
    /// The call `await`s on a bounded queue (backpressure) and then on the
    /// worker's reply; it never blocks a runtime worker thread. An empty input
    /// short-circuits to an empty result without touching the worker.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the service is shutting down,
    /// the worker has gone away, or the model itself errors. A degenerate
    /// (non-finite/all-zero) vector is NOT an error — it is zeroed and skipped
    /// at index-insert time (see `validate_batch`), so it can't roll back a
    /// whole corpus.
    pub async fn embed(&self, texts: Vec<String>) -> EmbedReply {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| IndexError::EmbeddingFailed {
                reason: "embedding service is shutting down".to_owned(),
            })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(EmbedRequest {
            texts,
            reply: reply_tx,
        })
        .await
        .map_err(|_| IndexError::EmbeddingFailed {
            reason: "embedding service worker is gone".to_owned(),
        })?;
        reply_rx.await.map_err(|_| IndexError::EmbeddingFailed {
            reason: "embedding service dropped the request without replying".to_owned(),
        })?
    }
}

impl Drop for EmbeddingService {
    fn drop(&mut self) {
        // Closing the sender makes the worker's `blocking_recv` return `None`,
        // so it drains any in-flight batch and exits. Then we join it.
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The worker thread body: pull requests off the queue, batch them, run one
/// model forward per batch, and scatter the results back.
///
/// Runs on its own OS thread with no Tokio runtime — it uses `blocking_recv`
/// for the first request of each batch and `try_recv` (bounded by
/// `max_latency`) to coalesce the rest. Blocking this thread inside the model
/// call is intentional and isolated from the application's async workers.
fn worker_loop(
    model: &Arc<dyn Embedder>,
    mut rx: mpsc::Receiver<EmbedRequest>,
    config: EmbeddingServiceConfig,
) {
    while let Some(first) = rx.blocking_recv() {
        let mut batch = vec![first];
        let deadline = Instant::now() + config.max_latency;
        let mut closed = false;

        while batch.len() < config.max_batch {
            match rx.try_recv() {
                Ok(req) => batch.push(req),
                Err(mpsc::error::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    // Brief poll while the accumulation window is still open.
                    std::thread::sleep(Duration::from_micros(100));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }

        process_batch(model.as_ref(), batch);

        if closed {
            break;
        }
    }
}

/// Run one model forward over every text in `batch`, then deliver each request
/// the slice of vectors that corresponds to its own inputs.
fn process_batch(model: &dyn Embedder, batch: Vec<EmbedRequest>) {
    let refs: Vec<&str> = batch
        .iter()
        .flat_map(|req| req.texts.iter().map(String::as_str))
        .collect();

    match model.embed(&refs) {
        Ok(all_vectors) => {
            if all_vectors.len() != refs.len() {
                let reason = format!(
                    "embedder returned {} vectors for {} inputs",
                    all_vectors.len(),
                    refs.len()
                );
                for req in batch {
                    let _ = req.reply.send(Err(IndexError::EmbeddingFailed {
                        reason: reason.clone(),
                    }));
                }
                return;
            }
            let mut vectors = all_vectors.into_iter();
            for req in batch {
                let slice: Vec<Vec<f32>> = vectors.by_ref().take(req.texts.len()).collect();
                let _ = req.reply.send(Ok(sanitize_batch(slice)));
            }
        }
        Err(e) => {
            let reason = e.to_string();
            for req in batch {
                let _ = req.reply.send(Err(IndexError::EmbeddingFailed {
                    reason: reason.clone(),
                }));
            }
        }
    }
}

/// Degenerate-vector guard, matching the HNSW insert guard (commit `fb3015a`):
/// a non-finite or all-zero vector is **zeroed and skipped at index-insert
/// time** (both `HnswIndex::insert` and `rebuild_hnsw_from_store` drop
/// zero/non-finite vectors), NOT failed.
///
/// This previously returned `Err` on the first bad vector, which propagated up
/// to `run_producer_consumer` and rolled back the **entire corpus** — a single
/// degenerate section (e.g. a UE C++ file whose embedding came back non-finite)
/// took down the whole index, surfacing as a corpus stuck at ERROR / 0 files.
/// The inline embed path never did this (it relies on the insert-time guard),
/// so routing through the `EmbeddingService` was a silent regression. We now
/// zero the offending vectors (so `SQLite` never stores NaN/±inf) and count
/// them in a warning, then continue — the good sections still index.
fn sanitize_batch(mut vectors: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let mut degenerate = 0usize;
    for v in &mut vectors {
        let non_finite = v.iter().any(|x| !x.is_finite());
        let all_zero = !v.is_empty() && v.iter().all(|x| *x == 0.0);
        if non_finite || all_zero {
            degenerate += 1;
            for x in v.iter_mut() {
                *x = 0.0;
            }
        }
    }
    if degenerate > 0 {
        tracing::warn!(
            degenerate,
            total = vectors.len(),
            "embedding batch contained degenerate (non-finite/zero) vectors; \
             zeroed them (skipped at index insert) instead of failing the batch"
        );
    }
    vectors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock embedder that counts how many times `embed` is called and the
    /// size of each batch, optionally sleeping to widen the batching window.
    /// Each text embeds to a vector whose first element encodes its byte length
    /// (so callers can verify they got *their own* results back).
    struct CountingEmbedder {
        dim: usize,
        calls: Arc<AtomicUsize>,
        max_seen_batch: Arc<AtomicUsize>,
        sleep: Duration,
        zero: bool,
    }

    impl CountingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                calls: Arc::new(AtomicUsize::new(0)),
                max_seen_batch: Arc::new(AtomicUsize::new(0)),
                sleep: Duration::ZERO,
                zero: false,
            }
        }
    }

    impl Embedder for CountingEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.max_seen_batch.fetch_max(texts.len(), Ordering::SeqCst);
            if self.sleep > Duration::ZERO {
                std::thread::sleep(self.sleep);
            }
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0_f32; self.dim];
                    if self.zero {
                        return v;
                    }
                    // Encode identity so the caller can verify correct scatter.
                    v[0] = t.len() as f32;
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[tokio::test]
    async fn single_request_round_trips_with_correct_dimension() {
        let svc = EmbeddingService::with_model(Arc::new(CountingEmbedder::new(8)));
        assert_eq!(svc.dimension(), 8);
        let out = svc
            .embed(vec!["hello".to_owned(), "hi".to_owned()])
            .await
            .expect("embed");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 8);
        // First element encodes byte length → proves we got our own results.
        assert!((out[0][0] - 5.0).abs() < f32::EPSILON);
        assert!((out[1][0] - 2.0).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn empty_input_short_circuits_without_calling_the_model() {
        let model = CountingEmbedder::new(4);
        let calls = Arc::clone(&model.calls);
        let svc = EmbeddingService::with_model(Arc::new(model));
        let out = svc.embed(Vec::new()).await.expect("embed");
        assert!(out.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0, "model must not be called");
    }

    #[tokio::test]
    async fn concurrent_requests_are_merged_into_fewer_model_forwards() {
        // A slow model widens the batching window: while the worker is busy on
        // the first request, the rest pile up and coalesce into one forward.
        let mut model = CountingEmbedder::new(4);
        model.sleep = Duration::from_millis(30);
        let calls = Arc::clone(&model.calls);
        let max_batch = Arc::clone(&model.max_seen_batch);
        let svc = Arc::new(EmbeddingService::spawn(
            Arc::new(model),
            EmbeddingServiceConfig {
                max_batch: 64,
                max_latency: Duration::from_millis(20),
                queue_capacity: 256,
            },
        ));

        let n = 16;
        let mut handles = Vec::new();
        for i in 0..n {
            let svc = Arc::clone(&svc);
            handles.push(tokio::spawn(async move {
                let text = "x".repeat(i + 1);
                let out = svc.embed(vec![text]).await.expect("embed");
                // Each caller gets exactly its own one vector back, correct len.
                assert_eq!(out.len(), 1);
                assert!((out[0][0] - (i + 1) as f32).abs() < f32::EPSILON);
            }));
        }
        for h in handles {
            h.await.expect("task");
        }

        let total_calls = calls.load(Ordering::SeqCst);
        assert!(
            total_calls < n,
            "expected merging (fewer than {n} forwards), got {total_calls}"
        );
        assert!(
            max_batch.load(Ordering::SeqCst) > 1,
            "expected at least one batch to coalesce multiple requests"
        );
    }

    #[tokio::test]
    async fn degenerate_vector_is_zeroed_not_failed() {
        let mut model = CountingEmbedder::new(4);
        model.zero = true;
        let svc = EmbeddingService::with_model(Arc::new(model));
        // A degenerate (all-zero) embedding must NOT fail the batch — it's
        // zeroed and skipped at index-insert time, so the rest of the corpus
        // still indexes (a single bad section can't roll back the whole corpus).
        let out = svc
            .embed(vec!["anything".to_owned()])
            .await
            .expect("degenerate vector must not fail the batch");
        assert_eq!(out.len(), 1);
        assert!(out[0].iter().all(|x| *x == 0.0), "degenerate vector zeroed");
    }

    #[test]
    fn sanitize_batch_zeroes_degenerate_vectors_instead_of_erroring() {
        let vectors = vec![
            vec![1.0, 2.0, 3.0],           // good — untouched
            vec![f32::NAN, 1.0, 2.0],      // non-finite — zeroed
            vec![f32::INFINITY, 0.0, 0.0], // non-finite — zeroed
            vec![0.0, 0.0, 0.0],           // all-zero — left zero
        ];
        let out = sanitize_batch(vectors);
        assert_eq!(out.len(), 4);
        assert!(
            (out[0][0] - 1.0).abs() < f32::EPSILON && out[0][1..].iter().all(|x| x.is_finite()),
            "good vector preserved"
        );
        assert!(out[1].iter().all(|x| *x == 0.0), "NaN vector zeroed");
        assert!(out[2].iter().all(|x| *x == 0.0), "inf vector zeroed");
        assert!(out[3].iter().all(|x| *x == 0.0), "zero vector stays zero");
    }

    #[tokio::test]
    async fn drop_joins_the_worker_without_hanging() {
        let svc = EmbeddingService::with_model(Arc::new(CountingEmbedder::new(4)));
        let _ = svc.embed(vec!["warm".to_owned()]).await.expect("embed");
        drop(svc); // must return promptly (worker joins on channel close)
    }
}
