//! Prefetch engine and LRU cache for speculative context pre-warming.
//!
//! The prefetch engine predicts what content an agent will need next based on
//! access patterns and pre-computes it into a fast LRU cache. When the agent
//! requests pre-warmed content, it is served in <1ms instead of requiring a
//! full cold retrieval (50–200ms).
//!
//! # Strategies
//!
//! - **Sequential**: When the agent reads section N, pre-warm section N+1 and
//!   the parent document summary.
//!
//! # Architecture
//!
//! - [`PrefetchCache`] — LRU cache with pre-computed text, token count, and
//!   heading path. Default capacity 50 items.
//! - [`PrefetchEngine`] — orchestrates prefetch strategies, triggers pre-warming
//!   after tool calls, and serves warm cache hits.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use crate::token::count_tokens;
use crate::types::Resolution;

/// Default number of items the prefetch cache can hold.
const DEFAULT_CACHE_CAPACITY: usize = 50;

/// A pre-computed cache entry ready for immediate delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// The content identifier (section ID or document ID).
    pub content_id: String,
    /// Full text content, ready to serve.
    pub text: String,
    /// Pre-computed token count of the text.
    pub token_count: usize,
    /// Heading hierarchy path, if applicable.
    pub heading_path: Option<Vec<String>>,
    /// Section summary, if available.
    pub summary: Option<String>,
    /// Resolution level of the cached content.
    pub resolution: Resolution,
    /// Number of claims available in the section (for warm read responses).
    pub claims_available: usize,
}

/// Hit/miss metrics for the prefetch cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct PrefetchMetrics {
    /// Number of cache hits (warm responses).
    pub hits: u64,
    /// Number of cache misses (cold retrievals).
    pub misses: u64,
}

impl PrefetchMetrics {
    /// Cache hit rate as a fraction (0.0–1.0).
    ///
    /// Returns 0.0 if no lookups have been performed.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f64 / total as f64
    }
}

/// LRU cache for pre-computed prefetch entries.
///
/// Uses a `HashMap` for O(1) lookups and a `VecDeque` for LRU ordering.
/// When capacity is reached, the least recently used entry is evicted.
///
/// # Examples
///
/// ```
/// use iris_core::session::prefetch::{PrefetchCache, CacheEntry};
/// use iris_core::types::Resolution;
///
/// let mut cache = PrefetchCache::new(2);
///
/// cache.insert("s1".to_string(), CacheEntry {
///     content_id: "s1".to_string(),
///     text: "Section one".to_string(),
///     token_count: 2,
///     heading_path: None,
///     summary: None,
///     resolution: Resolution::Section,
///     claims_available: 0,
/// });
///
/// assert!(cache.get("s1").is_some());
/// assert!(cache.get("s2").is_none());
/// ```
pub struct PrefetchCache {
    /// Map from content ID to cache entry.
    entries: HashMap<String, CacheEntry>,
    /// LRU ordering: front = least recently used, back = most recently used.
    order: VecDeque<String>,
    /// Maximum number of entries.
    capacity: usize,
    /// Hit/miss metrics.
    metrics: PrefetchMetrics,
}

impl PrefetchCache {
    /// Create a new prefetch cache with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
            metrics: PrefetchMetrics::default(),
        }
    }

    /// Create a new prefetch cache with the default capacity (50 items).
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY)
    }

    /// Look up an entry, moving it to the most-recently-used position.
    ///
    /// Records a hit or miss in the metrics.
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
        if self.entries.contains_key(key) {
            self.metrics.hits += 1;
            self.touch(key);
            self.entries.get(key)
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    /// Look up an entry without updating LRU order or metrics.
    #[must_use]
    pub fn peek(&self, key: &str) -> Option<&CacheEntry> {
        self.entries.get(key)
    }

    /// Insert an entry into the cache.
    ///
    /// If the cache is at capacity, the least recently used entry is evicted.
    /// If the key already exists, the entry is updated and moved to MRU.
    pub fn insert(&mut self, key: String, entry: CacheEntry) {
        // Zero-capacity cache accepts nothing
        if self.capacity == 0 {
            return;
        }

        if self.entries.contains_key(&key) {
            // Update existing entry
            self.entries.insert(key.clone(), entry);
            self.touch(&key);
            return;
        }

        // Evict LRU if at capacity
        if self.entries.len() >= self.capacity {
            if let Some(evicted_key) = self.order.pop_front() {
                self.entries.remove(&evicted_key);
            }
        }

        self.entries.insert(key.clone(), entry);
        self.order.push_back(key);
    }

    /// Remove an entry from the cache.
    ///
    /// Returns the removed entry if it existed.
    pub fn remove(&mut self, key: &str) -> Option<CacheEntry> {
        if let Some(entry) = self.entries.remove(key) {
            self.order.retain(|k| k != key);
            Some(entry)
        } else {
            None
        }
    }

    /// Number of entries currently in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cache capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current hit/miss metrics.
    #[must_use]
    pub fn metrics(&self) -> PrefetchMetrics {
        self.metrics
    }

    /// Reset hit/miss counters.
    pub fn reset_metrics(&mut self) {
        self.metrics = PrefetchMetrics::default();
    }

    /// Move a key to the most-recently-used position.
    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
    }
}

/// Prefetch engine that orchestrates speculative pre-warming strategies.
///
/// After each `iris_read` call, the engine predicts what the agent will
/// need next and inserts pre-computed entries into the [`PrefetchCache`].
/// Before cold retrieval, the engine checks the cache for a warm hit.
///
/// # Examples
///
/// ```
/// use iris_core::session::prefetch::PrefetchEngine;
///
/// let engine = PrefetchEngine::new(50);
/// assert!(engine.cache().is_empty());
/// ```
pub struct PrefetchEngine {
    /// The LRU prefetch cache.
    cache: PrefetchCache,
}

impl PrefetchEngine {
    /// Create a new prefetch engine with the given cache capacity.
    #[must_use]
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            cache: PrefetchCache::new(cache_capacity),
        }
    }

    /// Create a new prefetch engine with the default cache capacity.
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self {
            cache: PrefetchCache::with_default_capacity(),
        }
    }

    /// Try to serve a section from the warm cache.
    ///
    /// Returns `Some(entry)` if the content is pre-warmed, `None` for a
    /// cache miss (requiring cold retrieval).
    pub fn try_serve(&mut self, section_id: &str) -> Option<CacheEntry> {
        self.cache.get(section_id).cloned()
    }

    /// Record a miss (cold retrieval) in the metrics.
    pub fn record_miss(&mut self) {
        self.cache.metrics.misses += 1;
    }

    /// Pre-warm the cache after a read operation using sequential prefetch.
    ///
    /// Inserts the next section and parent document summary into the cache
    /// so subsequent reads can be served warm.
    pub fn prefetch_sequential(
        &mut self,
        next_section: Option<crate::storage::SectionRecord>,
        doc_summary: Option<(String, String)>,
        claims_count: Option<usize>,
    ) {
        // Pre-warm next section (sequential locality)
        if let Some(section) = next_section {
            let token_count = count_tokens(&section.text);
            self.cache.insert(
                section.id.0.clone(),
                CacheEntry {
                    content_id: section.id.0,
                    text: section.text,
                    token_count,
                    heading_path: Some(section.heading_path),
                    summary: section.summary,
                    resolution: Resolution::Section,
                    claims_available: claims_count.unwrap_or(0),
                },
            );
        }

        // Pre-warm parent document summary
        if let Some((doc_id, summary)) = doc_summary {
            let token_count = count_tokens(&summary);
            let cache_key = format!("doc-summary::{doc_id}");
            self.cache.insert(
                cache_key,
                CacheEntry {
                    content_id: doc_id,
                    text: summary.clone(),
                    token_count,
                    heading_path: None,
                    summary: Some(summary),
                    resolution: Resolution::Summary,
                    claims_available: 0,
                },
            );
        }
    }

    /// Read-only access to the underlying cache.
    #[must_use]
    pub fn cache(&self) -> &PrefetchCache {
        &self.cache
    }

    /// Mutable access to the underlying cache.
    pub fn cache_mut(&mut self) -> &mut PrefetchCache {
        &mut self.cache
    }

    /// Current prefetch metrics.
    #[must_use]
    pub fn metrics(&self) -> PrefetchMetrics {
        self.cache.metrics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, text: &str) -> CacheEntry {
        CacheEntry {
            content_id: id.to_string(),
            text: text.to_string(),
            token_count: count_tokens(text),
            heading_path: None,
            summary: None,
            resolution: Resolution::Section,
            claims_available: 0,
        }
    }

    fn make_section_record(
        id: &str,
        doc_id: &str,
        text: &str,
        position: i64,
    ) -> crate::storage::SectionRecord {
        crate::storage::SectionRecord {
            id: crate::types::SectionId(id.to_string()),
            document_id: crate::types::ContentId(doc_id.to_string()),
            heading_path: vec![format!("Heading for {id}")],
            depth: 1,
            text: text.to_string(),
            summary: Some(format!("Summary of {id}")),
            position,
        }
    }

    // --- PrefetchCache tests ---

    #[test]
    fn new_cache_is_empty() {
        let cache = PrefetchCache::new(10);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.capacity(), 10);
    }

    #[test]
    fn insert_and_get() {
        let mut cache = PrefetchCache::new(10);
        cache.insert("s1".to_string(), make_entry("s1", "hello world"));
        assert_eq!(cache.len(), 1);

        let entry = cache.get("s1").unwrap();
        assert_eq!(entry.content_id, "s1");
        assert_eq!(entry.text, "hello world");
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let mut cache = PrefetchCache::new(10);
        assert!(cache.get("missing").is_none());
    }

    #[test]
    fn lru_eviction_at_capacity() {
        let mut cache = PrefetchCache::new(2);
        cache.insert("s1".to_string(), make_entry("s1", "one"));
        cache.insert("s2".to_string(), make_entry("s2", "two"));
        assert_eq!(cache.len(), 2);

        // Insert a third — should evict s1 (LRU)
        cache.insert("s3".to_string(), make_entry("s3", "three"));
        assert_eq!(cache.len(), 2);
        assert!(cache.peek("s1").is_none());
        assert!(cache.peek("s2").is_some());
        assert!(cache.peek("s3").is_some());
    }

    #[test]
    fn get_moves_to_mru() {
        let mut cache = PrefetchCache::new(2);
        cache.insert("s1".to_string(), make_entry("s1", "one"));
        cache.insert("s2".to_string(), make_entry("s2", "two"));

        // Access s1 to make it MRU
        let _ = cache.get("s1");

        // Insert s3 — should evict s2 (now LRU), not s1
        cache.insert("s3".to_string(), make_entry("s3", "three"));
        assert!(cache.peek("s1").is_some());
        assert!(cache.peek("s2").is_none());
        assert!(cache.peek("s3").is_some());
    }

    #[test]
    fn insert_updates_existing_entry() {
        let mut cache = PrefetchCache::new(10);
        cache.insert("s1".to_string(), make_entry("s1", "old text"));
        cache.insert("s1".to_string(), make_entry("s1", "new text"));

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.peek("s1").unwrap().text, "new text");
    }

    #[test]
    fn remove_entry() {
        let mut cache = PrefetchCache::new(10);
        cache.insert("s1".to_string(), make_entry("s1", "hello"));

        let removed = cache.remove("s1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().content_id, "s1");
        assert!(cache.is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut cache = PrefetchCache::new(10);
        assert!(cache.remove("missing").is_none());
    }

    #[test]
    fn metrics_track_hits_and_misses() {
        let mut cache = PrefetchCache::new(10);
        cache.insert("s1".to_string(), make_entry("s1", "hello"));

        let _ = cache.get("s1"); // hit
        let _ = cache.get("s2"); // miss
        let _ = cache.get("s1"); // hit

        let m = cache.metrics();
        assert_eq!(m.hits, 2);
        assert_eq!(m.misses, 1);
    }

    #[test]
    fn hit_rate_calculation() {
        let mut metrics = PrefetchMetrics::default();
        assert!((metrics.hit_rate() - 0.0).abs() < f64::EPSILON);

        metrics.hits = 3;
        metrics.misses = 1;
        assert!((metrics.hit_rate() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn reset_metrics() {
        let mut cache = PrefetchCache::new(10);
        cache.insert("s1".to_string(), make_entry("s1", "hello"));
        let _ = cache.get("s1");
        assert_eq!(cache.metrics().hits, 1);

        cache.reset_metrics();
        assert_eq!(cache.metrics().hits, 0);
        assert_eq!(cache.metrics().misses, 0);
    }

    #[test]
    fn peek_does_not_update_metrics_or_order() {
        let mut cache = PrefetchCache::new(2);
        cache.insert("s1".to_string(), make_entry("s1", "one"));
        cache.insert("s2".to_string(), make_entry("s2", "two"));

        // Peek at s1 — should NOT make it MRU
        let _ = cache.peek("s1");
        assert_eq!(cache.metrics().hits, 0);

        // Insert s3 — should evict s1 (still LRU since peek doesn't touch)
        cache.insert("s3".to_string(), make_entry("s3", "three"));
        assert!(cache.peek("s1").is_none());
    }

    #[test]
    fn zero_capacity_cache() {
        let mut cache = PrefetchCache::new(0);
        cache.insert("s1".to_string(), make_entry("s1", "hello"));
        // Zero capacity means nothing is stored
        assert!(cache.is_empty());
    }

    #[test]
    fn many_inserts_maintain_capacity() {
        let mut cache = PrefetchCache::new(3);
        for i in 0..100 {
            let key = format!("s{i}");
            cache.insert(key.clone(), make_entry(&key, &format!("text {i}")));
        }
        assert_eq!(cache.len(), 3);
        // Only the last 3 should remain
        assert!(cache.peek("s97").is_some());
        assert!(cache.peek("s98").is_some());
        assert!(cache.peek("s99").is_some());
    }

    // --- PrefetchEngine tests ---

    #[test]
    fn engine_new_has_empty_cache() {
        let engine = PrefetchEngine::new(10);
        assert!(engine.cache().is_empty());
    }

    #[test]
    fn try_serve_returns_none_for_cold() {
        let mut engine = PrefetchEngine::new(10);
        assert!(engine.try_serve("s1").is_none());
    }

    #[test]
    fn prefetch_sequential_warms_next_section() {
        let mut engine = PrefetchEngine::new(10);
        let next = make_section_record("doc#s2", "doc", "Section two text", 1);

        engine.prefetch_sequential(Some(next), None, None);

        let entry = engine.try_serve("doc#s2");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.content_id, "doc#s2");
        assert_eq!(entry.text, "Section two text");
        assert_eq!(entry.resolution, Resolution::Section);
    }

    #[test]
    fn prefetch_sequential_warms_doc_summary() {
        let mut engine = PrefetchEngine::new(10);

        engine.prefetch_sequential(
            None,
            Some((
                "doc-api".to_string(),
                "API documentation overview".to_string(),
            )),
            None,
        );

        let entry = engine.try_serve("doc-summary::doc-api");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.resolution, Resolution::Summary);
        assert_eq!(entry.text, "API documentation overview");
    }

    #[test]
    fn prefetch_sequential_warms_both() {
        let mut engine = PrefetchEngine::new(10);
        let next = make_section_record("doc#s2", "doc", "Next section", 1);

        engine.prefetch_sequential(
            Some(next),
            Some(("doc".to_string(), "Doc summary".to_string())),
            None,
        );

        assert!(engine.cache().peek("doc#s2").is_some());
        assert!(engine.cache().peek("doc-summary::doc").is_some());
    }

    #[test]
    fn try_serve_after_prefetch_is_warm_hit() {
        let mut engine = PrefetchEngine::new(10);
        let next = make_section_record("doc#s2", "doc", "Warm content", 1);
        engine.prefetch_sequential(Some(next), None, None);

        // First serve is a hit
        let result = engine.try_serve("doc#s2");
        assert!(result.is_some());
        assert_eq!(engine.metrics().hits, 1);
        assert_eq!(engine.metrics().misses, 0);
    }

    #[test]
    fn try_serve_miss_records_metric() {
        let mut engine = PrefetchEngine::new(10);
        let _ = engine.try_serve("nonexistent");
        assert_eq!(engine.metrics().hits, 0);
        assert_eq!(engine.metrics().misses, 1);
    }

    #[test]
    fn record_miss_increments_counter() {
        let mut engine = PrefetchEngine::new(10);
        engine.record_miss();
        engine.record_miss();
        assert_eq!(engine.metrics().misses, 2);
    }

    #[test]
    fn prefetch_respects_cache_capacity() {
        let mut engine = PrefetchEngine::new(2);

        // Fill cache with 3 sequential prefetches — only last 2 should remain
        for i in 0..3 {
            let section =
                make_section_record(&format!("s{i}"), "doc", &format!("text {i}"), i64::from(i));
            engine.prefetch_sequential(Some(section), None, None);
        }

        assert_eq!(engine.cache().len(), 2);
        assert!(engine.cache().peek("s0").is_none());
        assert!(engine.cache().peek("s1").is_some());
        assert!(engine.cache().peek("s2").is_some());
    }

    #[test]
    fn prefetch_with_no_next_and_no_summary_is_noop() {
        let mut engine = PrefetchEngine::new(10);
        engine.prefetch_sequential(None, None, None);
        assert!(engine.cache().is_empty());
    }

    #[test]
    fn default_capacity_is_50() {
        let engine = PrefetchEngine::with_default_capacity();
        assert_eq!(engine.cache().capacity(), 50);
    }
}
