//! Vector index subsystem for approximate nearest-neighbor search.
//!
//! The [`VectorIndex`] trait defines the interface for vector storage and
//! retrieval. The [`HnswIndex`] implementation uses the `hnswlib-rs` crate
//! for HNSW-based ANN search with memory-mapped persistence.

mod hnsw;

pub use hnsw::HnswIndex;

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
