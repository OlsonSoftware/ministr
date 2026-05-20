//! Durable corpus-bundle persistence hook.
//!
//! [`BlobSink`] is the trait the daemon fires whenever a corpus
//! finishes ingesting. The local stack ships no concrete
//! implementation — self-hosted serve leaves indexes durable on
//! the user's own disk. Cloud deployments wire
//! `ministr_cloud::blob_sink::BlobBackendSink`, which exports the
//! corpus to a bundle and uploads it to Azure Blob Storage so the
//! index survives ACA pod restarts.
//!
//! # Why sync, not async
//!
//! Mirrors the [`crate::UsageSink`] convention: a fire-and-forget
//! `enqueue_upload` method that returns immediately. Async trait
//! methods would force either `impl Future` (which breaks `dyn`
//! dispatch in stable Rust) or boxed-future plumbing
//! (`Pin<Box<dyn Future>>`) on every call. Cloud impls spawn their
//! own `tokio::spawn(async { ... })` for the actual upload, so the
//! daemon's ingestion-completion path never blocks on storage I/O.

use std::path::PathBuf;

/// Sink for durable corpus-bundle exports emitted after each
/// successful ingestion.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn BlobSink>` inside `AppState`. The trait is `dyn`-safe
/// (no generics, no `impl Future`); the cloud crate's concrete
/// `BlobBackendSink` spawns a tokio task per call.
pub trait BlobSink: Send + Sync + std::fmt::Debug {
    /// Queue an upload of the corpus at `corpus_dir` under
    /// `corpus_id`. Returns immediately; the implementation is
    /// responsible for running the bundle export + upload off the
    /// caller's task.
    ///
    /// Fire-and-forget — the caller (registry completion reactor)
    /// never observes errors from the sink, so an implementation's
    /// storage hiccup never fails the enclosing ingestion. The
    /// implementation is responsible for logging its own failures.
    fn enqueue_upload(&self, corpus_id: String, corpus_dir: PathBuf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockSink {
        events: Mutex<Vec<(String, PathBuf)>>,
    }

    impl BlobSink for MockSink {
        fn enqueue_upload(&self, corpus_id: String, corpus_dir: PathBuf) {
            self.events.lock().unwrap().push((corpus_id, corpus_dir));
        }
    }

    #[test]
    fn trait_is_dyn_compatible() {
        // Compile-time proof — if BlobSink isn't dyn-safe, this
        // line fails to type-check.
        let sink: std::sync::Arc<dyn BlobSink> = std::sync::Arc::new(MockSink::default());
        sink.enqueue_upload("c1".to_string(), PathBuf::from("/tmp/c1"));
        sink.enqueue_upload("c2".to_string(), PathBuf::from("/tmp/c2"));
    }

    #[test]
    fn mock_sink_captures_events() {
        let sink = MockSink::default();
        sink.enqueue_upload("c1".to_string(), PathBuf::from("/tmp/c1"));
        sink.enqueue_upload("c1".to_string(), PathBuf::from("/tmp/c1-again"));
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "c1");
        assert_eq!(events[1].1, PathBuf::from("/tmp/c1-again"));
    }
}
