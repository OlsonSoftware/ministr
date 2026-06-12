//! Exact (brute-force) vector index with fully deterministic results.
//!
//! [`ExactScanIndex`] exists for one purpose: a retrieval-quality eval gate
//! that is byte-identical across runs. HNSW graph construction is not
//! deterministic — the same ingest can rank equal-quality neighbors
//! differently run to run (measured: 4 distinct eval outputs in 6 runs,
//! ±0.01 on aggregate metrics) — so any acceptance gate built on it cannot
//! distinguish a real regression from graph noise. The exact scan removes
//! that noise at full timing parity at eval scale (a few hundred vectors),
//! where model load dominates the wall clock.
//!
//! Not intended for production corpora: search is O(n) per query.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::RwLock;

use crate::error::IndexError;

use super::{SearchResult, VectorIndex};

/// Brute-force cosine-distance index with fully deterministic results.
///
/// - Storage is a `BTreeMap<String, Vec<f32>>`, so iteration order is the id
///   order, independent of insertion order.
/// - Distance matches the HNSW backend's cosine: `1 - cos(q, v)`.
/// - Ties are broken by id ascending, so equal-distance results are stable.
#[derive(Debug)]
pub struct ExactScanIndex {
    dim: usize,
    vectors: RwLock<BTreeMap<String, Vec<f32>>>,
}

impl ExactScanIndex {
    /// Create an empty index for vectors of dimension `dim`.
    #[must_use]
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            vectors: RwLock::new(BTreeMap::new()),
        }
    }
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 1.0;
    }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}

impl VectorIndex for ExactScanIndex {
    fn insert(&self, id: &str, vector: &[f32]) -> Result<(), IndexError> {
        if vector.len() != self.dim {
            return Err(IndexError::EmbeddingFailed {
                reason: format!("dimension mismatch: {} != {}", vector.len(), self.dim),
            });
        }
        // Mirror the HNSW zero/non-finite guard so degenerate vectors can't
        // poison every top-k.
        if !vector.iter().all(|v| v.is_finite()) || vector.iter().all(|v| *v == 0.0) {
            return Ok(());
        }
        self.vectors
            .write()
            .expect("exact index lock poisoned")
            .insert(id.to_string(), vector.to_vec());
        Ok(())
    }

    fn search_knn(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>, IndexError> {
        let vectors = self.vectors.read().expect("exact index lock poisoned");
        let mut results: Vec<SearchResult> = vectors
            .iter()
            .map(|(id, v)| SearchResult {
                id: id.clone(),
                distance: cosine_distance(query, v),
            })
            .collect();
        results.sort_by(|a, b| {
            a.distance
                .total_cmp(&b.distance)
                .then_with(|| a.id.cmp(&b.id))
        });
        results.truncate(k);
        Ok(results)
    }

    fn delete(&self, id: &str) -> Result<bool, IndexError> {
        Ok(self
            .vectors
            .write()
            .expect("exact index lock poisoned")
            .remove(id)
            .is_some())
    }

    fn persist(&self, _dir: &Path) -> Result<(), IndexError> {
        // Eval-only index; nothing to persist.
        Ok(())
    }

    fn len(&self) -> usize {
        self.vectors
            .read()
            .expect("exact index lock poisoned")
            .len()
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_deterministic_regardless_of_insertion_order() {
        // Identical (id, vector) sets inserted in opposite orders must
        // produce identical ranked output.
        let fwd = ExactScanIndex::new(2);
        let rev = ExactScanIndex::new(2);
        let entries: &[(&str, [f32; 2])] = &[
            ("a", [1.0, 0.0]),
            ("b", [0.9, 0.1]),
            ("c", [0.0, 1.0]),
            ("d", [0.5, 0.5]),
        ];
        for (id, v) in entries {
            fwd.insert(id, v).unwrap();
        }
        for (id, v) in entries.iter().rev() {
            rev.insert(id, v).unwrap();
        }
        let q = [1.0, 0.0];
        assert_eq!(
            fwd.search_knn(&q, 4).unwrap(),
            rev.search_knn(&q, 4).unwrap()
        );
    }

    #[test]
    fn equal_distance_ties_break_by_id_ascending() {
        let index = ExactScanIndex::new(2);
        // Same direction → identical cosine distance to the query.
        index.insert("z", &[2.0, 0.0]).unwrap();
        index.insert("a", &[1.0, 0.0]).unwrap();
        index.insert("m", &[3.0, 0.0]).unwrap();
        let results = index.search_knn(&[1.0, 0.0], 3).unwrap();
        let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "m", "z"]);
    }

    #[test]
    fn cosine_distance_matches_hand_computed_values() {
        let index = ExactScanIndex::new(2);
        index.insert("same", &[2.0, 0.0]).unwrap();
        index.insert("orthogonal", &[0.0, 1.0]).unwrap();
        index.insert("opposite", &[-1.0, 0.0]).unwrap();
        let results = index.search_knn(&[1.0, 0.0], 3).unwrap();
        assert_eq!(results[0].id, "same");
        assert!((results[0].distance - 0.0).abs() < 1e-6);
        assert_eq!(results[1].id, "orthogonal");
        assert!((results[1].distance - 1.0).abs() < 1e-6);
        assert_eq!(results[2].id, "opposite");
        assert!((results[2].distance - 2.0).abs() < 1e-6);
    }

    #[test]
    fn zero_and_non_finite_vectors_are_silently_skipped() {
        let index = ExactScanIndex::new(2);
        index.insert("zero", &[0.0, 0.0]).unwrap();
        index.insert("nan", &[f32::NAN, 1.0]).unwrap();
        index.insert("inf", &[f32::INFINITY, 1.0]).unwrap();
        index.insert("ok", &[1.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);
        let results = index.search_knn(&[1.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "ok");
    }

    #[test]
    fn dimension_mismatch_is_rejected() {
        let index = ExactScanIndex::new(3);
        assert!(index.insert("bad", &[1.0, 2.0]).is_err());
        assert_eq!(index.dimension(), 3);
    }

    #[test]
    fn insert_replaces_and_delete_removes() {
        let index = ExactScanIndex::new(2);
        index.insert("a", &[1.0, 0.0]).unwrap();
        index.insert("a", &[0.0, 1.0]).unwrap();
        assert_eq!(index.len(), 1);
        let top = index.search_knn(&[0.0, 1.0], 1).unwrap();
        assert!((top[0].distance - 0.0).abs() < 1e-6);
        assert!(index.delete("a").unwrap());
        assert!(!index.delete("a").unwrap());
        assert!(index.is_empty());
    }
}
