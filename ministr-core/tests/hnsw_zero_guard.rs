//! Regression: a zero / non-finite embedding vector must never enter the HNSW
//! index. Under cosine similarity a zero vector is at distance 0 from every
//! query, so a single one dominates the top-k of every search and buries all
//! real results — the root cause of the "survey returns the same irrelevant
//! docs regardless of query" bug (f-ministr-corpus-survey-degenerate).

use ministr_core::index::{HnswIndex, VectorIndex};

#[test]
fn zero_vector_is_not_indexed_and_does_not_poison_search() {
    let index = HnswIndex::new(4, 100).unwrap();

    // Three real, distinct unit-ish vectors.
    index.insert("a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    index.insert("b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    index.insert("c", &[0.0, 0.0, 1.0, 0.0]).unwrap();

    // A zero vector (the poison) and a non-finite one — both must be rejected.
    index.insert("zero", &[0.0, 0.0, 0.0, 0.0]).unwrap();
    index.insert("nan", &[f32::NAN, 0.0, 0.0, 0.0]).unwrap();

    // Only the 3 real vectors are indexed.
    assert_eq!(index.len(), 3, "degenerate vectors must be skipped");

    // A query close to `a` returns `a` first, and NO result is the poison.
    let hits = index.search_knn(&[0.9, 0.1, 0.0, 0.0], 4).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "a", "nearest real vector should win");
    assert!(
        hits.iter().all(|h| h.id != "zero" && h.id != "nan"),
        "degenerate vectors must never appear in results"
    );

    // Distances are real/varied, not the degenerate all-zero pattern.
    let all_zero = hits.iter().all(|h| h.distance.abs() < 1e-6);
    assert!(!all_zero, "search must not collapse to all-zero distances");
}
