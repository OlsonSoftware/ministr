//! Cross-session analytics for iris-core.
//!
//! Tracks section access frequency and co-access patterns across sessions
//! to inform prefetch prioritization. Data is persisted in `SQLite` and
//! queried by the prefetch engine to pre-warm frequently-accessed and
//! co-accessed sections.
//!
//! # Architecture
//!
//! - [`Analytics`] — main service wrapping the storage layer, providing
//!   recording and query methods for cross-session access data.
//! - Access frequency: incremented on each `iris_read` call.
//! - Co-access patterns: recorded at session end from the session trajectory.

use crate::error::StorageError;
use crate::storage::{CoAccessRecord, CorpusStats, SectionAccessStat, SqliteStorage, Storage};
use crate::types::SectionId;

/// Default number of top sections to return.
const DEFAULT_TOP_LIMIT: usize = 10;

/// Default number of co-accessed sections to return.
const DEFAULT_CO_ACCESS_LIMIT: usize = 5;

/// Cross-session analytics service.
///
/// Wraps the storage layer to provide access frequency tracking and
/// co-access pattern detection. Used by the prefetch engine to prioritize
/// pre-warming of sections that are historically popular.
///
/// # Examples
///
/// ```no_run
/// # async fn example() {
/// use iris_core::analytics::Analytics;
/// use iris_core::storage::SqliteStorage;
/// use iris_core::types::SectionId;
///
/// let storage = SqliteStorage::open_in_memory().unwrap();
/// let analytics = Analytics::new(storage);
///
/// analytics.record_access(&SectionId("s1".into())).await.unwrap();
/// let top = analytics.top_sections(5).await.unwrap();
/// assert_eq!(top.len(), 1);
/// assert_eq!(top[0].access_count, 1);
/// # }
/// ```
pub struct Analytics {
    storage: SqliteStorage,
}

impl Analytics {
    /// Create a new analytics service backed by the given storage.
    #[must_use]
    pub fn new(storage: SqliteStorage) -> Self {
        Self { storage }
    }

    /// Record a section access, incrementing its cross-session frequency.
    ///
    /// Call this after each `iris_read` to build up access statistics.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database operation fails.
    pub async fn record_access(&self, section_id: &SectionId) -> Result<(), StorageError> {
        self.storage.record_section_access(section_id).await
    }

    /// Record co-access patterns from a session's trajectory.
    ///
    /// For each unique pair of sections in the trajectory, increments the
    /// co-access count. Call this at session end or periodically during
    /// long sessions.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database operation fails.
    pub async fn record_co_accesses(&self, section_ids: &[SectionId]) -> Result<(), StorageError> {
        if section_ids.len() < 2 {
            return Ok(());
        }
        // Deduplicate section IDs before recording pairs
        let mut unique: Vec<SectionId> = Vec::new();
        for id in section_ids {
            if !unique.iter().any(|u| u.0 == id.0) {
                unique.push(id.clone());
            }
        }
        if unique.len() < 2 {
            return Ok(());
        }
        self.storage.record_co_accesses(&unique).await
    }

    /// Get the most frequently accessed sections across all sessions.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database query fails.
    pub async fn top_sections(&self, limit: usize) -> Result<Vec<SectionAccessStat>, StorageError> {
        self.storage.get_top_sections(limit).await
    }

    /// Get sections most frequently co-accessed with the given section.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database query fails.
    pub async fn co_accessed_with(
        &self,
        section_id: &SectionId,
        limit: usize,
    ) -> Result<Vec<CoAccessRecord>, StorageError> {
        self.storage.get_co_accessed(section_id, limit).await
    }

    /// Get aggregate corpus statistics.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database query fails.
    pub async fn corpus_stats(&self) -> Result<CorpusStats, StorageError> {
        self.storage.get_corpus_stats().await
    }

    /// Get the default limit for top sections queries.
    #[must_use]
    pub const fn default_top_limit() -> usize {
        DEFAULT_TOP_LIMIT
    }

    /// Get the default limit for co-access queries.
    #[must_use]
    pub const fn default_co_access_limit() -> usize {
        DEFAULT_CO_ACCESS_LIMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(s: &str) -> SectionId {
        SectionId(s.to_string())
    }

    fn setup() -> Analytics {
        let storage = SqliteStorage::open_in_memory().unwrap();
        Analytics::new(storage)
    }

    #[tokio::test]
    async fn record_and_query_access() {
        let a = setup();
        a.record_access(&sid("s1")).await.unwrap();
        a.record_access(&sid("s1")).await.unwrap();
        a.record_access(&sid("s2")).await.unwrap();

        let top = a.top_sections(10).await.unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].section_id, sid("s1"));
        assert_eq!(top[0].access_count, 2);
        assert_eq!(top[1].section_id, sid("s2"));
        assert_eq!(top[1].access_count, 1);
    }

    #[tokio::test]
    async fn top_sections_respects_limit() {
        let a = setup();
        for i in 0..5 {
            a.record_access(&sid(&format!("s{i}"))).await.unwrap();
        }

        let top = a.top_sections(3).await.unwrap();
        assert_eq!(top.len(), 3);
    }

    #[tokio::test]
    async fn record_and_query_co_access() {
        let a = setup();
        let ids = vec![sid("s1"), sid("s2"), sid("s3")];
        a.record_co_accesses(&ids).await.unwrap();

        let co = a.co_accessed_with(&sid("s1"), 10).await.unwrap();
        assert_eq!(co.len(), 2);
        // Both s2 and s3 should be co-accessed with s1
        let partners: Vec<&str> = co.iter().map(|c| c.section_id.0.as_str()).collect();
        assert!(partners.contains(&"s2"));
        assert!(partners.contains(&"s3"));
    }

    #[tokio::test]
    async fn co_access_count_increments() {
        let a = setup();
        // Record co-access twice
        let ids = vec![sid("s1"), sid("s2")];
        a.record_co_accesses(&ids).await.unwrap();
        a.record_co_accesses(&ids).await.unwrap();

        let co = a.co_accessed_with(&sid("s1"), 10).await.unwrap();
        assert_eq!(co.len(), 1);
        assert_eq!(co[0].co_count, 2);
    }

    #[tokio::test]
    async fn co_access_deduplicates_trajectory() {
        let a = setup();
        // Trajectory with duplicates: s1 accessed twice, s2 once
        let ids = vec![sid("s1"), sid("s2"), sid("s1")];
        a.record_co_accesses(&ids).await.unwrap();

        let co = a.co_accessed_with(&sid("s1"), 10).await.unwrap();
        assert_eq!(co.len(), 1);
        assert_eq!(co[0].section_id, sid("s2"));
        assert_eq!(co[0].co_count, 1);
    }

    #[tokio::test]
    async fn co_access_with_single_section_is_noop() {
        let a = setup();
        a.record_co_accesses(&[sid("s1")]).await.unwrap();
        let co = a.co_accessed_with(&sid("s1"), 10).await.unwrap();
        assert!(co.is_empty());
    }

    #[tokio::test]
    async fn co_access_empty_is_noop() {
        let a = setup();
        a.record_co_accesses(&[]).await.unwrap();
    }

    #[tokio::test]
    async fn corpus_stats_empty() {
        let a = setup();
        let stats = a.corpus_stats().await.unwrap();
        assert_eq!(stats.total_accesses, 0);
        assert_eq!(stats.unique_sections_accessed, 0);
        assert_eq!(stats.co_access_pairs, 0);
    }

    #[tokio::test]
    async fn corpus_stats_with_data() {
        let a = setup();
        a.record_access(&sid("s1")).await.unwrap();
        a.record_access(&sid("s1")).await.unwrap();
        a.record_access(&sid("s2")).await.unwrap();
        a.record_co_accesses(&[sid("s1"), sid("s2")]).await.unwrap();

        let stats = a.corpus_stats().await.unwrap();
        assert_eq!(stats.total_accesses, 3);
        assert_eq!(stats.unique_sections_accessed, 2);
        assert_eq!(stats.co_access_pairs, 1);
    }

    #[tokio::test]
    async fn co_access_respects_limit() {
        let a = setup();
        let ids = vec![sid("s1"), sid("s2"), sid("s3"), sid("s4"), sid("s5")];
        a.record_co_accesses(&ids).await.unwrap();

        let co = a.co_accessed_with(&sid("s1"), 2).await.unwrap();
        assert_eq!(co.len(), 2);
    }

    #[tokio::test]
    async fn co_access_ordering_by_count() {
        let a = setup();
        // s1+s2 co-accessed 3 times, s1+s3 once
        for _ in 0..3 {
            a.record_co_accesses(&[sid("s1"), sid("s2")]).await.unwrap();
        }
        a.record_co_accesses(&[sid("s1"), sid("s3")]).await.unwrap();

        let co = a.co_accessed_with(&sid("s1"), 10).await.unwrap();
        assert_eq!(co[0].section_id, sid("s2"));
        assert_eq!(co[0].co_count, 3);
        assert_eq!(co[1].section_id, sid("s3"));
        assert_eq!(co[1].co_count, 1);
    }

    #[test]
    fn default_limits() {
        assert_eq!(Analytics::default_top_limit(), 10);
        assert_eq!(Analytics::default_co_access_limit(), 5);
    }
}
