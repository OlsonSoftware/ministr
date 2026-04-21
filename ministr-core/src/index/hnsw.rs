//! [`HnswIndex`] — HNSW-based vector index using `hnsw_rs`.
//!
//! Wraps the `hnsw_rs` HNSW graph for cosine-similarity approximate nearest-
//! neighbor search. Maintains a bidirectional mapping between string IDs and
//! the integer IDs required by `hnsw_rs`. Supports soft-delete by tracking
//! deleted IDs and filtering them from search results.
//!
//! Persistence uses the built-in `file_dump` / `HnswIo::load_hnsw` from
//! `hnsw_rs` plus a JSON sidecar for the string ID mapping.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use std::sync::RwLock;

use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw, HnswIo, Neighbour};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use crate::error::IndexError;

use super::{SearchResult, VectorIndex, VectorIndexLoad};

/// Base filename for the HNSW dump files.
const DUMP_BASENAME: &str = "ministr_hnsw";

/// Filename for the ID mapping sidecar.
const ID_MAP_FILE: &str = "id_map.json";

/// Default M parameter (max connections per node per layer).
const DEFAULT_M: usize = 16;

/// Default maximum layer count.
const DEFAULT_MAX_LAYER: usize = 16;

/// Default `ef_construction` parameter (search width during construction).
const DEFAULT_EF_CONSTRUCTION: usize = 200;

/// Default `ef_search` parameter (search width during queries).
const DEFAULT_EF_SEARCH: usize = 50;

/// HNSW-based approximate nearest-neighbor index with cosine similarity.
///
/// Uses `hnsw_rs` for the graph structure with a bidirectional string ID
/// mapping. Thread-safe via interior `RwLock` — multiple concurrent reads,
/// exclusive writes.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::index::{HnswIndex, VectorIndex};
///
/// let index = HnswIndex::new(384, 10_000)?;
/// index.insert("section-1", &vec![0.1; 384])?;
///
/// let results = index.search_knn(&vec![0.1; 384], 5)?;
/// assert_eq!(results[0].id, "section-1");
/// # Ok::<(), ministr_core::error::IndexError>(())
/// ```
pub struct HnswIndex {
    inner: RwLock<HnswInner>,
}

impl std::fmt::Debug for HnswIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (len, dim) = self
            .inner
            .read()
            .map(|inner| (inner.int_to_id.len(), inner.dim))
            .unwrap_or((0, 0));
        f.debug_struct("HnswIndex")
            .field("len", &len)
            .field("dimension", &dim)
            .finish()
    }
}

/// Interior state behind the `RwLock`.
struct HnswInner {
    hnsw: Hnsw<'static, f32, DistCosine>,
    /// String ID to integer ID mapping.
    id_to_int: HashMap<String, usize>,
    /// Integer ID to string ID mapping.
    int_to_id: HashMap<usize, String>,
    /// Set of soft-deleted integer IDs.
    deleted: HashSet<usize>,
    /// Next integer ID to assign.
    next_id: usize,
    /// Vector dimensionality.
    dim: usize,
    /// Search width parameter for queries.
    ef_search: usize,
    /// Name of the embedding model that produced these vectors.
    model_name: Option<String>,
}

/// Configuration for building an [`HnswIndex`].
#[derive(Debug, Clone, Copy)]
pub struct HnswIndexConfig {
    /// Vector dimensionality.
    pub dimension: usize,
    /// Maximum number of elements the index can hold.
    pub max_elements: usize,
    /// Max connections per node per layer.
    pub m: usize,
    /// Maximum layer count.
    pub max_layer: usize,
    /// Search width during index construction.
    pub ef_construction: usize,
    /// Search width during queries.
    pub ef_search: usize,
}

impl HnswIndexConfig {
    /// Create a config with the given dimension and max elements, using defaults
    /// for other parameters.
    #[must_use]
    pub fn new(dimension: usize, max_elements: usize) -> Self {
        Self {
            dimension,
            max_elements,
            m: DEFAULT_M,
            max_layer: DEFAULT_MAX_LAYER,
            ef_construction: DEFAULT_EF_CONSTRUCTION,
            ef_search: DEFAULT_EF_SEARCH,
        }
    }

    /// Set the M parameter (max connections per node per layer).
    #[must_use]
    pub fn with_m(mut self, m: usize) -> Self {
        self.m = m;
        self
    }

    /// Set the `ef_construction` parameter.
    #[must_use]
    pub fn with_ef_construction(mut self, ef: usize) -> Self {
        self.ef_construction = ef;
        self
    }

    /// Set the `ef_search` parameter.
    #[must_use]
    pub fn with_ef_search(mut self, ef: usize) -> Self {
        self.ef_search = ef;
        self
    }
}

/// Serializable ID mapping for persistence.
#[derive(Serialize, Deserialize)]
struct IdMapData {
    dim: usize,
    ef_search: usize,
    id_to_int: HashMap<String, usize>,
    deleted: Vec<usize>,
    next_id: usize,
    /// Name of the embedding model that produced these vectors.
    /// `None` for indexes created before model tracking was added.
    #[serde(default)]
    model_name: Option<String>,
}

impl HnswIndex {
    /// Create a new empty HNSW index with default parameters.
    ///
    /// Uses `M=16`, `max_layer=16`, `ef_construction=200`, `ef_search=50`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the dimension is zero.
    #[must_use = "constructors return a new value"]
    pub fn new(dimension: usize, max_elements: usize) -> Result<Self, IndexError> {
        Self::with_config(HnswIndexConfig::new(dimension, max_elements))
    }

    /// Set the embedding model name for this index.
    ///
    /// Persisted alongside the index data so that model changes can be
    /// detected on reload, preventing silent vector space mismatches.
    pub fn set_model_name(&self, name: &str) {
        if let Ok(mut inner) = self.inner.write() {
            inner.model_name = Some(name.to_owned());
        }
    }

    /// Return the embedding model name, if one has been set.
    ///
    /// Returns `None` for legacy indexes created before model tracking.
    #[must_use]
    pub fn model_name(&self) -> Option<String> {
        self.inner
            .read()
            .ok()
            .and_then(|inner| inner.model_name.clone())
    }

    /// Create a new HNSW index with custom configuration.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the dimension is zero.
    #[instrument(skip_all, fields(dim = config.dimension, max = config.max_elements, m = config.m))]
    #[must_use = "constructors return a new value"]
    pub fn with_config(config: HnswIndexConfig) -> Result<Self, IndexError> {
        if config.dimension == 0 {
            return Err(IndexError::EmbeddingFailed {
                reason: "vector dimension must be > 0".to_string(),
            });
        }

        let hnsw = Hnsw::new(
            config.m,
            config.max_elements,
            config.max_layer,
            config.ef_construction,
            DistCosine {},
        );

        debug!(
            dim = config.dimension,
            max_elements = config.max_elements,
            "created new HNSW index"
        );

        Ok(Self {
            inner: RwLock::new(HnswInner {
                hnsw,
                id_to_int: HashMap::new(),
                int_to_id: HashMap::new(),
                deleted: HashSet::new(),
                next_id: 0,
                dim: config.dimension,
                ef_search: config.ef_search,
                model_name: None,
            }),
        })
    }
}

impl VectorIndex for HnswIndex {
    fn insert(&self, id: &str, vector: &[f32]) -> Result<(), IndexError> {
        let mut inner = self.inner.write().map_err(|e| IndexError::QueryFailed {
            reason: format!("index lock poisoned: {e}"),
        })?;

        if vector.len() != inner.dim {
            return Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "vector dimension mismatch: expected {}, got {}",
                    inner.dim,
                    vector.len()
                ),
            });
        }

        // If ID already exists, soft-delete the old entry
        if let Some(&old_int_id) = inner.id_to_int.get(id) {
            inner.deleted.insert(old_int_id);
            inner.int_to_id.remove(&old_int_id);
        }

        // Assign a new integer ID
        let int_id = inner.next_id;
        inner.next_id += 1;

        inner.id_to_int.insert(id.to_string(), int_id);
        inner.int_to_id.insert(int_id, id.to_string());

        inner.hnsw.insert((vector, int_id));

        Ok(())
    }

    fn search_knn(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>, IndexError> {
        let inner = self.inner.read().map_err(|e| IndexError::QueryFailed {
            reason: format!("index lock poisoned: {e}"),
        })?;

        if query.len() != inner.dim {
            return Err(IndexError::QueryFailed {
                reason: format!(
                    "query dimension mismatch: expected {}, got {}",
                    inner.dim,
                    query.len()
                ),
            });
        }

        let live_count = inner.int_to_id.len();
        if live_count == 0 {
            return Ok(Vec::new());
        }

        // Request more results than k to account for deleted entries we'll filter out
        let search_k = k + inner.deleted.len();
        let neighbours: Vec<Neighbour> = inner.hnsw.search(query, search_k, inner.ef_search);

        let mut results: Vec<SearchResult> = neighbours
            .into_iter()
            .filter(|n| !inner.deleted.contains(&n.d_id))
            .filter_map(|n| {
                inner.int_to_id.get(&n.d_id).map(|string_id| SearchResult {
                    id: string_id.clone(),
                    distance: n.distance,
                })
            })
            .take(k)
            .collect();

        // Ensure results are sorted by distance (ascending)
        results.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    fn delete(&self, id: &str) -> Result<bool, IndexError> {
        let mut inner = self.inner.write().map_err(|e| IndexError::QueryFailed {
            reason: format!("index lock poisoned: {e}"),
        })?;

        if let Some(&int_id) = inner.id_to_int.get(id) {
            inner.deleted.insert(int_id);
            inner.int_to_id.remove(&int_id);
            inner.id_to_int.remove(id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[instrument(skip(self), fields(dir = %dir.display()))]
    fn persist(&self, dir: &Path) -> Result<(), IndexError> {
        let inner = self.inner.read().map_err(|e| IndexError::LoadFailed {
            path: dir.to_path_buf(),
            reason: format!("index lock poisoned: {e}"),
        })?;

        fs::create_dir_all(dir).map_err(|e| IndexError::LoadFailed {
            path: dir.to_path_buf(),
            reason: format!("failed to create directory: {e}"),
        })?;

        // Clean up stale HNSW dump files from previous persists.
        // `file_dump()` creates new files with a unique numeric suffix each
        // time, so old dump files accumulate indefinitely without this cleanup.
        let mut cleaned = 0usize;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with(DUMP_BASENAME)
                    && name_str.contains(".hnsw.")
                    && fs::remove_file(entry.path()).is_ok()
                {
                    cleaned += 1;
                }
            }
        }
        if cleaned > 0 {
            debug!(cleaned, "removed stale HNSW dump files before persist");
        }

        // Dump the HNSW graph and data
        inner
            .hnsw
            .file_dump(dir, DUMP_BASENAME)
            .map_err(|e| IndexError::LoadFailed {
                path: dir.to_path_buf(),
                reason: format!("failed to dump HNSW: {e}"),
            })?;

        // Save ID mapping as JSON sidecar
        let id_map = IdMapData {
            dim: inner.dim,
            ef_search: inner.ef_search,
            id_to_int: inner.id_to_int.clone(),
            deleted: inner.deleted.iter().copied().collect(),
            next_id: inner.next_id,
            model_name: inner.model_name.clone(),
        };

        let map_path = dir.join(ID_MAP_FILE);
        let file = File::create(&map_path).map_err(|e| IndexError::LoadFailed {
            path: map_path.clone(),
            reason: format!("failed to create ID map file: {e}"),
        })?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, &id_map).map_err(|e| IndexError::LoadFailed {
            path: map_path,
            reason: format!("failed to write ID map: {e}"),
        })?;

        info!(
            vectors = inner.int_to_id.len(),
            dir = %dir.display(),
            "persisted HNSW index"
        );

        Ok(())
    }

    fn len(&self) -> usize {
        self.inner
            .read()
            .map(|inner| inner.int_to_id.len())
            .unwrap_or(0)
    }

    fn dimension(&self) -> usize {
        self.inner.read().map(|inner| inner.dim).unwrap_or(0)
    }
}

impl VectorIndexLoad for HnswIndex {
    #[instrument(skip_all, fields(dir = %dir.display()))]
    fn load(dir: &Path) -> Result<Self, IndexError> {
        // Load ID mapping
        let map_path = dir.join(ID_MAP_FILE);
        let map_content = fs::read_to_string(&map_path).map_err(|e| IndexError::LoadFailed {
            path: map_path.clone(),
            reason: format!("failed to read ID map: {e}"),
        })?;
        let id_map: IdMapData =
            serde_json::from_str(&map_content).map_err(|e| IndexError::LoadFailed {
                path: map_path,
                reason: format!("failed to parse ID map: {e}"),
            })?;

        // Rebuild reverse mapping
        let int_to_id: HashMap<usize, String> = id_map
            .id_to_int
            .iter()
            .map(|(k, &v)| (v, k.clone()))
            .collect();
        let deleted: HashSet<usize> = id_map.deleted.into_iter().collect();

        // Load HNSW graph + data.
        // We leak the HnswIo because Hnsw<'b, T, D> borrows from it ('a: 'b).
        // This is intentional: the index is long-lived and loaded once per process.
        let hnsw_io = Box::leak(Box::new(HnswIo::new(dir, DUMP_BASENAME)));
        let hnsw: Hnsw<'static, f32, DistCosine> =
            hnsw_io.load_hnsw().map_err(|e| IndexError::LoadFailed {
                path: dir.to_path_buf(),
                reason: format!("failed to load HNSW: {e}"),
            })?;

        info!(
            dim = id_map.dim,
            vectors = int_to_id.len(),
            dir = %dir.display(),
            "loaded HNSW index"
        );

        Ok(Self {
            inner: RwLock::new(HnswInner {
                hnsw,
                id_to_int: id_map.id_to_int,
                int_to_id,
                deleted,
                next_id: id_map.next_id,
                dim: id_map.dim,
                ef_search: id_map.ef_search,
                model_name: id_map.model_name,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Construction ---

    #[test]
    fn create_empty_index() {
        let index = HnswIndex::new(384, 1000).unwrap();
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
        assert_eq!(index.dimension(), 384);
    }

    #[test]
    fn zero_dimension_rejected() {
        let err = HnswIndex::new(0, 1000).unwrap_err();
        assert!(err.to_string().contains("dimension must be > 0"));
    }

    #[test]
    fn custom_config() {
        let config = HnswIndexConfig::new(128, 5000)
            .with_m(32)
            .with_ef_construction(400)
            .with_ef_search(100);
        let index = HnswIndex::with_config(config).unwrap();
        assert_eq!(index.dimension(), 128);
    }

    // --- Insert & Search ---

    #[test]
    fn insert_and_search_single() {
        let dim = 8;
        let index = HnswIndex::new(dim, 100).unwrap();

        let vector = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        index.insert("vec-1", &vector).unwrap();

        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());

        let results = index.search_knn(&vector, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "vec-1");
        // Distance to self should be ~0 for cosine
        assert!(results[0].distance < 0.01);
    }

    #[test]
    fn insert_multiple_and_search() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();

        // Insert vectors pointing in different directions
        index.insert("north", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("east", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.insert("south", &[0.0, 0.0, 1.0, 0.0]).unwrap();

        assert_eq!(index.len(), 3);

        // Query close to "north"
        let results = index.search_knn(&[0.9, 0.1, 0.0, 0.0], 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "north");
    }

    #[test]
    fn search_empty_index() {
        let index = HnswIndex::new(4, 100).unwrap();
        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_k_larger_than_index() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();
        index.insert("only", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        // Ask for 10 results when only 1 exists
        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn dimension_mismatch_insert_rejected() {
        let index = HnswIndex::new(4, 100).unwrap();
        let err = index.insert("bad", &[1.0, 0.0]).unwrap_err();
        assert!(err.to_string().contains("dimension mismatch"));
    }

    #[test]
    fn dimension_mismatch_search_rejected() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        let err = index.search_knn(&[1.0, 0.0], 1).unwrap_err();
        assert!(err.to_string().contains("dimension mismatch"));
    }

    // --- Delete ---

    #[test]
    fn delete_existing() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);

        let deleted = index.delete("v1").unwrap();
        assert!(deleted);
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn delete_nonexistent() {
        let index = HnswIndex::new(4, 100).unwrap();
        let deleted = index.delete("nope").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn deleted_vectors_not_in_search_results() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();

        index.insert("keep", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("remove", &[0.9, 0.1, 0.0, 0.0]).unwrap();

        index.delete("remove").unwrap();

        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "keep");
    }

    // --- Replace (insert with existing ID) ---

    #[test]
    fn insert_replaces_existing_id() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();

        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("v1", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        // Should still have 1 live vector
        assert_eq!(index.len(), 1);

        let results = index.search_knn(&[0.0, 1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results[0].id, "v1");
        // Should be close to the new vector, not the old one
        assert!(results[0].distance < 0.01);
    }

    // --- Persistence ---

    #[test]
    fn persist_and_load_roundtrip() {
        let dim = 8;
        let index = HnswIndex::new(dim, 1000).unwrap();

        index
            .insert("alpha", &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("beta", &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("gamma", &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("test_index");

        // Persist
        index.persist(&index_dir).unwrap();

        // Verify files exist
        assert!(index_dir.join(ID_MAP_FILE).exists());

        // Load
        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(loaded.dimension(), dim);
        assert_eq!(loaded.len(), 3);

        // Search should work on loaded index
        let results = loaded
            .search_knn(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3)
            .unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "alpha");
    }

    #[test]
    fn persist_creates_directory() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("deep").join("nested").join("index");
        index.persist(&nested).unwrap();
        assert!(nested.join(ID_MAP_FILE).exists());
    }

    #[test]
    fn persist_cleans_up_stale_dump_files() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("cleanup_test");

        // Persist twice — second persist should clean up files from the first
        index.persist(&index_dir).unwrap();

        let count_before: usize = fs::read_dir(&index_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".hnsw.")
            })
            .count();
        assert!(count_before > 0);

        // Insert another vector so the dump files are different
        index.insert("v2", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.persist(&index_dir).unwrap();

        let count_after: usize = fs::read_dir(&index_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".hnsw.")
            })
            .count();

        // Should have exactly one pair (data + graph), not accumulating
        assert_eq!(count_after, count_before);

        // Verify the index still loads correctly after cleanup
        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn load_nonexistent_fails() {
        let err = HnswIndex::load(Path::new("/nonexistent/path")).unwrap_err();
        assert!(matches!(err, IndexError::LoadFailed { .. }));
    }

    // --- Model name tracking ---

    #[test]
    fn model_name_default_none() {
        let index = HnswIndex::new(4, 100).unwrap();
        assert!(index.model_name().is_none());
    }

    #[test]
    fn set_and_get_model_name() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.set_model_name("jina-embeddings-v2-base-code");
        assert_eq!(
            index.model_name().as_deref(),
            Some("jina-embeddings-v2-base-code")
        );
    }

    #[test]
    fn model_name_persists_roundtrip() {
        let dim = 4;
        let index = HnswIndex::new(dim, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.set_model_name("nomic-embed-text-v1.5");

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("model_name_test");
        index.persist(&index_dir).unwrap();

        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(
            loaded.model_name().as_deref(),
            Some("nomic-embed-text-v1.5")
        );
    }

    #[test]
    fn legacy_index_loads_without_model_name() {
        // Simulate a legacy id_map.json without the model_name field
        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("legacy_test");
        fs::create_dir_all(&index_dir).unwrap();

        // Create an index, persist it, then strip model_name from the JSON
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.persist(&index_dir).unwrap();

        // Read and re-write id_map.json without model_name
        let map_path = index_dir.join(ID_MAP_FILE);
        let content = fs::read_to_string(&map_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&content).unwrap();
        value.as_object_mut().unwrap().remove("model_name");
        fs::write(&map_path, serde_json::to_string(&value).unwrap()).unwrap();

        // Load should succeed with model_name = None
        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert!(loaded.model_name().is_none());
        assert_eq!(loaded.len(), 1);
    }

    // --- Trait object usage ---

    #[test]
    fn works_as_trait_object() {
        let index: Box<dyn VectorIndex> = Box::new(HnswIndex::new(4, 100).unwrap());
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);
        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results[0].id, "v1");
    }

    // --- Concurrent reads ---

    #[test]
    fn concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let dim = 4;
        let index = Arc::new(HnswIndex::new(dim, 100).unwrap());
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("v2", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let idx = Arc::clone(&index);
                thread::spawn(move || {
                    let results = idx.search_knn(&[1.0, 0.0, 0.0, 0.0], 2).unwrap();
                    assert_eq!(results.len(), 2);
                    assert_eq!(results[0].id, "v1");
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }
}
