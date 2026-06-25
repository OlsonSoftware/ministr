//! The Parse stage of the ingestion pipeline (ADR 0001 D3, slice 2).
//!
//! Extracted from the `run_producer_consumer` god-function so the producer
//! side — the discover-driven file stream, cancellation gating, off-runtime
//! parse via [`IngestionPipeline::parse_and_store_file`], stats accounting,
//! periodic HNSW persist, rollback bookkeeping, and dispatch of
//! `(VectorId, text)` pairs to the Embed stage — is a single, isolated
//! boundary. It is the upstream half of the pipes-and-filters seam whose
//! downstream half is [`super::embed_stage::run_embed_stage`].
//!
//! Cross-stage plumbing and the four pipeline knobs the producer reads are
//! grouped into [`ParseStageWiring`] so `IngestionPipeline`'s fields stay
//! private — the only widened surface is `parse_and_store_file` itself.
//!
//! The remaining stages (Discover / Extract / Persist as explicit traits, and
//! collapsing `parse_and_store_file` behind an injectable parse fn so the
//! stage is testable without a full `IngestionPipeline`) are follow-up slices
//! of `f-ingest-staged-pipeline`.

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, trace, warn};

use crate::code::package_graph::PackageGraph;
use crate::index::VectorIndex;
use crate::storage::traits::Storage;
use crate::types::{ContentId, VectorId};

use super::pipeline::{FileItem, FileResult, IngestionPipeline, IngestionProgress, IngestionStats};
use super::symbols::PendingRef;

/// Cross-stage wiring + resolved config the Parse stage needs beyond the core
/// `(pipeline, files, storage, index, graph, stats)` inputs.
///
/// Bundles the producer→consumer plumbing (the embed-channel sender, the
/// rollback doc-id tracker, the two cancellation tokens) together with the
/// four pipeline knobs the producer reads (`concurrency`, `progress`,
/// `persist_every`, `corpus_dir`). This keeps [`run_parse_stage`] within the
/// argument budget while leaving `IngestionPipeline`'s fields private.
pub(super) struct ParseStageWiring<'a> {
    /// `buffer_unordered` width for the parse fan-out.
    pub concurrency: usize,
    /// Optional progress sink (UI counters).
    pub progress: Option<&'a Arc<IngestionProgress>>,
    /// Mid-run HNSW persist cadence; `None` disables it.
    pub persist_every: Option<usize>,
    /// On-disk corpus dir to snapshot the HNSW into when `persist_every` fires.
    pub corpus_dir: Option<&'a Path>,
    /// Sender into the Embed stage's channel.
    pub embed_tx: tokio::sync::mpsc::Sender<Vec<(VectorId, String)>>,
    /// Documents persisted this run, tracked for rollback on embed failure.
    pub indexed_doc_ids: &'a Mutex<Vec<ContentId>>,
    /// Internal cancel signal (tripped by the consumer on embed failure).
    pub internal_ct: &'a CancellationToken,
    /// Caller-supplied cancel signal, if any.
    pub external_ct: Option<&'a CancellationToken>,
}

/// Run the Parse stage: stream `file_items` through
/// [`IngestionPipeline::parse_and_store_file`] at `concurrency` width, account
/// each per-file outcome into `stats`, periodically snapshot the HNSW, track
/// persisted docs for rollback, and forward every file's embedding pairs to
/// the Embed stage via `wiring.embed_tx`.
///
/// Returns `(was_cancelled, pending_refs)` — whether either cancel token fired,
/// and the accumulated cross-reference work for the later resolve phase.
pub(super) async fn run_parse_stage<S, I>(
    pipeline: &IngestionPipeline,
    file_items: Vec<FileItem>,
    storage: &S,
    index: &I,
    active_graph: Option<&PackageGraph>,
    stats: &mut IngestionStats,
    wiring: ParseStageWiring<'_>,
) -> (bool, Vec<PendingRef>)
where
    S: Storage + ?Sized,
    I: VectorIndex + ?Sized,
{
    let ParseStageWiring {
        concurrency,
        progress,
        persist_every,
        corpus_dir,
        embed_tx,
        indexed_doc_ids,
        internal_ct,
        external_ct,
    } = wiring;

    info!(
        concurrency,
        files = file_items.len(),
        "starting concurrent file ingestion"
    );

    let mut all_pending_refs = Vec::new();
    let mut cancelled = false;

    let mut parse_stream = std::pin::pin!(
        stream::iter(file_items)
            .take_while(|_| {
                let external_stop = external_ct.is_some_and(CancellationToken::is_cancelled);
                let internal_stop = internal_ct.is_cancelled();
                let stop = external_stop || internal_stop;
                async move { !stop }
            })
            .map(|item| {
                // Bug #6: announce the file as *started* — before the parse
                // kicks off — so the UI shows work in progress, not the
                // previous finished file.
                if let Some(progress) = progress {
                    progress.set_current_file(&item.relative);
                }
                let internal_ct = internal_ct.clone();
                async move {
                    // Bug #2 (partial): check cancellation at parse entry so
                    // futures that `buffer_unordered` queued before a cancel
                    // fires don't spend CPU parsing a file the caller has
                    // already abandoned. Inner parse steps remain
                    // non-cancelable — threading the token through tree-sitter
                    // + extractors is a follow-up.
                    if internal_ct.is_cancelled()
                        || external_ct.is_some_and(CancellationToken::is_cancelled)
                    {
                        return (item, Ok(FileResult::Skipped));
                    }
                    let result = pipeline
                        .parse_and_store_file(
                            &item.path,
                            &item.relative,
                            item.root_path.as_deref(),
                            storage,
                            index,
                            active_graph,
                        )
                        .await;
                    (item, result)
                }
            })
            .buffer_unordered(concurrency)
    );

    while let Some((item, result)) = parse_stream.next().await {
        match result {
            Ok(FileResult::Skipped) => {
                debug!(path = %item.relative, "unchanged, skipping");
                stats.files_skipped += 1;
            }
            Ok(indexed @ FileResult::Indexed { .. }) => {
                let mut sink = ParseSink {
                    stats: &mut *stats,
                    all_pending_refs: &mut all_pending_refs,
                    storage,
                    index,
                    progress,
                    persist_every,
                    corpus_dir,
                    indexed_doc_ids,
                    embed_tx: &embed_tx,
                };
                if sink.record(&item, indexed).await.is_break() {
                    // Consumer dropped rx — it errored. Stop.
                    break;
                }
            }
            Err(e) => {
                // Bug #5: record the failing path + reason so callers can
                // surface the failure without scraping logs.
                let reason = e.to_string();
                tracing::error!(path = %item.relative, error = %reason, "failed to ingest file");
                stats.files_failed += 1;
                stats.failed_files.push((item.relative.clone(), reason));
            }
        }

        if let Some(progress) = progress {
            progress.increment_done();
        }
    }
    drop(embed_tx);

    if external_ct.is_some_and(CancellationToken::is_cancelled) || internal_ct.is_cancelled() {
        cancelled = true;
    }

    (cancelled, all_pending_refs)
}

/// Borrowed working set the per-file handler mutates while draining the parse
/// stream. Grouping it keeps [`ParseSink::record`] within the argument budget
/// and separates "account one parsed file" (its single responsibility) from
/// "drive the stream" ([`run_parse_stage`]).
struct ParseSink<'a, S: ?Sized, I: ?Sized> {
    stats: &'a mut IngestionStats,
    all_pending_refs: &'a mut Vec<PendingRef>,
    storage: &'a S,
    index: &'a I,
    progress: Option<&'a Arc<IngestionProgress>>,
    persist_every: Option<usize>,
    corpus_dir: Option<&'a Path>,
    indexed_doc_ids: &'a Mutex<Vec<ContentId>>,
    embed_tx: &'a tokio::sync::mpsc::Sender<Vec<(VectorId, String)>>,
}

impl<S, I> ParseSink<'_, S, I>
where
    S: Storage + ?Sized,
    I: VectorIndex + ?Sized,
{
    /// Account one parsed file: bump stats, snapshot the HNSW on cadence, track
    /// the doc for rollback, set its root, and forward its embedding pairs to
    /// the Embed stage. Returns [`ControlFlow::Break`] when the embed channel
    /// has closed (the consumer errored) so the caller stops the stream.
    ///
    /// A non-`Indexed` `result` is a no-op (the caller only routes `Indexed`
    /// here), so this stays `Continue`.
    async fn record(&mut self, item: &FileItem, result: FileResult) -> ControlFlow<()> {
        let FileResult::Indexed {
            sections,
            claims,
            pending_refs,
            embedding_pairs,
        } = result
        else {
            return ControlFlow::Continue(());
        };

        debug!(path = %item.relative, sections, claims, "parsed and stored");
        self.stats.files_indexed += 1;
        self.stats.total_sections += sections;
        self.stats.total_claims += claims;
        self.all_pending_refs.extend(pending_refs);

        maybe_persist_snapshot(
            self.index,
            self.persist_every,
            self.corpus_dir,
            self.stats.files_indexed,
        );

        // Track this doc for rollback on consumer failure.
        let doc_id = ContentId(item.relative.clone());
        if let Ok(mut guard) = self.indexed_doc_ids.lock() {
            guard.push(doc_id.clone());
        }

        if let Some(progress) = self.progress {
            progress.add_sections_done(sections);
        }

        if let Some(rid) = item.root_id.as_deref()
            && let Err(e) = self.storage.set_document_root(&doc_id, rid).await
        {
            debug!(path = %item.relative, error = %e, "failed to set document root");
        }

        if !embedding_pairs.is_empty() {
            if let Some(progress) = self.progress {
                progress.add_embeddings_total(embedding_pairs.len());
            }
            if self.embed_tx.send(embedding_pairs).await.is_err() {
                return ControlFlow::Break(());
            }
        }

        ControlFlow::Continue(())
    }
}

/// Periodic mid-run HNSW snapshot (/).
///
/// Fires only when *both* `persist_every` and a `corpus_dir` are configured
/// (callers that bundle at end-of-ingest leave `corpus_dir` unset), the
/// `files_indexed` count is a non-zero multiple of `persist_every`, and the
/// consumer has flushed at least one vector. HNSW persist is atomic
/// (tmp-rename + fsync), so the in-memory graph keeps taking inserts while the
/// snapshot is a recoverable point-in-time copy. The `!index.is_empty()` gate
/// avoids a useless "nb point 0" WARN every boundary while the parser races
/// ahead of the embedder.
fn maybe_persist_snapshot<I>(
    index: &I,
    persist_every: Option<usize>,
    corpus_dir: Option<&Path>,
    files_indexed: usize,
) where
    I: VectorIndex + ?Sized,
{
    let (Some(n), Some(dir)) = (persist_every, corpus_dir) else {
        return;
    };
    if n == 0 || !files_indexed.is_multiple_of(n) {
        return;
    }
    if index.is_empty() {
        trace!(
            files_indexed,
            "skipping mid-run HNSW persist: index has no vectors yet"
        );
        return;
    }
    match index.persist(dir) {
        Ok(()) => debug!(
            files_indexed,
            dir = %dir.display(),
            "mid-run HNSW persist snapshot"
        ),
        Err(e) => warn!(
            files_indexed,
            error = %e,
            "mid-run HNSW persist failed; continuing"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::NullVectorIndex;
    use crate::storage::SqliteStorage;

    /// Zeroed stats with a known discovered count. Built via the public struct
    /// literal because `IngestionStats::new` is private to the pipeline module.
    fn fresh_stats(discovered: usize) -> IngestionStats {
        IngestionStats {
            files_discovered: discovered,
            files_skipped: 0,
            files_indexed: 0,
            files_removed: 0,
            files_failed: 0,
            total_sections: 0,
            total_claims: 0,
            total_embeddings: 0,
            failed_files: Vec::new(),
        }
    }

    /// Write one markdown file under a fresh temp dir and return `(dir, item)`.
    /// The dir is returned so the caller keeps it alive for the test's duration.
    fn one_markdown_file() -> (tempfile::TempDir, FileItem) {
        let dir = tempfile::tempdir().expect("tempdir");
        let rel = "doc.md";
        let path = dir.path().join(rel);
        std::fs::write(
            &path,
            "# Title\n\nThis is a paragraph with enough words to form a real \
             section body that the markdown parser keeps and the pipeline \
             queues for embedding downstream.\n",
        )
        .expect("write fixture");
        let item = FileItem {
            path,
            relative: rel.to_owned(),
            root_id: None,
            root_path: Some(dir.path().to_path_buf()),
        };
        (dir, item)
    }

    /// The Parse stage parses + stores a discovered file, accounts it into
    /// `stats`, tracks it for rollback, and forwards its embedding pairs down
    /// the bounded channel to the (here, test-owned) Embed stage. Drains the
    /// receiver concurrently with `tokio::join!` — the stage `await`s on the
    /// bounded `send`, so an isolated test must consume the downstream end or
    /// the producer deadlocks on backpressure.
    #[tokio::test]
    async fn parse_stage_indexes_file_and_forwards_embedding_pairs() {
        let pipeline = IngestionPipeline::new().with_min_section_tokens(0);
        let storage = SqliteStorage::open_in_memory().expect("storage");
        let index = NullVectorIndex;
        let (_dir, item) = one_markdown_file();

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let indexed_doc_ids = Mutex::new(Vec::new());
        let internal_ct = CancellationToken::new();
        let mut stats = fresh_stats(1);

        let wiring = ParseStageWiring {
            concurrency: 2,
            progress: None,
            persist_every: None,
            corpus_dir: None,
            embed_tx: tx,
            indexed_doc_ids: &indexed_doc_ids,
            internal_ct: &internal_ct,
            external_ct: None,
        };

        let drain = async {
            let mut pairs = Vec::new();
            while let Some(batch) = rx.recv().await {
                pairs.extend(batch);
            }
            pairs
        };
        let run = run_parse_stage(
            &pipeline,
            vec![item],
            &storage,
            &index,
            None,
            &mut stats,
            wiring,
        );
        let ((cancelled, _pending), forwarded) = tokio::join!(run, drain);

        assert!(!cancelled, "no cancel token fired");
        assert_eq!(stats.files_indexed, 1, "the one file was indexed");
        assert!(stats.total_sections >= 1, "at least one section parsed");
        assert!(
            !forwarded.is_empty(),
            "the parse stage forwarded embedding pairs downstream"
        );
        assert!(
            indexed_doc_ids
                .lock()
                .expect("lock")
                .iter()
                .any(|c| c.0 == "doc.md"),
            "the indexed doc is tracked for rollback"
        );
    }

    /// A cancel token tripped before the stage starts short-circuits the parse
    /// stream: nothing is parsed, nothing is forwarded, and the run reports
    /// `cancelled = true`.
    #[tokio::test]
    async fn parse_stage_short_circuits_when_pre_cancelled() {
        let pipeline = IngestionPipeline::new();
        let storage = SqliteStorage::open_in_memory().expect("storage");
        let index = NullVectorIndex;
        let (_dir, item) = one_markdown_file();

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let indexed_doc_ids = Mutex::new(Vec::new());
        let internal_ct = CancellationToken::new();
        internal_ct.cancel(); // tripped before any file is parsed
        let mut stats = fresh_stats(1);

        let wiring = ParseStageWiring {
            concurrency: 2,
            progress: None,
            persist_every: None,
            corpus_dir: None,
            embed_tx: tx,
            indexed_doc_ids: &indexed_doc_ids,
            internal_ct: &internal_ct,
            external_ct: None,
        };

        let drain = async {
            let mut batches = 0usize;
            while rx.recv().await.is_some() {
                batches += 1;
            }
            batches
        };
        let run = run_parse_stage(
            &pipeline,
            vec![item],
            &storage,
            &index,
            None,
            &mut stats,
            wiring,
        );
        let ((cancelled, pending), forwarded_batches) = tokio::join!(run, drain);

        assert!(
            cancelled,
            "the pre-tripped internal token marks the run cancelled"
        );
        assert_eq!(stats.files_indexed, 0, "no file was parsed after cancel");
        assert!(pending.is_empty(), "no pending refs accumulated");
        assert_eq!(forwarded_batches, 0, "nothing forwarded downstream");
    }
}
