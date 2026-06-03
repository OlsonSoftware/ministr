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

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IndexError, StorageError};
use crate::index::{HnswIndex, HnswIndexConfig, VectorIndex, VectorIndexLoad};

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

    /// A cheap freshness fingerprint of the indexed-vector source of truth:
    /// the live vector `count` plus a monotonic `generation` that advances on
    /// every vector mutation. Used by [`load_cached_or_rebuild_hnsw`] to decide
    /// whether a persisted HNSW dump is still valid without a full rebuild.
    ///
    /// Returns `None` for backends that can't produce it — the caller then
    /// always rebuilds, so a missing fingerprint is safe (never a drift risk).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the underlying store cannot be read.
    fn indexed_vector_fingerprint(
        &self,
    ) -> impl Future<Output = Result<Option<VectorFingerprint>, StorageError>> + Send {
        async { Ok(None) }
    }
}

/// A cheap, O(1) freshness fingerprint of the indexed-vector source of truth.
///
/// `count` catches additions/removals; `generation` (bumped on every vector
/// mutation) additionally catches a same-count delete+add churn that `count`
/// alone would miss — together they make a stale persisted cache detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorFingerprint {
    /// Number of live indexed vectors.
    pub count: u64,
    /// Monotonic counter advanced on every indexed-vector mutation.
    pub generation: u64,
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

/// Cache-format / indexing-semantics version stamped into every persisted
/// HNSW cache token.
///
/// **Bump this** whenever a change to indexing or embedding semantics means a
/// graph dumped by an older binary must NOT be trusted on load (e.g. a change
/// to how vectors are normalized, truncated, or how the graph is constructed).
/// A bump invalidates every on-disk cache, forcing a clean rebuild — this is
/// the structural cure for the "fixed in code / stale on disk" bug class.
pub const HNSW_CACHE_VERSION: u32 = 1;

/// Filename of the seam-owned validity-token sidecar written next to the HNSW
/// dump. It is the ONLY artifact this module adds to the index directory; the
/// dump + id-map sidecar are produced (untouched) by [`HnswIndex::persist`].
const CACHE_TOKEN_FILE: &str = "cache_token.json";

/// The validity token that gates loading a persisted HNSW dump as a *cache*.
///
/// A persisted dump is trusted only when its token byte-for-byte matches a
/// token recomputed from the live source of truth at load time. Any
/// divergence — version bump, model/dim change, or a vector count/generation
/// change — fails the match and forces a rebuild, so the dump can never serve
/// stale results (ADR 0001 D4's no-drift guarantee, preserved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheToken {
    cache_version: u32,
    model: Option<String>,
    dim: usize,
    count: u64,
    generation: u64,
}

impl CacheToken {
    fn new(dimension: usize, model_name: Option<&str>, fp: VectorFingerprint) -> Self {
        Self {
            cache_version: HNSW_CACHE_VERSION,
            model: model_name.map(ToOwned::to_owned),
            dim: dimension,
            count: fp.count,
            generation: fp.generation,
        }
    }
}

/// Read the persisted token sidecar, returning `None` if it is absent or
/// unreadable/unparsable (any such case is treated as a cache miss → rebuild).
fn read_cache_token(index_dir: &Path) -> Option<CacheToken> {
    let path = index_dir.join(CACHE_TOKEN_FILE);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Best-effort write of the token sidecar next to a freshly-persisted dump.
/// A failure here only costs the *next* start a rebuild — never correctness —
/// so it is logged, not propagated.
fn write_cache_token(index_dir: &Path, token: &CacheToken) {
    let path = index_dir.join(CACHE_TOKEN_FILE);
    match serde_json::to_vec(token) {
        Ok(bytes) => {
            if let Err(e) = fs::write(&path, bytes) {
                tracing::warn!(error = %e, path = %path.display(), "failed to write HNSW cache token");
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to serialize HNSW cache token"),
    }
}

/// Load a persisted HNSW dump as a validated *derived cache*, or rebuild it.
///
/// This is the drift-safe near-instant-restart seam (f-ingest-hnsw-cache-token):
///
/// 1. Recompute a [`CacheToken`] from the live source of truth.
/// 2. If a dump + a token sidecar exist next to it AND the persisted token
///    matches AND the dump is model/dim-compatible, [`HnswIndex::load`] it —
///    skipping the O(N·log N·M) graph construction that makes a cold rebuild
///    cost seconds on a large corpus.
/// 3. Otherwise [`rebuild_hnsw_from_store`], then persist the dump + a fresh
///    token so the *next* restart is a cache hit.
///
/// Returns:
/// - `Ok(Some(index))` — a usable index (cache hit, or a non-empty rebuild).
/// - `Ok(None)` — no indexed vectors (a pre-V24 / legacy corpus); the caller
///   should fall back to its existing on-disk-dump / fresh-index path.
/// - `Err(_)` — the store could not be read.
///
/// Drift is structurally impossible: the dump is never trusted without a
/// matching token, and a mismatch always rebuilds (ADR 0001 D4 preserved).
///
/// # Errors
///
/// Returns [`IndexError`] if the source of truth cannot be read or a rebuild
/// fails.
pub async fn load_cached_or_rebuild_hnsw<S: IndexedVectorStore + ?Sized>(
    store: &S,
    index_dir: &Path,
    dimension: usize,
    model_name: Option<&str>,
) -> Result<Option<HnswIndex>, IndexError> {
    // (1) Fingerprint the source of truth. A backend that can't fingerprint
    // (default `None`) simply skips the fast path and always rebuilds.
    let fingerprint =
        store
            .indexed_vector_fingerprint()
            .await
            .map_err(|e| IndexError::LoadFailed {
                path: PathBuf::from("<indexed_vectors>"),
                reason: format!("failed to fingerprint indexed vectors: {e}"),
            })?;

    // (2) Fast path: a valid, matching, compatible dump loads directly.
    if let Some(fp) = fingerprint
        && fp.count > 0
        && index_dir.exists()
        && read_cache_token(index_dir).as_ref() == Some(&CacheToken::new(dimension, model_name, fp))
    {
        match HnswIndex::load(index_dir) {
            Ok(loaded) => {
                let compatible = match model_name {
                    Some(m) => loaded.check_compatible(dimension, m, index_dir).is_ok(),
                    None => true,
                };
                if compatible {
                    tracing::info!(
                        vectors = fp.count,
                        generation = fp.generation,
                        "loaded HNSW index from validated on-disk cache (skipped rebuild)"
                    );
                    return Ok(Some(loaded));
                }
                tracing::warn!("cached HNSW dump failed compatibility check — rebuilding");
            }
            Err(e) => {
                tracing::warn!(error = %e, "cached HNSW dump unreadable — rebuilding");
            }
        }
    }

    // (3) Rebuild from the source of truth (today's safe behavior), then
    // persist the dump + a fresh token so the next restart is a cache hit.
    let rebuilt = rebuild_hnsw_from_store(store, dimension, model_name).await?;
    if rebuilt.is_empty() {
        // Legacy corpus with no indexed_vectors — let the caller fall back.
        return Ok(None);
    }
    if let Some(fp) = fingerprint {
        match rebuilt.persist(index_dir) {
            Ok(()) => write_cache_token(index_dir, &CacheToken::new(dimension, model_name, fp)),
            Err(e) => {
                // Non-fatal: the rebuilt index is fully usable in memory; we
                // just won't get a fast restart until a later persist succeeds.
                tracing::warn!(error = %e, "failed to persist rebuilt HNSW cache (will rebuild next start)");
            }
        }
    }
    Ok(Some(rebuilt))
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

    // --- f-ingest-hnsw-cache-token: validated derived-HNSW cache ---

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store with a configurable freshness fingerprint that counts how many
    /// times the (expensive) rebuild read path is taken. A cache hit never
    /// calls `list_indexed_vectors`; a rebuild always does — so the call count
    /// is a precise observable for "did we load the cache or rebuild?".
    struct CountingStore {
        vectors: Vec<(String, Vec<f32>)>,
        fingerprint: Option<VectorFingerprint>,
        list_calls: AtomicUsize,
    }

    impl CountingStore {
        fn new(vectors: Vec<(&str, Vec<f32>)>, fingerprint: Option<VectorFingerprint>) -> Self {
            Self {
                vectors: vectors
                    .into_iter()
                    .map(|(id, v)| (id.to_string(), v))
                    .collect(),
                fingerprint,
                list_calls: AtomicUsize::new(0),
            }
        }
        fn rebuilds(&self) -> usize {
            self.list_calls.load(Ordering::SeqCst)
        }
    }

    impl IndexedVectorStore for CountingStore {
        async fn list_indexed_vectors(&self) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.vectors.clone())
        }
        async fn indexed_vector_fingerprint(
            &self,
        ) -> Result<Option<VectorFingerprint>, StorageError> {
            Ok(self.fingerprint)
        }
    }

    fn fp(count: u64, generation: u64) -> VectorFingerprint {
        VectorFingerprint { count, generation }
    }

    #[tokio::test]
    async fn unchanged_corpus_loads_cache_without_rebuilding() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("idx");
        let v = vec![("a", vec![1.0, 0.0, 0.0]), ("b", vec![0.0, 1.0, 0.0])];

        // First start: no cache → rebuild + persist + write token.
        let first = CountingStore::new(v.clone(), Some(fp(2, 1)));
        let idx = load_cached_or_rebuild_hnsw(&first, &dir, 3, Some("m"))
            .await
            .unwrap()
            .expect("non-empty rebuild");
        assert_eq!(idx.len(), 2);
        assert_eq!(first.rebuilds(), 1, "first start must rebuild");
        assert!(dir.join(CACHE_TOKEN_FILE).exists(), "token written");

        // Second start, identical fingerprint: must LOAD the cache (no rebuild).
        let second = CountingStore::new(v, Some(fp(2, 1)));
        let idx = load_cached_or_rebuild_hnsw(&second, &dir, 3, Some("m"))
            .await
            .unwrap()
            .expect("cache hit");
        assert_eq!(idx.len(), 2);
        assert_eq!(second.rebuilds(), 0, "cache hit must NOT rebuild");
    }

    #[tokio::test]
    async fn generation_change_forces_rebuild_even_at_equal_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("idx");

        let first = CountingStore::new(
            vec![("a", vec![1.0, 0.0, 0.0]), ("b", vec![0.0, 1.0, 0.0])],
            Some(fp(2, 1)),
        );
        load_cached_or_rebuild_hnsw(&first, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();

        // Same COUNT (2) but a bumped generation (a delete+add churn): the
        // count alone would falsely accept the stale cache — generation saves
        // us. Different vectors prove the rebuild reflects the new truth.
        let churned = CountingStore::new(
            vec![("c", vec![0.0, 0.0, 1.0]), ("d", vec![1.0, 1.0, 0.0])],
            Some(fp(2, 2)),
        );
        let idx = load_cached_or_rebuild_hnsw(&churned, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(churned.rebuilds(), 1, "generation change must rebuild");
        let hit = idx.search_knn(&[0.0, 0.0, 1.0], 1).unwrap();
        assert_eq!(hit[0].id, "c", "rebuilt index reflects the new vectors");
    }

    #[tokio::test]
    async fn count_change_forces_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("idx");
        let first = CountingStore::new(vec![("a", vec![1.0, 0.0, 0.0])], Some(fp(1, 1)));
        load_cached_or_rebuild_hnsw(&first, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();

        let grown = CountingStore::new(
            vec![("a", vec![1.0, 0.0, 0.0]), ("b", vec![0.0, 1.0, 0.0])],
            Some(fp(2, 2)),
        );
        load_cached_or_rebuild_hnsw(&grown, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grown.rebuilds(), 1, "count change must rebuild");
    }

    #[tokio::test]
    async fn corrupt_or_version_mismatched_token_rebuilds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("idx");
        let v = vec![("a", vec![1.0, 0.0, 0.0])];

        // Build a real cache first.
        let first = CountingStore::new(v.clone(), Some(fp(1, 1)));
        load_cached_or_rebuild_hnsw(&first, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();

        // Corrupt the token sidecar → treated as a miss → rebuild.
        fs::write(dir.join(CACHE_TOKEN_FILE), b"{ not json").unwrap();
        let after_corrupt = CountingStore::new(v.clone(), Some(fp(1, 1)));
        load_cached_or_rebuild_hnsw(&after_corrupt, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_corrupt.rebuilds(), 1, "corrupt token must rebuild");

        // A stale cache-version (older binary's dump) must not be trusted.
        let stale = CacheToken {
            cache_version: HNSW_CACHE_VERSION + 1,
            model: Some("m".to_string()),
            dim: 3,
            count: 1,
            generation: 1,
        };
        write_cache_token(&dir, &stale);
        let after_version = CountingStore::new(v, Some(fp(1, 1)));
        load_cached_or_rebuild_hnsw(&after_version, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_version.rebuilds(), 1, "version bump must rebuild");
    }

    #[tokio::test]
    async fn missing_fingerprint_always_rebuilds_and_empty_yields_none() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("idx");

        // No fingerprint (legacy/other backend) → never trusts a cache.
        let no_fp = CountingStore::new(vec![("a", vec![1.0, 0.0, 0.0])], None);
        load_cached_or_rebuild_hnsw(&no_fp, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        load_cached_or_rebuild_hnsw(&no_fp, &dir, 3, Some("m"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(no_fp.rebuilds(), 2, "no fingerprint ⇒ always rebuild");

        // Empty source of truth ⇒ None so the caller can do its legacy fallback.
        let empty = CountingStore::new(vec![], Some(fp(0, 0)));
        let out = load_cached_or_rebuild_hnsw(&empty, &dir, 3, Some("m"))
            .await
            .unwrap();
        assert!(out.is_none(), "empty store yields None");
    }
}
