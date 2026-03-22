//! Vector index subsystem for approximate nearest-neighbor search.
//!
//! The [`VectorIndex`] trait defines the interface for dense vector storage and
//! retrieval. The [`HnswIndex`] implementation uses the `hnswlib-rs` crate
//! for HNSW-based ANN search with memory-mapped persistence.
//!
//! The [`SparseIndex`] trait and [`InvertedIndex`] implementation provide
//! sparse vector storage for keyword-level matching via SPLADE embeddings.

mod hnsw;
mod inverted;

pub use hnsw::HnswIndex;
pub use inverted::InvertedIndex;

use std::path::Path;

use crate::error::IndexError;

/// A single result from a k-nearest-neighbor search.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// The string ID of the matched vector.
    pub id: String,
    /// Distance from the query vector (lower is more similar for cosine).
    pub distance: f32,
}

/// Interface for vector index implementations.
///
/// Provides insert, search, delete, and persistence operations over
/// string-keyed float vectors. Implementations must be `Send + Sync`
/// for use across async tasks.
///
/// # Examples
///
/// ```no_run
/// use iris_core::index::{HnswIndex, VectorIndex};
///
/// let index = HnswIndex::new(384, 10_000)?;
/// index.insert("section-1", &vec![0.1; 384])?;
///
/// let results = index.search_knn(&vec![0.1; 384], 5)?;
/// assert_eq!(results[0].id, "section-1");
/// # Ok::<(), iris_core::error::IndexError>(())
/// ```
pub trait VectorIndex: Send + Sync {
    /// Insert a vector with the given string ID.
    ///
    /// If a vector with the same ID already exists, it is replaced.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the vector dimension
    /// does not match the index dimension.
    fn insert(&self, id: &str, vector: &[f32]) -> Result<(), IndexError>;

    /// Search for the `k` nearest neighbors to the query vector.
    ///
    /// Results are sorted by distance (ascending — closest first).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::QueryFailed`] if the search fails.
    fn search_knn(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>, IndexError>;

    /// Delete a vector by its string ID.
    ///
    /// Returns `true` if the vector was found and deleted, `false` if
    /// it did not exist.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::QueryFailed`] if the delete operation fails.
    fn delete(&self, id: &str) -> Result<bool, IndexError>;

    /// Persist the index to the given directory.
    ///
    /// Creates the directory if it does not exist. Writes the HNSW graph
    /// and vector data as separate files.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::LoadFailed`] if persistence fails.
    fn persist(&self, dir: &Path) -> Result<(), IndexError>;

    /// The number of live (non-deleted) vectors in the index.
    fn len(&self) -> usize;

    /// Whether the index contains no vectors.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The dimensionality of vectors in this index.
    fn dimension(&self) -> usize;
}

/// Load a vector index from a persisted directory.
///
/// This is separate from the trait because `load` requires `Self: Sized`
/// and cannot be called on trait objects.
///
/// # Errors
///
/// Returns [`IndexError::LoadFailed`] if loading fails.
pub trait VectorIndexLoad: VectorIndex + Sized {
    /// Load a previously persisted index from the given directory.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::LoadFailed`] if the index files are missing or corrupted.
    fn load(dir: &Path) -> Result<Self, IndexError>;
}

/// A single result from a sparse index search.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseSearchResult {
    /// The string ID of the matched document.
    pub id: String,
    /// Dot-product score (higher is more relevant).
    pub score: f32,
}

/// Interface for sparse vector index implementations.
///
/// Stores sparse vectors (index/value pairs) keyed by string IDs,
/// and supports dot-product search for retrieval.
pub trait SparseIndex: Send + Sync {
    /// Insert a sparse vector with the given string ID.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if insertion fails.
    fn insert_sparse(&self, id: &str, indices: &[u32], values: &[f32]) -> Result<(), IndexError>;

    /// Search for documents most similar to the query sparse vector.
    ///
    /// Results are sorted by dot-product score descending (highest first).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::QueryFailed`] if search fails.
    fn search_sparse(
        &self,
        query_indices: &[u32],
        query_values: &[f32],
        k: usize,
    ) -> Result<Vec<SparseSearchResult>, IndexError>;

    /// Delete a document by its string ID.
    ///
    /// Returns `true` if the document was found and deleted.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::QueryFailed`] if the delete operation fails.
    fn delete_sparse(&self, id: &str) -> Result<bool, IndexError>;

    /// Persist the sparse index to the given directory.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::LoadFailed`] if persistence fails.
    fn persist_sparse(&self, dir: &Path) -> Result<(), IndexError>;

    /// Load a sparse index from a persisted directory.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::LoadFailed`] if loading fails.
    fn load_sparse(dir: &Path) -> Result<Self, IndexError>
    where
        Self: Sized;

    /// The number of documents in the sparse index.
    fn len_sparse(&self) -> usize;

    /// Whether the sparse index is empty.
    fn is_empty_sparse(&self) -> bool {
        self.len_sparse() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_result_construction() {
        let result = SearchResult {
            id: "section-1".to_string(),
            distance: 0.25,
        };
        assert_eq!(result.id, "section-1");
        assert!((result.distance - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn search_result_clone() {
        let result = SearchResult {
            id: "claim-42".to_string(),
            distance: 0.1,
        };
        let cloned = result.clone();
        assert_eq!(result, cloned);
    }
}
