//! [`InvertedIndex`] — sparse inverted index for keyword-level matching.
//!
//! Stores sparse vectors as posting lists (`term_id` → `[(doc_id, weight)]`).
//! Supports dot-product search by accumulating weights across query terms.
//! Persistence uses JSON serialization to a sidecar file alongside the HNSW index.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::error::IndexError;

use super::{SparseIndex, SparseSearchResult};

/// File name for the persisted sparse index.
const SPARSE_INDEX_FILE: &str = "sparse_index.json";

/// In-memory inverted index for sparse vector search.
///
/// Each term (identified by a `u32` vocabulary index) maps to a posting list
/// of (`document_id`, weight) pairs. Search computes dot products by iterating
/// over query terms and accumulating scores from matching postings.
///
/// Thread-safe via `RwLock` — concurrent reads, exclusive writes.
pub struct InvertedIndex {
    inner: RwLock<InvertedIndexInner>,
}

/// The serializable inner state of the inverted index.
#[derive(Debug, Serialize, Deserialize, Default)]
struct InvertedIndexInner {
    /// `term_id` → `[(doc_index, weight)]`
    /// We store doc IDs as indices into `doc_ids` for compactness.
    postings: BTreeMap<u32, Vec<Posting>>,
    /// All known document string IDs, indexed by position.
    doc_ids: Vec<String>,
    /// Reverse lookup: doc string ID → index in `doc_ids`.
    #[serde(skip)]
    doc_id_map: HashMap<String, usize>,
    /// Tombstoned doc indices — kept so `doc_ids` slots stay stable for
    /// posting-list references, but excluded from `len_sparse` and
    /// from search results. Re-inserting a deleted ID clears its tombstone.
    #[serde(default)]
    deleted: HashSet<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Posting {
    doc_idx: usize,
    weight: f32,
}

impl InvertedIndexInner {
    /// Rebuild the `doc_id_map` from `doc_ids` (needed after deserialization).
    fn rebuild_doc_id_map(&mut self) {
        self.doc_id_map = self
            .doc_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
    }

    /// Get or create a doc index for the given string ID.
    fn doc_index(&mut self, id: &str) -> usize {
        if let Some(&idx) = self.doc_id_map.get(id) {
            return idx;
        }
        let idx = self.doc_ids.len();
        self.doc_ids.push(id.to_string());
        self.doc_id_map.insert(id.to_string(), idx);
        idx
    }
}

impl InvertedIndex {
    /// Create a new empty inverted index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(InvertedIndexInner::default()),
        }
    }
}

impl Default for InvertedIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseIndex for InvertedIndex {
    #[instrument(skip(self, indices, values), fields(id, terms = indices.len()))]
    fn insert_sparse(&self, id: &str, indices: &[u32], values: &[f32]) -> Result<(), IndexError> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("sparse index lock poisoned: {e}"),
            })?;

        // Remove old postings for this doc if it exists
        if let Some(&doc_idx) = inner.doc_id_map.get(id) {
            for postings in inner.postings.values_mut() {
                postings.retain(|p| p.doc_idx != doc_idx);
            }
        }

        let doc_idx = inner.doc_index(id);
        // Re-inserting an ID revives it from the tombstone set so
        // len_sparse and search treat it as present again.
        inner.deleted.remove(&doc_idx);

        for (&term_id, &weight) in indices.iter().zip(values.iter()) {
            inner
                .postings
                .entry(term_id)
                .or_default()
                .push(Posting { doc_idx, weight });
        }

        Ok(())
    }

    fn search_sparse(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<Vec<SparseSearchResult>, IndexError> {
        let inner = self.inner.read().map_err(|e| IndexError::QueryFailed {
            reason: format!("sparse index lock poisoned: {e}"),
        })?;

        // Accumulate dot-product scores per document
        let mut scores: HashMap<usize, f32> = HashMap::new();

        for (&term_id, &query_weight) in query_indices.iter().zip(query_values.iter()) {
            if let Some(postings) = inner.postings.get(&term_id) {
                for posting in postings {
                    *scores.entry(posting.doc_idx).or_default() += query_weight * posting.weight;
                }
            }
        }

        // Sort by score descending, take top k
        let mut results: Vec<(usize, f32)> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);

        Ok(results
            .into_iter()
            .map(|(doc_idx, score)| SparseSearchResult {
                id: inner.doc_ids[doc_idx].clone(),
                score,
            })
            .collect())
    }

    fn delete_sparse(&self, id: &str) -> Result<bool, IndexError> {
        let mut inner = self.inner.write().map_err(|e| IndexError::QueryFailed {
            reason: format!("sparse index lock poisoned: {e}"),
        })?;

        let Some(&doc_idx) = inner.doc_id_map.get(id) else {
            return Ok(false);
        };
        // Already tombstoned — treat as not-present so callers that
        // check the return value (e.g. to decide whether to emit a
        // coherence event) don't double-fire.
        if inner.deleted.contains(&doc_idx) {
            return Ok(false);
        }

        // Remove from all posting lists
        for postings in inner.postings.values_mut() {
            postings.retain(|p| p.doc_idx != doc_idx);
        }

        // Remove empty posting lists
        inner.postings.retain(|_, v| !v.is_empty());

        // Tombstone the doc_idx so doc_ids stays aligned with posting
        // references but len_sparse/search exclude it.
        inner.deleted.insert(doc_idx);

        Ok(true)
    }

    #[instrument(skip(self))]
    fn persist_sparse(&self, dir: &Path) -> Result<(), IndexError> {
        let inner = self.inner.read().map_err(|e| IndexError::LoadFailed {
            path: PathBuf::from(dir),
            reason: format!("sparse index lock poisoned: {e}"),
        })?;

        let path = dir.join(SPARSE_INDEX_FILE);

        std::fs::create_dir_all(dir).map_err(|e| IndexError::LoadFailed {
            path: PathBuf::from(dir),
            reason: format!("failed to create directory: {e}"),
        })?;

        let json = serde_json::to_string(&*inner).map_err(|e| IndexError::LoadFailed {
            path: path.clone(),
            reason: format!("failed to serialize sparse index: {e}"),
        })?;

        std::fs::write(&path, json).map_err(|e| IndexError::LoadFailed {
            path: path.clone(),
            reason: format!("failed to write sparse index: {e}"),
        })?;

        debug!(path = %path.display(), docs = inner.doc_ids.len(), "sparse index persisted");
        Ok(())
    }

    fn load_sparse(dir: &Path) -> Result<Self, IndexError> {
        let path = dir.join(SPARSE_INDEX_FILE);

        if !path.exists() {
            return Ok(Self::new());
        }

        let json = std::fs::read_to_string(&path).map_err(|e| IndexError::LoadFailed {
            path: path.clone(),
            reason: format!("failed to read sparse index: {e}"),
        })?;

        let mut inner: InvertedIndexInner =
            serde_json::from_str(&json).map_err(|e| IndexError::LoadFailed {
                path: path.clone(),
                reason: format!("failed to deserialize sparse index: {e}"),
            })?;

        inner.rebuild_doc_id_map();

        debug!(path = %path.display(), docs = inner.doc_ids.len(), "sparse index loaded");
        Ok(Self {
            inner: RwLock::new(inner),
        })
    }

    fn len_sparse(&self) -> usize {
        self.inner.read().map_or(0, |inner| {
            inner.doc_ids.len().saturating_sub(inner.deleted.len())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_search() {
        let index = InvertedIndex::new();

        // Doc 1: terms 10, 20 with weights
        index.insert_sparse("doc1", &[10, 20], &[1.0, 0.5]).unwrap();
        // Doc 2: terms 20, 30 with weights
        index.insert_sparse("doc2", &[20, 30], &[0.8, 1.0]).unwrap();

        // Query for term 20 — both docs should match
        let results = index.search_sparse(&[20], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 2);

        // Doc2 has weight 0.8 for term 20, Doc1 has 0.5
        assert_eq!(results[0].id, "doc2");
        assert_eq!(results[1].id, "doc1");
    }

    #[test]
    fn search_multiple_terms() {
        let index = InvertedIndex::new();

        index.insert_sparse("doc1", &[10, 20], &[1.0, 0.5]).unwrap();
        index.insert_sparse("doc2", &[20, 30], &[0.8, 1.0]).unwrap();

        // Query for terms 10 and 20 — doc1 matches both, doc2 only term 20
        let results = index.search_sparse(&[10, 20], &[1.0, 1.0], 10).unwrap();
        assert_eq!(results.len(), 2);
        // doc1: 1.0*1.0 + 0.5*1.0 = 1.5, doc2: 0.8*1.0 = 0.8
        assert_eq!(results[0].id, "doc1");
        assert!((results[0].score - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn search_empty_index() {
        let index = InvertedIndex::new();
        let results = index.search_sparse(&[10], &[1.0], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn search_respects_top_k() {
        let index = InvertedIndex::new();
        for i in 0..10_i32 {
            index
                .insert_sparse(&format!("doc{i}"), &[1], &[i as f32])
                .unwrap();
        }
        let results = index.search_sparse(&[1], &[1.0], 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn delete_removes_from_postings() {
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10, 20], &[1.0, 0.5]).unwrap();
        index.insert_sparse("doc2", &[10], &[0.8]).unwrap();

        assert!(index.delete_sparse("doc1").unwrap());
        assert!(!index.delete_sparse("nonexistent").unwrap());

        let results = index.search_sparse(&[10], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc2");
    }

    #[test]
    fn reinsert_updates_postings() {
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10], &[1.0]).unwrap();
        // Re-insert with different weight
        index.insert_sparse("doc1", &[10], &[2.0]).unwrap();

        let results = index.search_sparse(&[10], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn persist_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10, 20], &[1.0, 0.5]).unwrap();
        index.insert_sparse("doc2", &[20, 30], &[0.8, 1.0]).unwrap();

        index.persist_sparse(dir.path()).unwrap();

        let loaded = InvertedIndex::load_sparse(dir.path()).unwrap();
        let results = loaded.search_sparse(&[20], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "doc2");
    }

    #[test]
    fn load_nonexistent_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = InvertedIndex::load_sparse(dir.path()).unwrap();
        assert_eq!(loaded.len_sparse(), 0);
    }

    #[test]
    fn is_empty_sparse() {
        let index = InvertedIndex::new();
        assert!(index.is_empty_sparse());
        index.insert_sparse("doc1", &[10], &[1.0]).unwrap();
        assert!(!index.is_empty_sparse());
    }

    #[test]
    fn delete_decrements_len_sparse() {
        // Regression: delete_sparse used to clear postings but leave the
        // doc_ids / doc_id_map entries behind, so len_sparse kept
        // reporting the deleted doc.
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10], &[1.0]).unwrap();
        index.insert_sparse("doc2", &[10], &[1.0]).unwrap();
        assert_eq!(index.len_sparse(), 2);
        assert!(index.delete_sparse("doc1").unwrap());
        assert_eq!(index.len_sparse(), 1);
    }

    #[test]
    fn delete_then_persist_and_load_excludes_deleted_doc() {
        // Regression: the persisted sidecar used to carry deleted doc IDs
        // forward across load, so a reload would still "know about" docs
        // whose postings were gone.
        let dir = tempfile::tempdir().unwrap();
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10], &[1.0]).unwrap();
        index.insert_sparse("doc2", &[10], &[1.0]).unwrap();
        index.delete_sparse("doc1").unwrap();
        index.persist_sparse(dir.path()).unwrap();

        let loaded = InvertedIndex::load_sparse(dir.path()).unwrap();
        assert_eq!(loaded.len_sparse(), 1);
        let results = loaded.search_sparse(&[10], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc2");
        assert!(results.iter().all(|r| r.id != "doc1"));
    }

    #[test]
    fn delete_then_reinsert_works() {
        // A tombstoned entry must not block a fresh insert of the same ID.
        let index = InvertedIndex::new();
        index.insert_sparse("doc1", &[10], &[1.0]).unwrap();
        index.delete_sparse("doc1").unwrap();
        index.insert_sparse("doc1", &[10], &[2.0]).unwrap();
        assert_eq!(index.len_sparse(), 1);
        let results = index.search_sparse(&[10], &[1.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc1");
        assert!((results[0].score - 2.0).abs() < f32::EPSILON);
    }
}
