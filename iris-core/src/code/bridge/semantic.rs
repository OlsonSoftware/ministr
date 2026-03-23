//! Semantic bridge fallback using embedding co-occurrence.
//!
//! When no explicit [`BridgeExtractor`] produces a match for an endpoint, this
//! module provides a fallback that compares symbol name embeddings between
//! unmatched exports and imports to suggest possible cross-language links.
//!
//! # Architecture
//!
//! The [`SemanticBridgeFallback`] takes a set of unmatched endpoints and an
//! embedding function, computes pairwise cosine similarity between export and
//! import symbol names, and returns candidate [`BridgeLink`]s at
//! [`ConfidenceLevel::Fuzzy`] confidence for pairs exceeding the similarity
//! threshold.
//!
//! This is a **post-processing step** — it runs after the exact and
//! case-normalized passes in [`BridgeLinker`](super::linker::BridgeLinker).

use super::{BridgeEndpoint, BridgeLink, ConfidenceLevel, EndpointRole};

/// Minimum cosine similarity for two symbol names to be considered a semantic match.
const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.85;

/// A function that produces a dense embedding vector from a text string.
///
/// Implementations can wrap any embedding model (`fastembed`, `OpenAI`, etc.).
/// The returned vectors must all have the same dimensionality.
pub trait EmbeddingFn: Send + Sync {
    /// Embed a batch of text strings, returning one vector per input.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedding model fails.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SemanticFallbackError>;
}

/// Errors from the semantic bridge fallback.
#[derive(Debug, thiserror::Error)]
pub enum SemanticFallbackError {
    /// The embedding function returned an unexpected number of vectors.
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected vector count.
        expected: usize,
        /// Actual vector count returned.
        actual: usize,
    },
    /// The embedding function produced empty vectors.
    #[error("embedding produced empty vectors")]
    EmptyVectors,
    /// Wrapped error from the embedding model.
    #[error("embedding error: {0}")]
    Embedding(String),
}

/// Semantic bridge fallback that uses embedding similarity to suggest links.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::semantic::SemanticBridgeFallback;
///
/// let fallback = SemanticBridgeFallback::new(0.85);
/// assert!((fallback.threshold() - 0.85).abs() < f32::EPSILON);
/// ```
pub struct SemanticBridgeFallback {
    threshold: f32,
}

impl SemanticBridgeFallback {
    /// Create a new semantic fallback with the given cosine similarity threshold.
    #[must_use]
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    /// Returns the current similarity threshold.
    #[must_use]
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Find semantic matches among unmatched endpoints.
    ///
    /// Takes exports and imports that were not matched by exact or case-normalized
    /// passes, embeds their symbol names, and returns candidate links for pairs
    /// with cosine similarity above the threshold.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedding function fails.
    pub fn find_matches(
        &self,
        unmatched_exports: &[&BridgeEndpoint],
        unmatched_imports: &[&BridgeEndpoint],
        embedder: &dyn EmbeddingFn,
    ) -> Result<Vec<BridgeLink>, SemanticFallbackError> {
        if unmatched_exports.is_empty() || unmatched_imports.is_empty() {
            return Ok(Vec::new());
        }

        // Collect all symbol names for batch embedding
        let export_names: Vec<&str> = unmatched_exports
            .iter()
            .map(|ep| ep.symbol_name.as_str())
            .collect();
        let import_names: Vec<&str> = unmatched_imports
            .iter()
            .map(|ep| ep.symbol_name.as_str())
            .collect();

        let all_names: Vec<&str> = export_names
            .iter()
            .chain(import_names.iter())
            .copied()
            .collect();

        let embeddings = embedder.embed_batch(&all_names)?;
        if embeddings.len() != all_names.len() {
            return Err(SemanticFallbackError::DimensionMismatch {
                expected: all_names.len(),
                actual: embeddings.len(),
            });
        }

        let (export_embeddings, import_embeddings) = embeddings.split_at(export_names.len());

        let mut links = Vec::new();

        for (i, export_emb) in export_embeddings.iter().enumerate() {
            for (j, import_emb) in import_embeddings.iter().enumerate() {
                let similarity = cosine_similarity(export_emb, import_emb);
                if similarity >= self.threshold {
                    let mut export = unmatched_exports[i].clone();
                    let mut import = unmatched_imports[j].clone();

                    // Normalize binding keys to the export's key for linking
                    import.binding_key.clone_from(&export.binding_key);

                    // Set confidence to Fuzzy level
                    let fuzzy = ConfidenceLevel::Fuzzy.score();
                    export.confidence = fuzzy;
                    import.confidence = fuzzy;

                    links.push(BridgeLink::new(export, import));
                }
            }
        }

        // Sort by descending confidence (all Fuzzy, but ties broken by order)
        links.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(links)
    }
}

impl Default for SemanticBridgeFallback {
    fn default() -> Self {
        Self::new(DEFAULT_SIMILARITY_THRESHOLD)
    }
}

/// Compute cosine similarity between two vectors.
///
/// Returns 0.0 for zero-length or empty vectors.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::semantic::cosine_similarity;
///
/// let a = vec![1.0, 0.0, 0.0];
/// let b = vec![1.0, 0.0, 0.0];
/// assert!((cosine_similarity(&a, &b) - 1.0).abs() < f32::EPSILON);
///
/// let c = vec![0.0, 1.0, 0.0];
/// assert!(cosine_similarity(&a, &c).abs() < f32::EPSILON);
/// ```
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Collect unmatched endpoints from a set of all endpoints and existing links.
///
/// Returns `(unmatched_exports, unmatched_imports)`.
#[must_use]
pub fn collect_unmatched<'a>(
    endpoints: &'a [BridgeEndpoint],
    links: &[BridgeLink],
) -> (Vec<&'a BridgeEndpoint>, Vec<&'a BridgeEndpoint>) {
    use std::collections::HashSet;

    // Build a set of matched (kind, binding_key, role, file_path, line) tuples
    let mut matched: HashSet<(&str, &str, u32)> = HashSet::new();
    for link in links {
        matched.insert((
            link.export.file_path.as_str(),
            link.export.binding_key.as_str(),
            link.export.line,
        ));
        matched.insert((
            link.import.file_path.as_str(),
            link.import.binding_key.as_str(),
            link.import.line,
        ));
    }

    let mut exports = Vec::new();
    let mut imports = Vec::new();

    for ep in endpoints {
        let key = (ep.file_path.as_str(), ep.binding_key.as_str(), ep.line);
        if !matched.contains(&key) {
            match ep.role {
                EndpointRole::Export => exports.push(ep),
                EndpointRole::Import => imports.push(ep),
            }
        }
    }

    (exports, imports)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::bridge::{BridgeKind, EndpointRole};

    /// A mock embedding function that returns normalized vectors based on string identity.
    struct MockEmbedder;

    impl EmbeddingFn for MockEmbedder {
        fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SemanticFallbackError> {
            // Simple: hash each character into a 64-dim vector
            Ok(texts
                .iter()
                .map(|t| {
                    let mut vec = vec![0.0_f32; 64];
                    for (i, c) in t.chars().enumerate() {
                        #[allow(clippy::cast_precision_loss)]
                        let val = c as u32 as f32;
                        vec[i % 64] += val;
                    }
                    // Normalize
                    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for x in &mut vec {
                            *x /= norm;
                        }
                    }
                    vec
                })
                .collect())
        }
    }

    /// Embedding function that always returns identical vectors (similarity = 1.0).
    struct IdenticalEmbedder;

    impl EmbeddingFn for IdenticalEmbedder {
        fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SemanticFallbackError> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    /// Embedding function that returns orthogonal vectors (similarity = 0.0).
    struct OrthogonalEmbedder {
        dim: usize,
    }

    impl EmbeddingFn for OrthogonalEmbedder {
        fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, SemanticFallbackError> {
            Ok(texts
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let mut vec = vec![0.0_f32; self.dim];
                    vec[i % self.dim] = 1.0;
                    vec
                })
                .collect())
        }
    }

    fn make_endpoint(
        key: &str,
        kind: BridgeKind,
        role: EndpointRole,
        language: &str,
    ) -> BridgeEndpoint {
        BridgeEndpoint {
            binding_key: key.into(),
            kind,
            role,
            language: language.into(),
            file_path: format!("src/test.{}", if language == "rust" { "rs" } else { "ts" }),
            line: 1,
            symbol_name: key.into(),
            confidence: 1.0,
        }
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_similarity_empty_vectors() {
        assert!(cosine_similarity(&[], &[]).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_similarity_different_lengths() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < f32::EPSILON);
    }

    #[test]
    fn default_threshold() {
        let fallback = SemanticBridgeFallback::default();
        assert!((fallback.threshold() - 0.85).abs() < f32::EPSILON);
    }

    #[test]
    fn find_matches_empty_inputs() {
        let fallback = SemanticBridgeFallback::default();
        let embedder = MockEmbedder;

        let result = fallback.find_matches(&[], &[], &embedder).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn find_matches_identical_embeddings_produce_links() {
        let fallback = SemanticBridgeFallback::new(0.5);
        let embedder = IdenticalEmbedder;

        let export = make_endpoint(
            "get_users",
            BridgeKind::HttpRoute,
            EndpointRole::Export,
            "rust",
        );
        let import = make_endpoint(
            "fetchUsers",
            BridgeKind::HttpRoute,
            EndpointRole::Import,
            "typescript",
        );

        let exports = vec![&export];
        let imports = vec![&import];

        let links = fallback
            .find_matches(&exports, &imports, &embedder)
            .unwrap();
        assert_eq!(links.len(), 1);
        assert!((links[0].confidence - ConfidenceLevel::Fuzzy.score()).abs() < f32::EPSILON);
    }

    #[test]
    fn find_matches_orthogonal_embeddings_produce_no_links() {
        let fallback = SemanticBridgeFallback::new(0.5);
        let embedder = OrthogonalEmbedder { dim: 10 };

        let export = make_endpoint("alpha", BridgeKind::HttpRoute, EndpointRole::Export, "rust");
        let import = make_endpoint(
            "beta",
            BridgeKind::HttpRoute,
            EndpointRole::Import,
            "typescript",
        );

        let exports = vec![&export];
        let imports = vec![&import];

        let links = fallback
            .find_matches(&exports, &imports, &embedder)
            .unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn find_matches_respects_threshold() {
        let fallback = SemanticBridgeFallback::new(0.99);
        let embedder = MockEmbedder;

        // "get_user" and "getUser" should be somewhat similar but below 0.99
        let export = make_endpoint(
            "get_user",
            BridgeKind::HttpRoute,
            EndpointRole::Export,
            "rust",
        );
        let import = make_endpoint(
            "fetch_data",
            BridgeKind::HttpRoute,
            EndpointRole::Import,
            "typescript",
        );

        let exports = vec![&export];
        let imports = vec![&import];

        let links = fallback
            .find_matches(&exports, &imports, &embedder)
            .unwrap();
        // At 0.99 threshold, dissimilar names should not match
        assert!(links.is_empty());
    }

    #[test]
    fn collect_unmatched_filters_linked_endpoints() {
        let ep1 = make_endpoint(
            "greet",
            BridgeKind::TauriCommand,
            EndpointRole::Export,
            "rust",
        );
        let ep2 = make_endpoint(
            "greet",
            BridgeKind::TauriCommand,
            EndpointRole::Import,
            "typescript",
        );
        let ep3 = make_endpoint(
            "save",
            BridgeKind::TauriCommand,
            EndpointRole::Export,
            "rust",
        );

        let link = BridgeLink::new(ep1.clone(), ep2.clone());
        let endpoints = vec![ep1, ep2, ep3];

        let (exports, imports) = collect_unmatched(&endpoints, &[link]);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].binding_key, "save");
        assert!(imports.is_empty());
    }

    #[test]
    fn collect_unmatched_no_links_returns_all() {
        let ep1 = make_endpoint(
            "greet",
            BridgeKind::TauriCommand,
            EndpointRole::Export,
            "rust",
        );
        let ep2 = make_endpoint(
            "save",
            BridgeKind::TauriCommand,
            EndpointRole::Import,
            "typescript",
        );

        let endpoints = vec![ep1, ep2];

        let (exports, imports) = collect_unmatched(&endpoints, &[]);
        assert_eq!(exports.len(), 1);
        assert_eq!(imports.len(), 1);
    }
}
