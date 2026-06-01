//! Index health diagnostics — detect a *degenerate* vector set before it
//! silently breaks semantic search.
//!
//! Two catastrophic, query-breaking conditions are detected (the "complete
//! collapse" + zero-vector cases from the embedding-collapse literature):
//!
//! - **all-degenerate** — every vector is zero / non-finite. Under cosine a
//!   zero vector sits at distance 0 from *every* query, so an index of only
//!   zeros returns the same junk for any query (the exact failure mode that
//!   broke `ministr_survey`, see `f-ministr-corpus-survey-degenerate`).
//! - **collapsed** — two or more live vectors all point in the same direction
//!   (cosine ≈ 1 pairwise), so every query gets an effectively equal-distance
//!   ranking and real relevance is lost.
//!
//! This mirrors the per-vector guard in [`crate::index::HnswIndex::insert`]
//! (which refuses a single zero/non-finite vector) but operates over a *whole
//! set* — the durable source of truth re-loaded by
//! [`crate::index::rebuild_hnsw_from_store`] — so a degenerate corpus is caught
//! and surfaced rather than silently serving broken search.
//!
//! Dimensional/partial collapse (vectors spanning a low-rank subspace without
//! fully collapsing) is a model-quality concern, not a hard index poison, and
//! is deliberately out of scope here.

/// Squared-norm floor below which a vector is treated as zero/degenerate.
/// Matches the guard in `HnswIndex::insert`.
const MIN_NORM_SQ: f32 = 1e-12;

/// Cosine-similarity tolerance for "same direction". Two unit vectors whose
/// dot product exceeds `1.0 - COLLAPSE_EPS` are treated as parallel.
const COLLAPSE_EPS: f32 = 1e-6;

/// Health summary for a set of indexed vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorSetHealth {
    /// Total vectors inspected.
    pub total: usize,
    /// Count of zero / non-finite (degenerate) vectors.
    pub degenerate: usize,
    /// True when ≥2 live vectors all share one direction (complete collapse).
    pub collapsed: bool,
}

impl VectorSetHealth {
    /// Number of live (finite, non-zero) vectors.
    #[must_use]
    pub fn live(&self) -> usize {
        self.total - self.degenerate
    }

    /// Whether the set is degenerate — it would make semantic search return
    /// effectively equal-distance results for any query. True when the set is
    /// non-empty and either every vector is degenerate or the live vectors have
    /// collapsed to a single direction.
    #[must_use]
    pub fn is_degenerate(&self) -> bool {
        self.total > 0 && (self.degenerate == self.total || self.collapsed)
    }
}

/// Analyze a set of vectors for degeneracy in a single pass.
///
/// Accepts any iterator of vector slices so callers can stream borrowed data
/// (e.g. the `(id, vector)` pairs loaded by `rebuild_hnsw_from_store`) without
/// cloning. An empty set is reported as healthy (`total = 0`).
#[must_use]
pub fn analyze_vectors<'a, I>(vectors: I) -> VectorSetHealth
where
    I: IntoIterator<Item = &'a [f32]>,
{
    let mut total = 0usize;
    let mut degenerate = 0usize;
    let mut live = 0usize;
    let mut all_parallel = true;
    let mut reference_dir: Option<Vec<f32>> = None;

    for vector in vectors {
        total += 1;
        let norm_sq: f32 = vector.iter().map(|x| x * x).sum();
        if !norm_sq.is_finite() || norm_sq < MIN_NORM_SQ {
            degenerate += 1;
            continue;
        }
        live += 1;

        // Only keep comparing while every live vector has matched so far —
        // once one diverges, the set isn't collapsed and we can stop the dot
        // products (still draining the iterator to finish the degenerate count).
        if !all_parallel {
            continue;
        }
        let inv_norm = norm_sq.sqrt().recip();
        let dir: Vec<f32> = vector.iter().map(|x| x * inv_norm).collect();
        match &reference_dir {
            None => reference_dir = Some(dir),
            Some(reference) => {
                let cosine: f32 = reference.iter().zip(&dir).map(|(a, b)| a * b).sum();
                if cosine < 1.0 - COLLAPSE_EPS {
                    all_parallel = false;
                }
            }
        }
    }

    // Collapse requires ≥2 live vectors that are all parallel. A single live
    // vector (or none) is not a collapse.
    let collapsed = live >= 2 && all_parallel;

    VectorSetHealth {
        total,
        degenerate,
        collapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: analyze owned vectors without threading lifetimes at call sites.
    fn analyze(vectors: &[Vec<f32>]) -> VectorSetHealth {
        analyze_vectors(vectors.iter().map(Vec::as_slice))
    }

    #[test]
    fn empty_set_is_healthy() {
        let h = analyze(&[]);
        assert_eq!(h.total, 0);
        assert!(!h.is_degenerate(), "an empty index is not degenerate");
    }

    #[test]
    fn varied_set_is_healthy() {
        let h = analyze(&[
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ]);
        assert_eq!(h.degenerate, 0);
        assert!(!h.collapsed);
        assert!(!h.is_degenerate());
        assert_eq!(h.live(), 3);
    }

    #[test]
    fn single_live_vector_is_not_collapsed() {
        let h = analyze(&[vec![0.3, 0.4, 0.0]]);
        assert!(!h.collapsed, "one vector cannot be a collapse");
        assert!(!h.is_degenerate());
    }

    #[test]
    fn all_zero_set_is_degenerate() {
        let h = analyze(&[vec![0.0, 0.0, 0.0], vec![0.0, 0.0, 0.0]]);
        assert_eq!(h.degenerate, 2);
        assert_eq!(h.live(), 0);
        assert!(h.is_degenerate(), "an all-zero index poisons every query");
    }

    #[test]
    fn non_finite_counts_as_degenerate() {
        let h = analyze(&[vec![f32::NAN, 0.0], vec![f32::INFINITY, 1.0]]);
        assert_eq!(h.degenerate, 2);
        assert!(h.is_degenerate());
    }

    #[test]
    fn parallel_vectors_are_collapsed() {
        // Same direction at different magnitudes => cosine 1 pairwise.
        let h = analyze(&[
            vec![1.0, 1.0, 0.0],
            vec![2.0, 2.0, 0.0],
            vec![0.5, 0.5, 0.0],
        ]);
        assert_eq!(h.degenerate, 0);
        assert!(h.collapsed, "all live vectors share one direction");
        assert!(h.is_degenerate());
    }

    #[test]
    fn one_divergent_vector_breaks_collapse() {
        let h = analyze(&[
            vec![1.0, 1.0, 0.0],
            vec![1.0, 1.0, 0.0],
            vec![1.0, -1.0, 0.0],
        ]);
        assert!(!h.collapsed);
        assert!(!h.is_degenerate());
    }

    #[test]
    fn mixed_some_zero_some_varied_is_healthy() {
        let h = analyze(&[
            vec![0.0, 0.0, 0.0],
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
        ]);
        assert_eq!(h.degenerate, 1);
        assert_eq!(h.live(), 2);
        assert!(!h.collapsed);
        assert!(
            !h.is_degenerate(),
            "a few zeros among varied live vectors is not a degenerate index"
        );
    }
}
