//! Content-addressable embedding cache backed by `SQLite`.
//!
//! Stores precomputed embedding vectors keyed by `(content_hash, model_name)`.
//! When the same text chunk appears across sessions (or across files within
//! a single ingestion), the cached vector is returned instead of re-running
//! ONNX inference.
//!
//! The [`CachedEmbedder`] wrapper implements [`Embedder`] and is a drop-in
//! replacement that transparently serves cache hits and only delegates
//! cache misses to the inner embedder.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tracing::{debug, instrument};

use super::Embedder;
use crate::error::IndexError;

/// SQLite-backed cache for precomputed embedding vectors.
///
/// Vectors are stored as little-endian `f32` byte blobs, keyed by
/// `(SHA-256 content hash, model name)`.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use parking_lot::Mutex;
/// use ministr_core::embedding::cache::EmbeddingCache;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let conn = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory()?));
/// // (assumes migrations have already run on the connection)
/// let cache = EmbeddingCache::new(conn);
/// # Ok(())
/// # }
/// ```
pub struct EmbeddingCache {
    conn: Arc<Mutex<Connection>>,
}

impl EmbeddingCache {
    /// Create a new `EmbeddingCache` sharing the given connection.
    ///
    /// The connection must already have the `embedding_cache` table
    /// (created by schema migration V13).
    #[must_use]
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Look up cached embeddings for a batch of content hashes.
    ///
    /// Returns one `Option<Vec<f32>>` per input hash — `Some` for cache hits,
    /// `None` for misses.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the database query fails.
    pub fn get_batch(
        &self,
        hashes: &[&str],
        model: &str,
    ) -> Result<Vec<Option<Vec<f32>>>, IndexError> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare_cached(
                "SELECT vector FROM embedding_cache WHERE content_hash = ?1 AND model_name = ?2",
            )
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("cache query prepare failed: {e}"),
            })?;

        let mut results = Vec::with_capacity(hashes.len());
        for &hash in hashes {
            let row: Option<Vec<u8>> = stmt
                .query_row(rusqlite::params![hash, model], |row| {
                    row.get::<_, Vec<u8>>(0)
                })
                .ok();

            results.push(row.map(|bytes| decode_vector(&bytes)));
        }

        Ok(results)
    }

    /// Store a batch of embeddings in the cache.
    ///
    /// Each entry is `(content_hash, vector)`. Uses `INSERT OR REPLACE`
    /// to handle duplicate keys.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the database insert fails.
    pub fn put_batch(&self, entries: &[(&str, &[f32])], model: &str) -> Result<(), IndexError> {
        if entries.is_empty() {
            return Ok(());
        }

        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare_cached(
                "INSERT OR REPLACE INTO embedding_cache (content_hash, model_name, vector) \
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("cache insert prepare failed: {e}"),
            })?;

        for &(hash, vector) in entries {
            let blob = encode_vector(vector);
            stmt.execute(rusqlite::params![hash, model, blob])
                .map_err(|e| IndexError::EmbeddingFailed {
                    reason: format!("cache insert failed for hash {hash}: {e}"),
                })?;
        }

        Ok(())
    }
}

/// Encode a `f32` vector as little-endian bytes.
fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for &val in vector {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Decode little-endian bytes back into a `f32` vector.
fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Compute the SHA-256 hex digest of a text string.
fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Caching wrapper around an [`Embedder`] implementation.
///
/// Transparently serves cached embeddings for unchanged text chunks
/// and only delegates cache misses to the inner embedder. Tracks
/// hit/miss statistics via atomic counters.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use parking_lot::Mutex;
/// use ministr_core::embedding::cache::{CachedEmbedder, EmbeddingCache};
/// use ministr_core::embedding::Embedder;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let conn = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory()?));
/// let cache = EmbeddingCache::new(conn);
/// // inner: Arc<dyn Embedder>
/// # let inner: Arc<dyn Embedder> = todo!();
/// let cached = CachedEmbedder::new(inner, cache, "all-MiniLM-L6-v2");
/// let vectors = cached.embed(&["hello world"])?;
/// # Ok(())
/// # }
/// ```
pub struct CachedEmbedder {
    inner: Arc<dyn Embedder>,
    cache: EmbeddingCache,
    model_name: String,
    cache_hits: AtomicUsize,
    cache_misses: AtomicUsize,
}

impl CachedEmbedder {
    /// Create a new `CachedEmbedder` wrapping the given embedder and cache.
    #[must_use]
    pub fn new(inner: Arc<dyn Embedder>, cache: EmbeddingCache, model_name: &str) -> Self {
        Self {
            inner,
            cache,
            model_name: model_name.to_string(),
            cache_hits: AtomicUsize::new(0),
            cache_misses: AtomicUsize::new(0),
        }
    }

    /// Number of embedding requests served from cache.
    #[must_use]
    pub fn cache_hits(&self) -> usize {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Number of embedding requests that required inference.
    #[must_use]
    pub fn cache_misses(&self) -> usize {
        self.cache_misses.load(Ordering::Relaxed)
    }
}

impl Embedder for CachedEmbedder {
    #[instrument(skip(self, texts), fields(count = texts.len()))]
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Hash each text
        let hashes: Vec<String> = texts.iter().map(|t| content_hash(t)).collect();

        // 1b. Collapse byte-identical texts within the batch. Measured on a
        // cold ingest: 23.7% of embed texts are byte-duplicates (sec-summary
        // == section text pairs co-occur in one flush batch), each previously
        // inferred and cache-queried separately. Identical bytes produce the
        // identical vector, so deduping is transparent by construction.
        // `slot[i]` is the unique-slot index serving input position `i`.
        let mut first: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::with_capacity(texts.len());
        let mut unique_idx: Vec<usize> = Vec::new(); // unique slot -> index into `texts`
        let mut slot: Vec<usize> = Vec::with_capacity(texts.len());
        for (i, &t) in texts.iter().enumerate() {
            if let Some(&u) = first.get(t) {
                slot.push(u);
            } else {
                first.insert(t, unique_idx.len());
                slot.push(unique_idx.len());
                unique_idx.push(i);
            }
        }
        let hash_refs: Vec<&str> = unique_idx.iter().map(|&i| hashes[i].as_str()).collect();

        // 2. Batch lookup (unique texts only)
        let cached = self.cache.get_batch(&hash_refs, &self.model_name)?;

        // 3. Identify misses (indices into the unique-slot space)
        let mut miss_indices: Vec<usize> = Vec::new();
        let mut miss_texts: Vec<&str> = Vec::new();
        for (i, entry) in cached.iter().enumerate() {
            if entry.is_none() {
                miss_indices.push(i);
                miss_texts.push(texts[unique_idx[i]]);
            }
        }

        // Hit/miss counters count UNIQUE texts per batch: duplicates of one
        // text are one inference (or one hit), not several.
        let hits = hash_refs.len() - miss_indices.len();
        self.cache_hits.fetch_add(hits, Ordering::Relaxed);
        self.cache_misses
            .fetch_add(miss_indices.len(), Ordering::Relaxed);

        // 4. Embed misses
        let miss_vectors = if miss_texts.is_empty() {
            Vec::new()
        } else {
            self.inner.embed(&miss_texts)?
        };

        // 5. Store new embeddings in cache
        if !miss_vectors.is_empty() {
            let entries: Vec<(&str, &[f32])> = miss_indices
                .iter()
                .zip(miss_vectors.iter())
                .map(|(&i, vec)| (hashes[unique_idx[i]].as_str(), vec.as_slice()))
                .collect();
            self.cache.put_batch(&entries, &self.model_name)?;
        }

        // 6. Materialize unique vectors (cached + fresh), then fan out to
        //    every input position via `slot`.
        let mut unique_vecs: Vec<Vec<f32>> =
            cached.into_iter().map(Option::unwrap_or_default).collect();
        for (&i, vector) in miss_indices.iter().zip(miss_vectors.into_iter()) {
            unique_vecs[i] = vector;
        }
        let results: Vec<Vec<f32>> = slot.into_iter().map(|u| unique_vecs[u].clone()).collect();

        debug!(
            hits,
            misses = miss_indices.len(),
            duplicates = texts.len() - hash_refs.len(),
            "embedding cache batch complete"
        );

        Ok(results)
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::AtomicUsize;

    use crate::storage::SqliteStorage;

    /// Mock embedder that counts how many texts it was asked to embed.
    struct CountingEmbedder {
        dim: usize,
        call_count: AtomicUsize,
        embed_count: AtomicUsize,
    }

    impl CountingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                call_count: AtomicUsize::new(0),
                embed_count: AtomicUsize::new(0),
            }
        }

        fn total_embedded(&self) -> usize {
            self.embed_count.load(Ordering::Relaxed)
        }
    }

    impl Embedder for CountingEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            self.embed_count.fetch_add(texts.len(), Ordering::Relaxed);
            #[allow(clippy::cast_precision_loss)]
            Ok(texts
                .iter()
                .enumerate()
                .map(|(i, _)| vec![i as f32; self.dim])
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn setup_cache() -> (SqliteStorage, EmbeddingCache) {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let cache = EmbeddingCache::new(storage.conn());
        (storage, cache)
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = vec![1.0_f32, -2.5, 0.0, 3.125];
        let encoded = encode_vector(&original);
        let decoded = decode_vector(&encoded);
        assert_eq!(original, decoded);
    }

    #[test]
    fn cache_miss_returns_none() {
        let (_storage, cache) = setup_cache();
        let results = cache.get_batch(&["nonexistent"], "test-model").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_none());
    }

    #[test]
    fn cache_put_then_get() {
        let (_storage, cache) = setup_cache();
        let vector = vec![1.0_f32, 2.0, 3.0];
        cache
            .put_batch(&[("hash1", &vector)], "test-model")
            .unwrap();

        let results = cache.get_batch(&["hash1"], "test-model").unwrap();
        assert_eq!(results[0].as_ref().unwrap(), &vector);
    }

    #[test]
    fn cache_differentiates_models() {
        let (_storage, cache) = setup_cache();
        let v1 = vec![1.0_f32];
        let v2 = vec![2.0_f32];
        cache.put_batch(&[("hash1", &v1)], "model-a").unwrap();
        cache.put_batch(&[("hash1", &v2)], "model-b").unwrap();

        let a = cache.get_batch(&["hash1"], "model-a").unwrap();
        let b = cache.get_batch(&["hash1"], "model-b").unwrap();
        assert_eq!(a[0].as_ref().unwrap(), &v1);
        assert_eq!(b[0].as_ref().unwrap(), &v2);
    }

    #[test]
    fn cached_embedder_caches_and_serves_hits() {
        let (_storage, cache) = setup_cache();
        let inner = Arc::new(CountingEmbedder::new(4));
        let inner_ref = Arc::clone(&inner);
        let embedder = CachedEmbedder::new(inner, cache, "test-model");

        // First call: all misses
        let v1 = embedder.embed(&["hello", "world"]).unwrap();
        assert_eq!(v1.len(), 2);
        assert_eq!(inner_ref.total_embedded(), 2);
        assert_eq!(embedder.cache_hits(), 0);
        assert_eq!(embedder.cache_misses(), 2);

        // Second call with same texts: all hits
        let v2 = embedder.embed(&["hello", "world"]).unwrap();
        assert_eq!(v2, v1);
        assert_eq!(inner_ref.total_embedded(), 2); // no new embeddings
        assert_eq!(embedder.cache_hits(), 2);
        assert_eq!(embedder.cache_misses(), 2);
    }

    #[test]
    fn cached_embedder_partial_hits() {
        let (_storage, cache) = setup_cache();
        let inner = Arc::new(CountingEmbedder::new(4));
        let inner_ref = Arc::clone(&inner);
        let embedder = CachedEmbedder::new(inner, cache, "test-model");

        // Cache "hello"
        embedder.embed(&["hello"]).unwrap();
        assert_eq!(inner_ref.total_embedded(), 1);

        // Now embed ["hello", "new text"] — "hello" is a hit, "new text" is a miss
        let result = embedder.embed(&["hello", "new text"]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(inner_ref.total_embedded(), 2); // only 1 new embedding
        assert_eq!(embedder.cache_hits(), 1); // "hello" hit on second call
        assert_eq!(embedder.cache_misses(), 1 + 1); // 1 miss each call
    }

    #[test]
    fn cached_embedder_empty_input() {
        let (_storage, cache) = setup_cache();
        let inner = Arc::new(CountingEmbedder::new(4));
        let embedder = CachedEmbedder::new(inner, cache, "test-model");

        let result = embedder.embed(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);

        let h3 = content_hash("different text");
        assert_ne!(h1, h3);
    }

    /// Text-deterministic mock: the vector depends only on the text bytes,
    /// like a real embedding model (identical bytes → identical vector).
    /// `CountingEmbedder` is position-dependent and would violate that
    /// premise, so dedup-transparency tests use this one.
    struct TextHashEmbedder {
        dim: usize,
        embed_count: AtomicUsize,
    }

    impl Embedder for TextHashEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            self.embed_count.fetch_add(texts.len(), Ordering::Relaxed);
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[test]
    fn duplicates_within_a_batch_are_inferred_once() {
        let (_storage, cache) = setup_cache();
        let inner = Arc::new(TextHashEmbedder {
            dim: 4,
            embed_count: AtomicUsize::new(0),
        });
        let inner_ref = Arc::clone(&inner);
        let embedder = CachedEmbedder::new(inner, cache, "test-model");

        let result = embedder
            .embed(&["alpha", "beta", "alpha", "alpha"])
            .unwrap();
        assert_eq!(result.len(), 4);
        // Only the 2 unique texts reach the inner embedder.
        assert_eq!(inner_ref.embed_count.load(Ordering::Relaxed), 2);
        // Counters use unique-text semantics.
        assert_eq!(embedder.cache_hits(), 0);
        assert_eq!(embedder.cache_misses(), 2);
        // Every duplicate position carries the identical vector.
        assert_eq!(result[0], result[2]);
        assert_eq!(result[0], result[3]);
        assert_ne!(result[0], result[1]);
    }

    #[test]
    fn dedup_output_is_position_identical_to_the_inner_embedder() {
        // The transparency invariant: for any batch (duplicates included),
        // CachedEmbedder::embed == inner.embed, position by position.
        let (_storage, cache) = setup_cache();
        let reference = TextHashEmbedder {
            dim: 4,
            embed_count: AtomicUsize::new(0),
        };
        let batch = ["a", "b", "a", "c", "b", "a"];
        let expected = reference.embed(&batch).unwrap();

        let inner = Arc::new(TextHashEmbedder {
            dim: 4,
            embed_count: AtomicUsize::new(0),
        });
        let embedder = CachedEmbedder::new(inner, cache, "test-model");
        let actual = embedder.embed(&batch).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn duplicate_hits_count_once_per_batch() {
        let (_storage, cache) = setup_cache();
        let inner = Arc::new(TextHashEmbedder {
            dim: 4,
            embed_count: AtomicUsize::new(0),
        });
        let inner_ref = Arc::clone(&inner);
        let embedder = CachedEmbedder::new(inner, cache, "test-model");

        embedder.embed(&["alpha", "alpha"]).unwrap();
        assert_eq!(embedder.cache_misses(), 1);

        // Second batch: the duplicated cached text is one hit, not two.
        embedder.embed(&["alpha", "alpha", "alpha"]).unwrap();
        assert_eq!(embedder.cache_hits(), 1);
        assert_eq!(inner_ref.embed_count.load(Ordering::Relaxed), 1);
    }
}
