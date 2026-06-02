//! In-memory vector store with brute-force similarity search.
//!
//! A deliberately self-contained reference implementation used by the ministr
//! retrieval-quality evaluation corpus. It exercises the long-content regime
//! (functions whose embedded section text exceeds 128 and 256 tokens) so the
//! truncation (RQ1), AST-chunking (RQ3), and late-chunking (RQ6) levers become
//! measurable on the golden set.

/// Unnormalized dot product of two equal-length vectors.
///
/// Panics in debug builds if the lengths differ; callers are expected to pass
/// vectors drawn from the same embedding model.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "dot product needs equal-length vectors");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Cosine similarity between two embedding vectors.
///
/// Cosine similarity is the dot product of the two vectors divided by the
/// product of their L2 norms, which makes it invariant to vector magnitude and
/// therefore a good fit for comparing dense text embeddings where only the
/// direction carries semantic meaning. When either vector is the zero vector
/// the similarity is undefined; we return `0.0` in that case rather than
/// producing a `NaN`, because downstream nearest-neighbour ranking treats a
/// zero score as "no signal" and a `NaN` would poison every comparison it
/// participates in (this is the same degenerate-vector hazard that motivated
/// the ingestion-time zero-vector guard in the production pipeline).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product = dot(a, b);
    let norm_a = dot(a, a).sqrt();
    let norm_b = dot(b, b).sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot_product / (norm_a * norm_b)
}

/// Find the `k` nearest neighbours of `query` within `corpus` by cosine
/// similarity, returning their indices paired with their scores in descending
/// order of similarity (the most relevant match first).
///
/// This is an exhaustive, brute-force scan: every corpus vector is compared
/// against the query, which is `O(n * d)` for `n` vectors of dimension `d`.
/// That is intentionally simple — for the small evaluation corpus an exact scan
/// is both fast enough and free of the recall loss that an approximate index
/// (HNSW, IVF) would introduce, so it serves as the ground-truth ranking that
/// approximate indexes are measured against.
///
/// Ties are broken by the lower corpus index so the ordering is deterministic
/// across runs, which matters when the result feeds a regression gate that
/// asserts on exact ranks. Requesting more neighbours than the corpus contains
/// simply returns the whole corpus, sorted.
fn knn_search(query: &[f32], corpus: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    // Score every candidate exactly once, keeping its original index so the
    // caller can map a rank back to the corpus entry it came from.
    let mut scored: Vec<(usize, f32)> = corpus
        .iter()
        .enumerate()
        .map(|(index, candidate)| (index, cosine_similarity(query, candidate)))
        .collect();

    // Sort by descending score; on a tie fall back to ascending index so the
    // ordering is stable and reproducible run to run.
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    scored.truncate(k);
    scored
}
