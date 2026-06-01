//! The Embed stage of the ingestion pipeline (ADR 0001 D3, slice 1).
//!
//! Extracted from the `run_producer_consumer` god-function so the embed step is
//! a single, isolated, testable boundary — the OCP seam the ADR names ("swap
//! the embed stage for the batched service"). [`run_embed_stage`] consumes the
//! producer's `(VectorId, text)` channel, batches it, embeds (via the dedicated
//! [`EmbeddingService`] when present, else the inline embedder), and inserts
//! into the vector index. The dual (Matryoshka) variant additionally stores the
//! full-dimension vectors in `SQLite` for two-stage reranking.
//!
//! The remaining stages (Discover / Parse / Extract / Persist as explicit
//! traits, plus decomposing the surrounding god-function) are follow-up slices
//! of `f-ingest-staged-pipeline`.

use std::sync::Arc;

use crate::embedding::{DualEmbedder, Embedder, EmbeddingService};
use crate::error::IngestionError;
use crate::index::VectorIndex;
use crate::storage::SqliteStorage;
use crate::types::VectorId;

use super::embedding::{
    EMBED_FLUSH_THRESHOLD, batch_embed_and_insert, batch_embed_and_insert_dual,
};
use super::pipeline::{IngestionPhase, IngestionProgress};

/// Run the embed stage: drain `embed_rx`, batch, embed, and insert into the
/// vector index. Dispatches to the dual (Matryoshka) path when `dual` is
/// `Some`, otherwise the single-embedder path — routing through `service` when
/// set so the synchronous embed runs off the Tokio runtime (ADR 0001 D1).
///
/// Returns the number of vectors inserted.
///
/// # Errors
///
/// Propagates [`IngestionError`] from embedding or vector insertion.
pub(super) async fn run_embed_stage<E, I>(
    embed_rx: tokio::sync::mpsc::Receiver<Vec<(VectorId, String)>>,
    embedder: &E,
    service: Option<&EmbeddingService>,
    dual: Option<(&dyn DualEmbedder, &SqliteStorage)>,
    index: &I,
    progress: Option<&Arc<IngestionProgress>>,
) -> Result<usize, IngestionError>
where
    E: Embedder + ?Sized,
    I: VectorIndex + ?Sized,
{
    if let Some((dual_embedder, full_dim_storage)) = dual {
        consume_dual(embed_rx, dual_embedder, index, full_dim_storage, progress).await
    } else {
        consume_single(embed_rx, embedder, service, index, progress).await
    }
}

/// Single-embedder path: consume embedding pairs from the producer channel,
/// batch them, and insert.
async fn consume_single<E, I>(
    mut embed_rx: tokio::sync::mpsc::Receiver<Vec<(VectorId, String)>>,
    embedder: &E,
    service: Option<&EmbeddingService>,
    index: &I,
    progress: Option<&Arc<IngestionProgress>>,
) -> Result<usize, IngestionError>
where
    E: Embedder + ?Sized,
    I: VectorIndex + ?Sized,
{
    let mut total_embeddings = 0usize;
    let mut buffer: Vec<(VectorId, String)> = Vec::new();
    // Track whether we've signalled the `Embedding` phase yet — flip it on the
    // FIRST batch received so SSE consumers see the phase change at the right
    // moment. Before the first batch arrives, the producer is still parsing;
    // flipping any earlier would misreport the work.
    let mut phase_flipped = false;

    while let Some(pairs) = embed_rx.recv().await {
        if !phase_flipped && let Some(p) = progress {
            p.set_phase(IngestionPhase::Embedding);
            phase_flipped = true;
        }
        buffer.extend(pairs);
        if buffer.len() >= EMBED_FLUSH_THRESHOLD {
            let count = batch_embed_and_insert(&buffer, embedder, service, index).await?;
            total_embeddings += count;
            if let Some(p) = progress {
                p.add_embeddings_done(count);
            }
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        let count = batch_embed_and_insert(&buffer, embedder, service, index).await?;
        total_embeddings += count;
        if let Some(p) = progress {
            p.add_embeddings_done(count);
        }
    }
    Ok(total_embeddings)
}

/// Dual path: consume embedding pairs using a [`DualEmbedder`], storing both
/// truncated vectors in the HNSW index and full-dimension vectors in `SQLite`.
async fn consume_dual<I>(
    mut embed_rx: tokio::sync::mpsc::Receiver<Vec<(VectorId, String)>>,
    dual_embedder: &dyn DualEmbedder,
    index: &I,
    full_dim_storage: &SqliteStorage,
    progress: Option<&Arc<IngestionProgress>>,
) -> Result<usize, IngestionError>
where
    I: VectorIndex + ?Sized,
{
    let mut total_embeddings = 0usize;
    let mut buffer: Vec<(VectorId, String)> = Vec::new();
    // Mirror of the single-embedder path — flip phase on the first batch so SSE
    // consumers see `Embedding` at the right moment.
    let mut phase_flipped = false;

    while let Some(pairs) = embed_rx.recv().await {
        if !phase_flipped && let Some(p) = progress {
            p.set_phase(IngestionPhase::Embedding);
            phase_flipped = true;
        }
        buffer.extend(pairs);
        if buffer.len() >= EMBED_FLUSH_THRESHOLD {
            let count =
                batch_embed_and_insert_dual(&buffer, dual_embedder, index, full_dim_storage)
                    .await?;
            total_embeddings += count;
            if let Some(p) = progress {
                p.add_embeddings_done(count);
            }
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        let count =
            batch_embed_and_insert_dual(&buffer, dual_embedder, index, full_dim_storage).await?;
        total_embeddings += count;
        if let Some(p) = progress {
            p.add_embeddings_done(count);
        }
    }
    Ok(total_embeddings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IndexError;
    use crate::index::HnswIndex;

    /// Minimal non-degenerate embedder for the stage test (the index rejects
    /// zero vectors, so the first component is set to 1.0).
    struct StageMockEmbedder {
        dim: usize,
    }

    impl Embedder for StageMockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .map(|_| {
                    let mut v = vec![0.0_f32; self.dim];
                    v[0] = 1.0;
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[tokio::test]
    async fn single_embed_stage_consumes_channel_and_inserts_vectors() {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let embedder = StageMockEmbedder { dim: 8 };
        let index = HnswIndex::new(8, 100).expect("hnsw index");

        tx.send(vec![
            (VectorId::section("doc#s1"), "hello".to_owned()),
            (VectorId::section("doc#s2"), "world".to_owned()),
        ])
        .await
        .expect("send pairs");
        drop(tx); // close the channel so the stage drains and returns

        let count = run_embed_stage(rx, &embedder, None, None, &index, None)
            .await
            .expect("embed stage");

        assert_eq!(count, 2, "both pairs embedded + inserted");
        assert_eq!(index.len(), 2, "both vectors landed in the index");
    }

    #[tokio::test]
    async fn embed_stage_on_empty_channel_inserts_nothing() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let embedder = StageMockEmbedder { dim: 4 };
        let index = HnswIndex::new(4, 16).expect("hnsw index");
        drop(tx);

        let count = run_embed_stage(rx, &embedder, None, None, &index, None)
            .await
            .expect("embed stage");

        assert_eq!(count, 0);
        assert_eq!(index.len(), 0);
    }
}
