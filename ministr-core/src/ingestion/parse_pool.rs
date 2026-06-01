//! Dedicated CPU thread pool for synchronous tree-sitter parsing.
//!
//! tree-sitter parsing is CPU-bound and blocking. The ingestion producer
//! (`pipeline::run_producer_consumer`) fans parse work out with
//! `buffer_unordered`, but that is *concurrency on the shared Tokio
//! workers*, not a dedicated CPU pool: every `parser.parse()` call runs
//! inline on a Tokio worker thread, so the heaviest CPU phase of indexing
//! competes with async IO/await tasks (storage writes, the embedding
//! consumer) for the same runtime threads. Under load the runtime can't
//! keep IO moving while cores are saturated parsing.
//!
//! This module owns a process-wide **dedicated** rayon pool sized to the
//! machine's parallelism. Parse work is dispatched there via
//! [`parse_on_pool`] and the calling Tokio task simply `await`s a oneshot —
//! the canonical "Tokio for IO, rayon for CPU" split. A dedicated pool
//! (rather than the global `rayon::spawn` pool) keeps parse CPU isolated
//! from any rayon used internally by the embedding backend, so parse and
//! embed don't fight over the same worker threads.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use crate::error::ParseError;
use crate::parser::DocumentParser;
use crate::types::DocumentTree;

/// Process-wide rayon pool dedicated to CPU-bound parse work.
///
/// Sized to [`std::thread::available_parallelism`] (falling back to 4 when
/// the platform can't report it) so all cores stay busy during parsing
/// without unbounded oversubscription. Built lazily on first parse.
fn parse_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("ministr-parse-{i}"))
            .build()
            .expect("failed to build the ministr parse thread pool")
    })
}

/// Parse `content` on the dedicated rayon pool, off the Tokio runtime.
///
/// The synchronous tree-sitter parse — including its internal
/// `PARSE_BUDGET` wall-clock timeout, which is enforced inside
/// `DocumentParser::parse` itself — runs on a rayon worker. The calling
/// Tokio task only `await`s the result, so async workers stay free to
/// drive IO and the embedding consumer concurrently.
///
/// `content` is an [`Arc<str>`] so it can be shared into the pool closure
/// (rayon requires `'static`) without copying the file contents; the
/// caller keeps its own handle for downstream symbol extraction.
///
/// # Errors
///
/// Returns whatever [`ParseError`] the parser produces. If the rayon
/// worker panics (or is otherwise dropped) before delivering a result, the
/// dropped oneshot sender surfaces as [`ParseError::Failed`] rather than a
/// panic on the async side.
pub(crate) async fn parse_on_pool(
    parser: Box<dyn DocumentParser>,
    path: PathBuf,
    content: Arc<str>,
) -> Result<DocumentTree, ParseError> {
    let err_path = path.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    parse_pool().spawn(move || {
        let result = parser.parse(&path, &content);
        // The receiver is only dropped if the awaiting task was cancelled;
        // in that case the parse result is simply discarded.
        let _ = tx.send(result);
    });
    rx.await.unwrap_or_else(|_| {
        Err(ParseError::Failed {
            path: err_path,
            reason: "parse worker panicked before returning a result".to_owned(),
        })
    })
}
