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
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw, HnswIo, Neighbour};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use crate::error::IndexError;
use crate::fs_util;

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

/// The crash-recovery sibling left by an interrupted atomic [`persist`].
///
/// [`persist`] swaps a fully-staged index into `dir` via a rename dance
/// (`dir` → `dir.bak`, `tmp` → `dir`, rm `dir.bak`). If the process dies
/// mid-swap, `<dir>.bak` still holds the previous *consistent* index and
/// [`load`] falls back to it.
///
/// [`persist`]: HnswIndex::persist
/// [`load`]: HnswIndex::load
fn backup_dir(dir: &Path) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let name = dir.file_name()?;
    let mut bak = name.to_os_string();
    bak.push(".bak");
    Some(parent.join(bak))
}

/// Whether `dir` contains a readable ID-map sidecar (the sentinel for a
/// complete on-disk index).
fn has_id_map(dir: &Path) -> bool {
    dir.join(ID_MAP_FILE).is_file()
}

/// Stage a complete, durably-flushed index into `tmp`.
///
/// Dumps the HNSW graph, writes the ID-map sidecar, then fsyncs every
/// staged file and the directory itself so the subsequent rename swap
/// can never expose a torn file after a power loss.
fn stage_index(inner: &HnswInner, tmp: &Path) -> Result<(), IndexError> {
    inner
        .hnsw
        .file_dump(tmp, DUMP_BASENAME)
        .map_err(|e| IndexError::LoadFailed {
            path: tmp.to_path_buf(),
            reason: format!("failed to dump HNSW: {e}"),
        })?;

    let id_map = IdMapData {
        dim: inner.dim,
        ef_search: inner.ef_search,
        id_to_int: inner.id_to_int.clone(),
        deleted: inner.deleted.iter().copied().collect(),
        next_id: inner.next_id,
        model_name: inner.model_name.clone(),
    };
    let map_path = tmp.join(ID_MAP_FILE);
    {
        let file = File::create(&map_path).map_err(|e| IndexError::LoadFailed {
            path: map_path.clone(),
            reason: format!("failed to create ID map file: {e}"),
        })?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &id_map).map_err(|e| IndexError::LoadFailed {
            path: map_path.clone(),
            reason: format!("failed to write ID map: {e}"),
        })?;
        writer.flush().map_err(|e| IndexError::LoadFailed {
            path: map_path.clone(),
            reason: format!("failed to flush ID map: {e}"),
        })?;
    }

    if let Ok(entries) = fs::read_dir(tmp) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                fs_util::fsync_file(&p).map_err(|e| IndexError::LoadFailed {
                    path: p.clone(),
                    reason: format!("failed to fsync staged file: {e}"),
                })?;
            }
        }
    }
    fs_util::fsync_dir(tmp).map_err(|e| IndexError::LoadFailed {
        path: tmp.to_path_buf(),
        reason: format!("failed to fsync temp directory: {e}"),
    })
}

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
            .map_or((0, 0), |inner| (inner.int_to_id.len(), inner.dim));
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

    /// Fail fast if this index is incompatible with the active embedder.
    ///
    /// A dimension mismatch is always incompatible — the stored vectors
    /// cannot be searched with `expected_dim`-wide queries. A model-name
    /// mismatch is incompatible when a name was stored (different model =
    /// different vector space). A legacy index with no stored model name
    /// is *adopted*: this returns `Ok(())` and the caller should
    /// [`set_model_name`](Self::set_model_name) to stamp it.
    ///
    /// `path` is only used to populate [`IndexError::ModelMismatch`].
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::ModelMismatch`] when the stored dimension or
    /// model name disagrees with the active embedder.
    pub fn check_compatible(
        &self,
        expected_dim: usize,
        expected_model: &str,
        path: &Path,
    ) -> Result<(), IndexError> {
        let stored_dim = self.dimension();
        let stored_model = self.model_name();
        let model_incompatible = matches!(
            crate::embedding::check_model_compatibility(expected_model, stored_model.as_deref(),),
            crate::embedding::ModelCompatibility::IncompatibleModel { .. }
        );
        if stored_dim != expected_dim || model_incompatible {
            return Err(IndexError::ModelMismatch {
                path: path.to_path_buf(),
                stored_dim,
                expected_dim,
                stored_model,
                expected_model: expected_model.to_owned(),
            });
        }
        Ok(())
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

    /// Atomically persist the index.
    ///
    /// The full new index (HNSW dump + ID-map sidecar) is staged and
    /// fsynced in a fresh sibling temp dir, then swapped into place with
    /// a rename dance: `dir` → `dir.bak`, `tmp` → `dir`, rm `dir.bak`.
    /// A crash at any step leaves either the untouched prior `dir` or a
    /// consistent `dir.bak` (which [`load`](Self::load) recovers from) —
    /// never a half-written index or dumps without a matching id-map.
    #[instrument(skip(self), fields(dir = %dir.display()))]
    fn persist(&self, dir: &Path) -> Result<(), IndexError> {
        let inner = self.inner.read().map_err(|e| IndexError::LoadFailed {
            path: dir.to_path_buf(),
            reason: format!("index lock poisoned: {e}"),
        })?;

        let parent = dir.parent().ok_or_else(|| IndexError::LoadFailed {
            path: dir.to_path_buf(),
            reason: "index directory has no parent".to_owned(),
        })?;
        let dir_name = dir
            .file_name()
            .ok_or_else(|| IndexError::LoadFailed {
                path: dir.to_path_buf(),
                reason: "index directory has no final component".to_owned(),
            })?
            .to_string_lossy()
            .into_owned();
        let bak = backup_dir(dir).ok_or_else(|| IndexError::LoadFailed {
            path: dir.to_path_buf(),
            reason: "cannot derive backup path".to_owned(),
        })?;

        fs::create_dir_all(parent).map_err(|e| IndexError::LoadFailed {
            path: parent.to_path_buf(),
            reason: format!("failed to create parent directory: {e}"),
        })?;

        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp = parent.join(format!("{dir_name}.tmp.{pid}.{nanos}"));

        // A leftover tmp from a previously-killed persist must not
        // poison this one.
        let _ = fs_util::remove_dir_all_robust_sync(&tmp);
        fs::create_dir_all(&tmp).map_err(|e| IndexError::LoadFailed {
            path: tmp.clone(),
            reason: format!("failed to create temp directory: {e}"),
        })?;

        if let Err(e) = stage_index(&inner, &tmp) {
            let _ = fs_util::remove_dir_all_robust_sync(&tmp);
            return Err(e);
        }

        // Atomic swap. Clear any stale backup first so the renames have
        // a clear target.
        let _ = fs_util::remove_dir_all_robust_sync(&bak);
        let dir_existed = dir.exists();
        if dir_existed && let Err(e) = fs_util::rename_robust(dir, &bak) {
            let _ = fs_util::remove_dir_all_robust_sync(&tmp);
            return Err(IndexError::LoadFailed {
                path: dir.to_path_buf(),
                reason: format!("failed to move current index aside: {e}"),
            });
        }
        if let Err(e) = fs_util::rename_robust(&tmp, dir) {
            // Roll back so the corpus is never left without a live dir.
            if dir_existed {
                let _ = fs_util::rename_robust(&bak, dir);
            }
            let _ = fs_util::remove_dir_all_robust_sync(&tmp);
            return Err(IndexError::LoadFailed {
                path: dir.to_path_buf(),
                reason: format!("failed to swap new index into place: {e}"),
            });
        }

        // New index is durably in place; drop the backup and flush the
        // parent so the rename pair itself is durable.
        let _ = fs_util::remove_dir_all_robust_sync(&bak);
        let _ = fs_util::fsync_dir(parent);

        info!(
            vectors = inner.int_to_id.len(),
            dir = %dir.display(),
            "persisted HNSW index"
        );

        Ok(())
    }

    fn len(&self) -> usize {
        self.inner.read().map_or(0, |inner| inner.int_to_id.len())
    }

    fn dimension(&self) -> usize {
        self.inner.read().map_or(0, |inner| inner.dim)
    }
}

impl VectorIndexLoad for HnswIndex {
    #[instrument(skip_all, fields(dir = %dir.display()))]
    fn load(dir: &Path) -> Result<Self, IndexError> {
        // Prefer the live directory; fall back to the crash-recovery
        // backup left by an interrupted atomic persist. Either way the
        // index is loaded exactly once per process (the `Box::leak`
        // contract below is unaffected — only the source path differs).
        let src = if has_id_map(dir) {
            dir.to_path_buf()
        } else if let Some(bak) = backup_dir(dir).filter(|b| has_id_map(b)) {
            warn!(
                dir = %dir.display(),
                bak = %bak.display(),
                "primary index missing id_map; recovering from backup"
            );
            bak
        } else {
            return Err(IndexError::LoadFailed {
                path: dir.join(ID_MAP_FILE),
                reason: "no id_map.json in index directory or backup".to_owned(),
            });
        };
        let dir = src.as_path();

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

        // Seed enough vectors that the HNSW graph's random layer
        // assignment can't leave a node unreachable from the entry
        // point. With just 2–3 nodes the layer distribution is
        // degenerate enough to occasionally drop a neighbor from
        // search results on Windows under parallel-test CPU load —
        // we've been bitten by this flake before.
        //
        // All vectors are orthogonal / far-apart axis-aligned or
        // clearly-separated directions so there's no ambiguity about
        // which one is closest to the query `[1, 0, 0, 0]`.
        index.insert("north", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("east", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.insert("south", &[0.0, 0.0, 1.0, 0.0]).unwrap();
        index.insert("up", &[0.0, 0.0, 0.0, 1.0]).unwrap();
        index.insert("west", &[-1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("down", &[0.0, 0.0, 0.0, -1.0]).unwrap();
        let s = std::f32::consts::FRAC_1_SQRT_2; // ≈ 0.7071, 45° component
        index.insert("ne", &[s, s, 0.0, 0.0]).unwrap();
        index.insert("se", &[0.0, s, s, 0.0]).unwrap();

        assert_eq!(index.len(), 8);

        // Query on the north axis — north must win top-1 unambiguously.
        // Asking for 3 results still exercises the k-NN codepath; we
        // assert on membership + ordering, not on getting exactly N
        // results, because HNSW is approximate by design.
        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 3).unwrap();
        assert!(
            !results.is_empty(),
            "approximate k-NN must return at least one result"
        );
        assert_eq!(
            results[0].id, "north",
            "top-1 must be north for an on-axis query: {results:?}"
        );
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

        // Padding vectors give the HNSW graph enough structure that
        // its probabilistic layer assignment can't leave "keep"
        // unreachable after we delete "remove". The real assertion is
        // that deleted ids are filtered from results — not that the
        // graph returns every live node when k is large.
        index.insert("keep", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        index.insert("remove", &[0.9, 0.1, 0.0, 0.0]).unwrap();
        index.insert("pad_e", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.insert("pad_s", &[0.0, 0.0, 1.0, 0.0]).unwrap();
        index.insert("pad_u", &[0.0, 0.0, 0.0, 1.0]).unwrap();
        index.insert("pad_w", &[-1.0, 0.0, 0.0, 0.0]).unwrap();

        index.delete("remove").unwrap();

        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        assert!(
            !results.iter().any(|r| r.id == "remove"),
            "deleted id should not surface in results: {results:?}"
        );
        assert!(
            results.iter().any(|r| r.id == "keep"),
            "live ids should still be returned: {results:?}"
        );
        assert_eq!(
            results[0].id, "keep",
            "top-1 for on-axis query must be keep: {results:?}"
        );
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

        // Seed enough axis-aligned vectors that the HNSW graph has a
        // proper layer structure after load. With only 2–3 nodes the
        // random layer assignment can leave a node unreachable from
        // the entry point under parallel-test load, and search then
        // returns fewer than k neighbors — see `insert_multiple_and_search`.
        index
            .insert("alpha", &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("beta", &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("gamma", &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("delta", &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("epsilon", &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("zeta", &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0])
            .unwrap();
        index
            .insert("eta", &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
            .unwrap();
        index
            .insert("theta", &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0])
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
        assert_eq!(loaded.len(), 8);

        // Search should work on loaded index. Assert top-1 ordering
        // (the real test of the round-trip) rather than an exact
        // result-count, since HNSW is approximate by design.
        let results = loaded
            .search_knn(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 3)
            .unwrap();
        assert!(
            !results.is_empty(),
            "loaded index must return at least one result"
        );
        assert_eq!(
            results[0].id, "alpha",
            "top-1 must be alpha after round-trip: {results:?}"
        );
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

    #[test]
    fn load_recovers_from_backup_when_primary_torn() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("idx");
        index.persist(&index_dir).unwrap();

        // Simulate a crash mid-swap: the consistent index is in
        // `<dir>.bak`, and `dir` is a torn directory with no id-map.
        let bak = backup_dir(&index_dir).unwrap();
        fs::rename(&index_dir, &bak).unwrap();
        fs::create_dir_all(&index_dir).unwrap();
        fs::write(index_dir.join("ministr_hnsw.hnsw.data"), b"garbage").unwrap();

        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_recovers_from_backup_when_primary_missing() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("idx");
        index.persist(&index_dir).unwrap();

        let bak = backup_dir(&index_dir).unwrap();
        fs::rename(&index_dir, &bak).unwrap();
        assert!(!index_dir.exists());

        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn persist_leaves_no_backup_or_temp_on_success() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let index_dir = tmp.path().join("idx");
        index.persist(&index_dir).unwrap();
        index.insert("v2", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.persist(&index_dir).unwrap();

        let bak = backup_dir(&index_dir).unwrap();
        assert!(
            !bak.exists(),
            "backup must be removed after a successful persist"
        );
        let leftover_tmp = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover_tmp, "no .tmp staging dir should remain");

        let loaded = HnswIndex::load(&index_dir).unwrap();
        assert_eq!(loaded.len(), 2);
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

    // --- Model/dimension compatibility guard ---

    #[test]
    fn check_compatible_ok_when_dim_and_model_match() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.set_model_name("all-MiniLM-L6-v2");
        index
            .check_compatible(4, "all-MiniLM-L6-v2", Path::new("idx"))
            .unwrap();
    }

    #[test]
    fn check_compatible_rejects_dimension_mismatch() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.set_model_name("all-MiniLM-L6-v2");
        let err = index
            .check_compatible(8, "all-MiniLM-L6-v2", Path::new("idx"))
            .unwrap_err();
        match err {
            IndexError::ModelMismatch {
                stored_dim,
                expected_dim,
                ..
            } => {
                assert_eq!(stored_dim, 4);
                assert_eq!(expected_dim, 8);
            }
            other => panic!("expected ModelMismatch, got {other:?}"),
        }
    }

    #[test]
    fn check_compatible_rejects_model_mismatch() {
        let index = HnswIndex::new(4, 100).unwrap();
        index.set_model_name("all-MiniLM-L6-v2");
        let err = index
            .check_compatible(4, "bge-small-en-v1.5", Path::new("idx"))
            .unwrap_err();
        assert!(matches!(err, IndexError::ModelMismatch { .. }));
    }

    #[test]
    fn check_compatible_adopts_legacy_index_without_model() {
        // No stored model name (legacy) — same dim — is compatible.
        let index = HnswIndex::new(4, 100).unwrap();
        assert!(index.model_name().is_none());
        index
            .check_compatible(4, "all-MiniLM-L6-v2", Path::new("idx"))
            .unwrap();
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

    /// Hammer `search_knn` from many threads with identical queries.
    ///
    /// Previously this test inserted only two vectors and asserted the
    /// top-1 ordering — but with only two nodes the HNSW graph is
    /// degenerate enough that the approximate search can return
    /// neighbors in an order that's sensitive to thread scheduling
    /// under parallel-test CPU contention on Windows. The result was
    /// a genuinely flaky test that would pass in isolation and fail
    /// under `cargo test` load.
    ///
    /// Robustness changes:
    /// 1. Seed a denser corpus so v1 (== query) is unambiguous across
    ///    every layer of the HNSW graph.
    /// 2. Run many iterations per thread to actually exercise the read
    ///    lock under contention — not just one-shot-and-done.
    /// 3. Collect every thread's result set and cross-check that they
    ///    are *identical* — the point of the test is to verify
    ///    concurrent reads don't corrupt search state. If they did,
    ///    different threads would see different neighbor sets.
    #[test]
    fn concurrent_reads() {
        use std::sync::Arc;
        use std::sync::Mutex;
        use std::thread;

        const THREADS: usize = 8;
        const ITERS_PER_THREAD: usize = 50;

        let dim = 4;
        let index = Arc::new(HnswIndex::new(dim, 100).unwrap());
        // v1 is identical to the query — cosine distance ≈ 0.
        index.insert("v1", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        // Near-neighbor: cosine distance small but nonzero.
        index.insert("v1_near", &[0.95, 0.05, 0.0, 0.0]).unwrap();
        // Orthogonal axis vectors — distance ≈ 1.
        index.insert("v2", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        index.insert("v3", &[0.0, 0.0, 1.0, 0.0]).unwrap();
        index.insert("v4", &[0.0, 0.0, 0.0, 1.0]).unwrap();
        // A few mixed vectors so the graph has non-trivial structure.
        index.insert("mix_ab", &[0.7, 0.7, 0.0, 0.0]).unwrap();
        index.insert("mix_cd", &[0.0, 0.0, 0.7, 0.7]).unwrap();

        let query = [1.0_f32, 0.0, 0.0, 0.0];
        let observed: Arc<Mutex<Vec<Vec<String>>>> =
            Arc::new(Mutex::new(Vec::with_capacity(THREADS)));

        let handles: Vec<_> = (0..THREADS)
            .map(|tid| {
                let idx = Arc::clone(&index);
                let observed = Arc::clone(&observed);
                thread::spawn(move || {
                    let mut last_ids: Option<Vec<String>> = None;
                    for iter in 0..ITERS_PER_THREAD {
                        let results = idx.search_knn(&query, 3).unwrap();
                        assert_eq!(
                            results.len(),
                            3,
                            "thread {tid} iter {iter}: expected 3 results, got {results:?}"
                        );
                        // v1 is the query itself (cosine ≈ 0) so it must
                        // always win top-1. If it doesn't, something is
                        // genuinely corrupting the search — not just
                        // a scheduling artifact.
                        assert_eq!(
                            results[0].id, "v1",
                            "thread {tid} iter {iter}: v1 must be nearest (distance {}), got {results:?}",
                            results[0].distance
                        );

                        let ids: Vec<String> =
                            results.iter().map(|r| r.id.clone()).collect();
                        if let Some(prev) = &last_ids {
                            assert_eq!(
                                prev, &ids,
                                "thread {tid} iter {iter}: result set changed between searches \
                                 on a static index — concurrent reads are corrupting state. \
                                 prev={prev:?} now={ids:?}"
                            );
                        }
                        last_ids = Some(ids);
                    }
                    // Record this thread's final ordering for the
                    // cross-thread consistency check below.
                    observed
                        .lock()
                        .expect("observed mutex poisoned")
                        .push(last_ids.expect("at least one iteration ran"));
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // All threads must have observed the exact same result set — a
        // static (read-only) index under concurrent readers cannot
        // return different neighbors to different callers.
        let observed = observed.lock().expect("observed mutex poisoned");
        let first = &observed[0];
        for (tid, other) in observed.iter().enumerate().skip(1) {
            assert_eq!(
                first, other,
                "thread 0 and thread {tid} disagree on result ordering: \
                 t0={first:?} t{tid}={other:?}"
            );
        }
    }
}
