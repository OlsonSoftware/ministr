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
//! - **Topical**: Maintain a running topic vector (EMA of recent section
//!   embeddings) and pre-warm sections nearest to the current topic.
//! - **Structural**: Pre-warm sibling sections from the same document
//!   (adjacent by position in the document tree).
//!
//! # Architecture
//!
//! - [`PrefetchCache`] — LRU cache with pre-computed text, token count, and
//!   heading path. Default capacity 50 items.
//! - [`PrefetchEngine`] — orchestrates prefetch strategies, triggers pre-warming
//!   after tool calls, and serves warm cache hits.
//! - [`TopicTracker`] — maintains an EMA-weighted running topic vector from
//!   the last K section embeddings for topical prefetch prediction.

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

use crate::token::count_tokens;
use crate::types::Resolution;

/// Default number of items the prefetch cache can hold.
const DEFAULT_CACHE_CAPACITY: usize = 50;

/// Default number of recent section embeddings to track for topical prefetch.
const DEFAULT_TOPIC_HISTORY: usize = 5;

/// Default EMA decay factor for the topic vector (higher = more weight on recent).
const DEFAULT_TOPIC_ALPHA: f32 = 0.3;

/// Maximum number of sibling sections to pre-warm per structural prefetch.
const MAX_STRUCTURAL_PREFETCH: usize = 3;

/// The strategy that warmed a cache entry.
///
/// Tracked per entry so that hit rate metrics can be broken down by strategy,
/// revealing which prediction method is most effective for a given session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefetchStrategy {
    /// Sequential locality: section N+1 after reading section N.
    Sequential,
    /// Topical similarity: sections nearest to the running topic vector.
    Topical,
    /// Structural proximity: sibling sections from the same document.
    Structural,
}

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
    /// Which prefetch strategy warmed this entry.
    pub strategy: PrefetchStrategy,
}

/// Hit/miss metrics for the prefetch cache, broken down by strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct PrefetchMetrics {
    /// Total number of cache hits (warm responses).
    pub hits: u64,
    /// Total number of cache misses (cold retrievals).
    pub misses: u64,
    /// Hits from sequential prefetch entries.
    pub sequential_hits: u64,
    /// Hits from topical prefetch entries.
    pub topical_hits: u64,
    /// Hits from structural prefetch entries.
    pub structural_hits: u64,
}

impl PrefetchMetrics {
    /// Overall cache hit rate as a fraction (0.0–1.0).
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

    /// Hit rate for a specific strategy as a fraction (0.0–1.0).
    ///
    /// Returns 0.0 if no lookups have been performed.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn strategy_hit_rate(&self, strategy: PrefetchStrategy) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        let strategy_hits = match strategy {
            PrefetchStrategy::Sequential => self.sequential_hits,
            PrefetchStrategy::Topical => self.topical_hits,
            PrefetchStrategy::Structural => self.structural_hits,
        };
        strategy_hits as f64 / total as f64
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
/// use iris_core::session::prefetch::{PrefetchCache, CacheEntry, PrefetchStrategy};
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
///     strategy: PrefetchStrategy::Sequential,
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
    /// Records a hit or miss in the metrics. Hits are also attributed to
    /// the strategy that warmed the entry.
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
        if self.entries.contains_key(key) {
            self.metrics.hits += 1;
            // Attribute hit to the strategy that warmed this entry
            if let Some(entry) = self.entries.get(key) {
                match entry.strategy {
                    PrefetchStrategy::Sequential => self.metrics.sequential_hits += 1,
                    PrefetchStrategy::Topical => self.metrics.topical_hits += 1,
                    PrefetchStrategy::Structural => self.metrics.structural_hits += 1,
                }
            }
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

/// Tracks a running topic vector using exponential moving average (EMA)
/// of recent section embeddings.
///
/// After each `iris_read`, the section's embedding is recorded. The topic
/// vector is the EMA-weighted average of the last K embeddings, giving
/// higher weight to recently accessed content. This vector can be used to
/// query the HNSW index for topically similar sections to pre-warm.
///
/// # Examples
///
/// ```
/// use iris_core::session::prefetch::TopicTracker;
///
/// let mut tracker = TopicTracker::new(3, 0.3);
/// assert!(tracker.topic_vector().is_none());
///
/// tracker.record_access(vec![1.0, 0.0, 0.0]);
/// assert!(tracker.topic_vector().is_some());
/// ```
pub struct TopicTracker {
    /// Recent section embeddings (newest at back).
    recent_vectors: VecDeque<Vec<f32>>,
    /// Maximum number of embeddings to retain.
    max_history: usize,
    /// EMA decay factor (0.0–1.0). Higher means more weight on recent vectors.
    alpha: f32,
}

impl TopicTracker {
    /// Create a new topic tracker with the given history window and decay factor.
    #[must_use]
    pub fn new(max_history: usize, alpha: f32) -> Self {
        Self {
            recent_vectors: VecDeque::with_capacity(max_history),
            max_history,
            alpha,
        }
    }

    /// Create a topic tracker with default parameters (K=5, α=0.3).
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_TOPIC_HISTORY, DEFAULT_TOPIC_ALPHA)
    }

    /// Record a section embedding after an access.
    ///
    /// Maintains the sliding window at `max_history` size.
    pub fn record_access(&mut self, embedding: Vec<f32>) {
        if self.recent_vectors.len() >= self.max_history {
            self.recent_vectors.pop_front();
        }
        self.recent_vectors.push_back(embedding);
    }

    /// Compute the EMA-weighted topic vector from recent embeddings.
    ///
    /// Returns `None` if no embeddings have been recorded. The most recent
    /// embedding has the highest weight, decaying exponentially by `alpha`
    /// for older entries.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn topic_vector(&self) -> Option<Vec<f32>> {
        if self.recent_vectors.is_empty() {
            return None;
        }

        let dim = self.recent_vectors[0].len();
        let mut result = vec![0.0f32; dim];
        let mut weight_sum = 0.0f32;

        // Iterate oldest to newest; newest gets highest weight
        for (i, vec) in self.recent_vectors.iter().enumerate() {
            // Weight: (1 - alpha)^(n - 1 - i) where n = len
            let age = (self.recent_vectors.len() - 1 - i) as f32;
            let weight = (1.0 - self.alpha).powf(age);
            weight_sum += weight;
            for (j, &v) in vec.iter().enumerate() {
                if j < dim {
                    result[j] += v * weight;
                }
            }
        }

        // Normalize by total weight
        if weight_sum > 0.0 {
            for v in &mut result {
                *v /= weight_sum;
            }
        }

        Some(result)
    }

    /// Number of embeddings currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.recent_vectors.len()
    }

    /// Whether no embeddings have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.recent_vectors.is_empty()
    }

    /// The maximum history window size.
    #[must_use]
    pub fn max_history(&self) -> usize {
        self.max_history
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
    /// Running topic vector tracker for topical prefetch.
    topic_tracker: TopicTracker,
}

impl PrefetchEngine {
    /// Create a new prefetch engine with the given cache capacity.
    #[must_use]
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            cache: PrefetchCache::new(cache_capacity),
            topic_tracker: TopicTracker::with_defaults(),
        }
    }

    /// Create a new prefetch engine with the default cache capacity.
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self {
            cache: PrefetchCache::with_default_capacity(),
            topic_tracker: TopicTracker::with_defaults(),
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
                    strategy: PrefetchStrategy::Sequential,
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
                    strategy: PrefetchStrategy::Sequential,
                },
            );
        }
    }

    /// Pre-warm the cache with sibling sections from the same document.
    ///
    /// Takes all sections from the parent document and inserts up to
    /// [`MAX_STRUCTURAL_PREFETCH`] siblings that are not already cached.
    /// Siblings are sections adjacent to the current read position.
    pub fn prefetch_structural(
        &mut self,
        siblings: Vec<crate::storage::SectionRecord>,
        claims_counts: &HashMap<String, usize>,
    ) {
        let mut inserted = 0;
        for section in siblings {
            if inserted >= MAX_STRUCTURAL_PREFETCH {
                break;
            }
            // Skip sections already in cache
            if self.cache.peek(&section.id.0).is_some() {
                continue;
            }
            let claims = claims_counts.get(&section.id.0).copied().unwrap_or(0);
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
                    claims_available: claims,
                    strategy: PrefetchStrategy::Structural,
                },
            );
            inserted += 1;
        }
    }

    /// Pre-warm the cache with topically similar sections.
    ///
    /// Takes candidate sections scored by similarity to the running topic
    /// vector (from vector index search) and inserts those not already cached.
    pub fn prefetch_topical(
        &mut self,
        candidates: Vec<crate::storage::SectionRecord>,
        claims_counts: &HashMap<String, usize>,
    ) {
        for section in candidates {
            // Skip sections already in cache
            if self.cache.peek(&section.id.0).is_some() {
                continue;
            }
            let claims = claims_counts.get(&section.id.0).copied().unwrap_or(0);
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
                    claims_available: claims,
                    strategy: PrefetchStrategy::Topical,
                },
            );
        }
    }

    /// Record a section embedding for topical prefetch tracking.
    pub fn record_topic_access(&mut self, embedding: Vec<f32>) {
        self.topic_tracker.record_access(embedding);
    }

    /// Get the current topic vector for index queries.
    ///
    /// Returns `None` if no section embeddings have been recorded yet.
    #[must_use]
    pub fn topic_vector(&self) -> Option<Vec<f32>> {
        self.topic_tracker.topic_vector()
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

    /// Read-only access to the topic tracker.
    #[must_use]
    pub fn topic_tracker(&self) -> &TopicTracker {
        &self.topic_tracker
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
            strategy: PrefetchStrategy::Sequential,
        }
    }

    fn make_entry_with_strategy(id: &str, text: &str, strategy: PrefetchStrategy) -> CacheEntry {
        CacheEntry {
            content_id: id.to_string(),
            text: text.to_string(),
            token_count: count_tokens(text),
            heading_path: None,
            summary: None,
            resolution: Resolution::Section,
            claims_available: 0,
            strategy,
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

    // --- TopicTracker tests ---

    #[test]
    fn topic_tracker_empty_returns_none() {
        let tracker = TopicTracker::new(5, 0.3);
        assert!(tracker.topic_vector().is_none());
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn topic_tracker_single_vector_returns_it() {
        let mut tracker = TopicTracker::new(5, 0.3);
        tracker.record_access(vec![1.0, 0.0, 0.0]);

        let topic = tracker.topic_vector().unwrap();
        assert_eq!(topic.len(), 3);
        assert!((topic[0] - 1.0).abs() < 1e-5);
        assert!((topic[1]).abs() < 1e-5);
        assert!((topic[2]).abs() < 1e-5);
    }

    #[test]
    fn topic_tracker_ema_weights_recent_higher() {
        let mut tracker = TopicTracker::new(5, 0.5);

        // First access: [1, 0, 0] (older, lower weight)
        tracker.record_access(vec![1.0, 0.0, 0.0]);
        // Second access: [0, 1, 0] (newer, higher weight)
        tracker.record_access(vec![0.0, 1.0, 0.0]);

        let topic = tracker.topic_vector().unwrap();
        // With alpha=0.5, weights are:
        //   older (i=0): (1-0.5)^1 = 0.5
        //   newer (i=1): (1-0.5)^0 = 1.0
        // Weighted sum: [0.5, 1.0, 0] / 1.5 = [0.333, 0.667, 0]
        assert!(
            topic[1] > topic[0],
            "newer vector should have higher weight"
        );
    }

    #[test]
    fn topic_tracker_respects_max_history() {
        let mut tracker = TopicTracker::new(2, 0.3);

        tracker.record_access(vec![1.0, 0.0]);
        tracker.record_access(vec![0.0, 1.0]);
        tracker.record_access(vec![0.5, 0.5]);

        assert_eq!(tracker.len(), 2);
        // The first vector [1.0, 0.0] should be evicted
    }

    #[test]
    fn topic_tracker_with_defaults() {
        let tracker = TopicTracker::with_defaults();
        assert_eq!(tracker.max_history(), DEFAULT_TOPIC_HISTORY);
        assert!(tracker.is_empty());
    }

    // --- Per-strategy metrics tests ---

    #[test]
    fn metrics_track_strategy_hits() {
        let mut cache = PrefetchCache::new(10);
        cache.insert(
            "seq".to_string(),
            make_entry_with_strategy("seq", "sequential", PrefetchStrategy::Sequential),
        );
        cache.insert(
            "top".to_string(),
            make_entry_with_strategy("top", "topical", PrefetchStrategy::Topical),
        );
        cache.insert(
            "str".to_string(),
            make_entry_with_strategy("str", "structural", PrefetchStrategy::Structural),
        );

        let _ = cache.get("seq"); // sequential hit
        let _ = cache.get("top"); // topical hit
        let _ = cache.get("str"); // structural hit
        let _ = cache.get("top"); // another topical hit
        let _ = cache.get("miss"); // miss

        let m = cache.metrics();
        assert_eq!(m.hits, 4);
        assert_eq!(m.misses, 1);
        assert_eq!(m.sequential_hits, 1);
        assert_eq!(m.topical_hits, 2);
        assert_eq!(m.structural_hits, 1);
    }

    #[test]
    fn strategy_hit_rate() {
        let metrics = PrefetchMetrics {
            hits: 4,
            misses: 1,
            sequential_hits: 1,
            topical_hits: 2,
            structural_hits: 1,
        };

        assert!((metrics.strategy_hit_rate(PrefetchStrategy::Sequential) - 0.2).abs() < 1e-5);
        assert!((metrics.strategy_hit_rate(PrefetchStrategy::Topical) - 0.4).abs() < 1e-5);
        assert!((metrics.strategy_hit_rate(PrefetchStrategy::Structural) - 0.2).abs() < 1e-5);
    }

    #[test]
    fn strategy_hit_rate_empty() {
        let metrics = PrefetchMetrics::default();
        assert!((metrics.strategy_hit_rate(PrefetchStrategy::Sequential)).abs() < f64::EPSILON);
    }

    // --- Structural prefetch tests ---

    #[test]
    fn prefetch_structural_inserts_siblings() {
        let mut engine = PrefetchEngine::new(10);
        let siblings = vec![
            make_section_record("doc#s1", "doc", "Sibling one", 0),
            make_section_record("doc#s2", "doc", "Sibling two", 1),
            make_section_record("doc#s3", "doc", "Sibling three", 2),
        ];

        engine.prefetch_structural(siblings, &HashMap::new());

        assert!(engine.cache().peek("doc#s1").is_some());
        assert!(engine.cache().peek("doc#s2").is_some());
        assert!(engine.cache().peek("doc#s3").is_some());
    }

    #[test]
    fn prefetch_structural_respects_max_limit() {
        let mut engine = PrefetchEngine::new(10);
        let siblings: Vec<_> = (0..10)
            .map(|i| {
                make_section_record(&format!("s{i}"), "doc", &format!("text {i}"), i64::from(i))
            })
            .collect();

        engine.prefetch_structural(siblings, &HashMap::new());

        // Only MAX_STRUCTURAL_PREFETCH (3) should be inserted
        assert_eq!(engine.cache().len(), MAX_STRUCTURAL_PREFETCH);
    }

    #[test]
    fn prefetch_structural_skips_cached() {
        let mut engine = PrefetchEngine::new(10);

        // Pre-warm s1 via sequential
        let s1 = make_section_record("doc#s1", "doc", "Section one", 0);
        engine.prefetch_sequential(Some(s1), None, None);

        // Now structural prefetch with s1 and s2
        let siblings = vec![
            make_section_record("doc#s1", "doc", "Section one", 0),
            make_section_record("doc#s2", "doc", "Section two", 1),
        ];
        engine.prefetch_structural(siblings, &HashMap::new());

        // s1 should remain sequential strategy (not overwritten)
        assert_eq!(
            engine.cache().peek("doc#s1").unwrap().strategy,
            PrefetchStrategy::Sequential
        );
        // s2 should be structural
        assert_eq!(
            engine.cache().peek("doc#s2").unwrap().strategy,
            PrefetchStrategy::Structural
        );
    }

    #[test]
    fn prefetch_structural_uses_claims_counts() {
        let mut engine = PrefetchEngine::new(10);
        let siblings = vec![make_section_record("doc#s1", "doc", "Section", 0)];
        let mut counts = HashMap::new();
        counts.insert("doc#s1".to_string(), 5);

        engine.prefetch_structural(siblings, &counts);

        assert_eq!(engine.cache().peek("doc#s1").unwrap().claims_available, 5);
    }

    // --- Topical prefetch tests ---

    #[test]
    fn prefetch_topical_inserts_candidates() {
        let mut engine = PrefetchEngine::new(10);
        let candidates = vec![
            make_section_record("topic#s1", "doc", "Similar section", 0),
            make_section_record("topic#s2", "doc", "Another similar", 1),
        ];

        engine.prefetch_topical(candidates, &HashMap::new());

        assert!(engine.cache().peek("topic#s1").is_some());
        assert!(engine.cache().peek("topic#s2").is_some());
        assert_eq!(
            engine.cache().peek("topic#s1").unwrap().strategy,
            PrefetchStrategy::Topical
        );
    }

    #[test]
    fn prefetch_topical_skips_cached() {
        let mut engine = PrefetchEngine::new(10);

        // Pre-warm via sequential
        let s1 = make_section_record("s1", "doc", "Text", 0);
        engine.prefetch_sequential(Some(s1), None, None);

        // Topical should skip s1
        let candidates = vec![make_section_record("s1", "doc", "Text", 0)];
        engine.prefetch_topical(candidates, &HashMap::new());

        assert_eq!(
            engine.cache().peek("s1").unwrap().strategy,
            PrefetchStrategy::Sequential
        );
    }

    // --- Engine topic tracking integration ---

    #[test]
    fn engine_records_topic_and_produces_vector() {
        let mut engine = PrefetchEngine::new(10);
        assert!(engine.topic_vector().is_none());

        engine.record_topic_access(vec![1.0, 0.0, 0.0, 0.0]);
        assert!(engine.topic_vector().is_some());

        engine.record_topic_access(vec![0.0, 1.0, 0.0, 0.0]);
        let topic = engine.topic_vector().unwrap();
        assert_eq!(topic.len(), 4);
    }

    #[test]
    fn prefetch_strategy_serde() {
        let json = serde_json::to_string(&PrefetchStrategy::Sequential).unwrap();
        assert_eq!(json, r#""sequential""#);

        let json = serde_json::to_string(&PrefetchStrategy::Topical).unwrap();
        assert_eq!(json, r#""topical""#);

        let json = serde_json::to_string(&PrefetchStrategy::Structural).unwrap();
        assert_eq!(json, r#""structural""#);
    }

    #[test]
    fn warm_hit_from_structural_records_strategy_metric() {
        let mut engine = PrefetchEngine::new(10);
        let siblings = vec![make_section_record("s1", "doc", "Text", 0)];
        engine.prefetch_structural(siblings, &HashMap::new());

        // Serve the structural entry
        let entry = engine.try_serve("s1");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().strategy, PrefetchStrategy::Structural);

        let m = engine.metrics();
        assert_eq!(m.hits, 1);
        assert_eq!(m.structural_hits, 1);
        assert_eq!(m.sequential_hits, 0);
        assert_eq!(m.topical_hits, 0);
    }
}
