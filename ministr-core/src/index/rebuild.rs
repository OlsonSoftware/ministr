//! Rebuild the in-memory ANN index from the ACID vector source of truth
//! (ADR 0001, decision D4).
//!
//! # Why this exists
//!
//! Historically the HNSW graph had its own `persist`/`load` to a separate
//! on-disk dump, independent of the `SQLite` content store. That split is the
//! root of two bug classes:
//!
//! 1. **zero-vector poison** — a degenerate (zero / non-finite) vector written
//!    to the HNSW could not be transactionally reconciled with `SQLite`;
//! 2. **"fixed in code / stale on disk"** — a code fix to indexing logic left
//!    the previously-persisted graph dump unchanged until a forced re-index.
//!
//! The fix is to make the `SQLite` store the single source of truth for
//! vectors (they commit with their metadata in one transaction) and treat the
//! HNSW as a *derived* in-memory structure, rebuilt from the store on load via
//! [`rebuild_hnsw_from_store`]. There is no separate file to diverge, and the
//! insert-time degenerate guard ([`VectorIndex::insert`]) is re-applied on
//! every rebuild — so both bug classes become structurally impossible while
//! ANN speed is preserved.
//!
//! [`IndexedVectorStore`] is the dependency-inversion seam: any backend that
//! can stream back the exact indexed vectors can drive a rebuild, so the
//! `SQLite` + HNSW pairing is one swappable impl among others (e.g. a future
//! `sqlite-vec` or LanceDB backend evaluated in the store-seam benchmark).

use std::future::Future;
use std::path::PathBuf;

use crate::error::{IndexError, StorageError};
use crate::index::{HnswIndex, HnswIndexConfig, VectorIndex};

/// A durable store that can stream back the EXACT vectors inserted into the
/// ANN index — the source of truth from which the index is rebuilt.
///
/// This is the D4 dependency-inversion seam (the ADR's "`CorpusStore`/vector
/// store" trait): [`rebuild_hnsw_from_store`] depends on this abstraction, not
/// on `SqliteStorage`, so the persistence backend is swappable.
pub trait IndexedVectorStore: Send + Sync {
    /// Stream every persisted indexed vector as `(vector_id, vector)`.
    ///
    /// For dual/Matryoshka corpora these are the *truncated* vectors the HNSW
    /// actually searches — not the full-dimension rerank vectors.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the underlying store cannot be read.
    fn list_indexed_vectors(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, Vec<f32>)>, StorageError>> + Send;
}

/// Rebuild an [`HnswIndex`] from the durable vector source of truth.
///
/// Every stored vector is re-inserted through [`VectorIndex::insert`], which
/// re-applies the degenerate-vector guard — a zero / non-finite vector is
/// skipped, so a poisoned vector can never enter the rebuilt index. The index
/// is sized to the stored vector count (with a floor of 1 so the empty-corpus
/// case still constructs a valid index).
///
/// `model_name`, when provided, is stamped on the index so a later
/// [`HnswIndex::check_compatible`] can detect a vector-space mismatch.
///
/// # Errors
///
/// - [`IndexError::LoadFailed`] if the store cannot be read.
/// - [`IndexError::EmbeddingFailed`] if `dimension` is zero or a stored vector
///   has a mismatched dimension.
pub async fn rebuild_hnsw_from_store<S: IndexedVectorStore + ?Sized>(
    store: &S,
    dimension: usize,
    model_name: Option<&str>,
) -> Result<HnswIndex, IndexError> {
    let vectors = store
        .list_indexed_vectors()
        .await
        .map_err(|e| IndexError::LoadFailed {
            path: PathBuf::from("<indexed_vectors>"),
            reason: format!("failed to read indexed vectors from store: {e}"),
        })?;

    // Degenerate-index invariant (f-ingest-gov-invariants): the per-vector
    // guard in `insert` keeps zeros out of the live graph, but it can't see
    // that the *whole* source of truth has collapsed (all-zero, or every live
    // vector pointing one way — every query then returns equal-distance junk).
    // Surface it loudly here, at the rebuild boundary, so a poisoned corpus is
    // diagnosable instead of silently serving broken search. Non-fatal: the
    // rebuild still proceeds (the guard keeps the graph structurally valid).
    let health = crate::index::analyze_vectors(vectors.iter().map(|(_, v)| v.as_slice()));
    if health.is_degenerate() {
        tracing::warn!(
            total = health.total,
            degenerate = health.degenerate,
            collapsed = health.collapsed,
            "rebuilt index is degenerate: every query would return equal-distance \
             results — re-index required"
        );
    }

    let max_elements = vectors.len().max(1);
    let index = HnswIndex::with_config(HnswIndexConfig::new(dimension, max_elements))?;
    if let Some(name) = model_name {
        index.set_model_name(name);
    }

    for (id, vector) in &vectors {
        // `insert` applies the dimension check + the degenerate-vector guard:
        // a zero / non-finite vector is silently skipped (poison structurally
        // impossible), a dimension mismatch is a hard error (fail loud).
        index.insert(id, vector)?;
    }

    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory store — proves rebuild depends only on the trait,
    /// not on SQLite.
    struct MockStore(Vec<(String, Vec<f32>)>);

    impl IndexedVectorStore for MockStore {
        async fn list_indexed_vectors(&self) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn rebuild_reconstructs_and_applies_degenerate_guard() {
        let store = MockStore(vec![
            ("a".to_string(), vec![1.0, 0.0, 0.0]),
            ("b".to_string(), vec![0.0, 1.0, 0.0]),
            // Degenerate (zero) vector — must be guarded out on rebuild so it
            // cannot poison cosine search.
            ("zero".to_string(), vec![0.0, 0.0, 0.0]),
        ]);

        let index = rebuild_hnsw_from_store(&store, 3, Some("test-model"))
            .await
            .unwrap();

        // Only the two finite vectors are live; the zero vector was skipped.
        assert_eq!(index.len(), 2, "zero vector must not be indexed");

        let hits = index.search_knn(&[1.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(hits[0].id, "a", "nearest neighbor reconstructed correctly");
        assert!(
            hits.iter().all(|h| h.id != "zero"),
            "degenerate vector must never surface in results"
        );

        assert_eq!(
            index.model_name().as_deref(),
            Some("test-model"),
            "model name stamped for compatibility checks"
        );
    }

    #[tokio::test]
    async fn rebuild_empty_store_yields_empty_index() {
        let store = MockStore(vec![]);
        let index = rebuild_hnsw_from_store(&store, 384, None).await.unwrap();
        assert!(index.is_empty());
        assert_eq!(index.dimension(), 384);
    }
}
