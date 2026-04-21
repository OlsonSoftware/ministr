//! Prefetch engine and cache for speculative context pre-warming.
//!
//! The prefetch engine predicts what content an agent will need next based on
//! access patterns and pre-computes it into a cache. When the agent
//! requests pre-warmed content, it is served in <1ms instead of requiring a
//! full cold retrieval (50-200ms).
//!
//! # Strategies
//!
//! - **Sequential**: When the agent reads section N, pre-warm section N+1 and
//!   the parent document summary.
//! - **Topical**: Maintain a running topic vector (EMA of recent section
//!   embeddings) and pre-warm sections nearest to the current topic.
//! - **Structural**: Pre-warm sibling sections from the same document
//!   (adjacent by position in the document tree).
//! - **`CrossSession`**: Pre-warm sections that are frequently accessed across
//!   sessions or frequently co-accessed with the current section.
//!
//! # Architecture
//!
//! - [`PriorityCache`] — Priority-scored cache with confidence-weighted eviction
//!   based on SolidAttention (FAST '26) scheduling patterns.
//! - [`PrefetchEngine`] — orchestrates prefetch strategies and serves warm hits.
//! - [`TopicTracker`] — EMA-weighted running topic vector.

pub mod priority;

pub use priority::{PriorityCache, StrategyWeights};

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
    /// Cross-session analytics: frequently accessed or co-accessed sections.
    CrossSession,
    /// Survey expansion: parent sections of claim-level survey hits.
    SurveyExpand,
    /// Agent intent prediction: sections predicted from tool call patterns.
    AgentPlan,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
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
    /// Hits from cross-session prefetch entries.
    pub cross_session_hits: u64,
    /// Hits from survey-expand prefetch entries.
    pub survey_expand_hits: u64,
    /// Hits from agent-plan intent prediction entries.
    pub agent_plan_hits: u64,
}

impl PrefetchMetrics {
    /// Overall cache hit rate as a fraction (0.0–1.0).
    ///
    /// Returns 0.0 if no lookups have been performed.
    ///
    /// # Examples
    ///
    /// ```
    /// use ministr_core::session::PrefetchMetrics;
    ///
    /// let metrics = PrefetchMetrics { hits: 3, misses: 7, ..Default::default() };
    /// assert!((metrics.hit_rate() - 0.3).abs() < f64::EPSILON);
    ///
    /// let empty = PrefetchMetrics::default();
    /// assert!((empty.hit_rate() - 0.0).abs() < f64::EPSILON);
    /// ```
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
            PrefetchStrategy::CrossSession => self.cross_session_hits,
            PrefetchStrategy::SurveyExpand => self.survey_expand_hits,
            PrefetchStrategy::AgentPlan => self.agent_plan_hits,
        };
        strategy_hits as f64 / total as f64
    }
}

/// Tracks a running topic vector using exponential moving average (EMA)
/// of recent section embeddings.
///
/// After each `ministr_read`, the section's embedding is recorded. The topic
/// vector is the EMA-weighted average of the last K embeddings, giving
/// higher weight to recently accessed content. This vector can be used to
/// query the HNSW index for topically similar sections to pre-warm.
///
/// # Examples
///
/// ```
/// use ministr_core::session::prefetch::TopicTracker;
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
    /// Maintains the sliding window at `max_history` size. Rejects embeddings
    /// whose dimension differs from the first retained vector — a mid-session
    /// embedder swap would otherwise silently corrupt the running topic vector.
    pub fn record_access(&mut self, embedding: Vec<f32>) {
        if let Some(front) = self.recent_vectors.front()
            && front.len() != embedding.len()
        {
            tracing::warn!(
                expected_dim = front.len(),
                got_dim = embedding.len(),
                "TopicTracker: ignoring embedding with mismatched dimension"
            );
            return;
        }
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
/// After each `ministr_read` call, the engine predicts what the agent will
/// need next and inserts pre-computed entries into the [`PrefetchCache`].
/// Before cold retrieval, the engine checks the cache for a warm hit.
///
/// # Examples
///
/// ```
/// use ministr_core::session::prefetch::PrefetchEngine;
///
/// let engine = PrefetchEngine::new(50);
/// assert!(engine.cache().is_empty());
/// ```
pub struct PrefetchEngine {
    /// Priority-scored prefetch cache (recency decay + strategy weights
    /// + confidence-weighted eviction, as documented in the design).
    cache: PriorityCache,
    /// Running topic vector tracker for topical prefetch.
    topic_tracker: TopicTracker,
    /// Agent intent tracker for tool-call-pattern-based prediction.
    intent_tracker: IntentTracker,
}

impl PrefetchEngine {
    /// Create a new prefetch engine with the given cache capacity.
    #[must_use]
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            cache: PriorityCache::new(cache_capacity),
            topic_tracker: TopicTracker::with_defaults(),
            intent_tracker: IntentTracker::default(),
        }
    }

    /// Create a new prefetch engine with the default cache capacity.
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self {
            cache: PriorityCache::new(DEFAULT_CACHE_CAPACITY),
            topic_tracker: TopicTracker::with_defaults(),
            intent_tracker: IntentTracker::default(),
        }
    }

    /// Advance the cache's internal turn counter so recency decay actually
    /// moves. Call once per agent interaction.
    pub fn advance_turn(&mut self) {
        self.cache.advance_turn();
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
        self.cache.record_miss();
    }

    /// Pre-warm the cache after a read operation using sequential prefetch.
    ///
    /// Inserts the next section into the cache so the typical "read through
    /// a doc top-to-bottom" pattern serves warm. Document summaries used to
    /// be pre-warmed here too, but the daemon's read path only looks up
    /// plain section IDs via [`try_serve`], so those entries were dead
    /// writes occupying cache slots without ever being retrieved — they've
    /// been removed. Document-summary prefetch can come back once a
    /// retrieval path actually uses it.
    pub fn prefetch_sequential(
        &mut self,
        next_section: Option<crate::storage::SectionRecord>,
        claims_count: Option<usize>,
    ) {
        let Some(section) = next_section else { return };
        let token_count = count_tokens(&section.text);
        self.cache.insert_default(
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
            self.cache.insert_default(
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
            self.cache.insert_default(
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

    /// Pre-warm the cache with sections from cross-session analytics.
    ///
    /// Takes sections that are either frequently accessed across sessions
    /// or frequently co-accessed with the current section. Inserts those
    /// not already cached.
    pub fn prefetch_cross_session(
        &mut self,
        candidates: Vec<crate::storage::SectionRecord>,
        claims_counts: &HashMap<String, usize>,
    ) {
        for section in candidates {
            if self.cache.peek(&section.id.0).is_some() {
                continue;
            }
            let claims = claims_counts.get(&section.id.0).copied().unwrap_or(0);
            let token_count = count_tokens(&section.text);
            self.cache.insert_default(
                section.id.0.clone(),
                CacheEntry {
                    content_id: section.id.0,
                    text: section.text,
                    token_count,
                    heading_path: Some(section.heading_path),
                    summary: section.summary,
                    resolution: Resolution::Section,
                    claims_available: claims,
                    strategy: PrefetchStrategy::CrossSession,
                },
            );
        }
    }

    /// Pre-warm the cache with parent sections of claim-level survey hits.
    ///
    /// After a survey returns claim-level results, the parent sections are
    /// pre-warmed so that subsequent `ministr_read` calls can be served from
    /// the warm cache. Sections already in the cache or already delivered
    /// are skipped.
    pub fn prefetch_survey_expand(
        &mut self,
        sections: Vec<crate::storage::SectionRecord>,
        claims_counts: &HashMap<String, usize>,
    ) {
        for section in sections {
            if self.cache.peek(&section.id.0).is_some() {
                continue;
            }
            let claims = claims_counts.get(&section.id.0).copied().unwrap_or(0);
            let token_count = count_tokens(&section.text);
            self.cache.insert_default(
                section.id.0.clone(),
                CacheEntry {
                    content_id: section.id.0,
                    text: section.text,
                    token_count,
                    heading_path: Some(section.heading_path),
                    summary: section.summary,
                    resolution: Resolution::Section,
                    claims_available: claims,
                    strategy: PrefetchStrategy::SurveyExpand,
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
    pub fn cache(&self) -> &PriorityCache {
        &self.cache
    }

    /// Mutable access to the underlying cache.
    pub fn cache_mut(&mut self) -> &mut PriorityCache {
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

    /// Record a tool call for intent prediction.
    pub fn record_tool_call(&mut self, tool_name: &str, key_arg: &str) {
        self.intent_tracker.record_call(tool_name, key_arg);
    }

    /// Record survey results as predicted next reads.
    ///
    /// Clears previous survey predictions and stores the new section IDs.
    /// These become `AgentPlan` prefetch candidates on the next prefetch cycle.
    pub fn record_survey_results(&mut self, section_ids: Vec<String>) {
        self.intent_tracker.pending_survey_ids = section_ids;
    }

    /// Prefetch sections predicted by agent intent analysis.
    ///
    /// Inserts predicted section records into the cache with `AgentPlan` strategy.
    /// Respects a cap of `MAX_INTENT_PREFETCH` entries to avoid cache thrashing.
    pub fn prefetch_from_intent(&mut self, sections: Vec<crate::storage::SectionRecord>) {
        let mut inserted = 0;
        for section in sections {
            if inserted >= MAX_INTENT_PREFETCH {
                break;
            }
            if self.cache.peek(&section.id.0).is_some() {
                continue;
            }
            let token_count = crate::token::count_tokens(&section.text);
            let key = section.id.0.clone();
            self.cache.insert_default(
                key,
                CacheEntry {
                    content_id: section.id.0,
                    text: section.text,
                    token_count,
                    heading_path: Some(section.heading_path),
                    summary: section.summary,
                    resolution: Resolution::Section,
                    claims_available: 0,
                    strategy: PrefetchStrategy::AgentPlan,
                },
            );
            inserted += 1;
        }
    }

    /// Get predicted section IDs from the intent tracker.
    ///
    /// Returns the pending survey result IDs that haven't been prefetched yet.
    #[must_use]
    pub fn predicted_section_ids(&self) -> &[String] {
        &self.intent_tracker.pending_survey_ids
    }

    /// Evict cache entries for sections that have been invalidated.
    ///
    /// Called by the coherence engine when source files change. Prevents
    /// serving stale pre-warmed content after file modifications.
    pub fn invalidate(&mut self, stale_section_ids: &[String]) {
        for id in stale_section_ids {
            self.cache.remove(id);
        }
    }

    /// Clear all cached entries (useful after a full re-index).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Maximum number of entries prefetched per agent-intent trigger.
const MAX_INTENT_PREFETCH: usize = 5;

/// Tracks agent tool call patterns to predict future reads.
///
/// Observes the sequence of tool calls (name + key argument) and maintains
/// a list of predicted section IDs from the most recent survey results.
#[derive(Debug, Default)]
struct IntentTracker {
    /// Recent tool calls for pattern matching.
    recent_calls: VecDeque<ToolCallSignal>,
    /// Section IDs from the most recent `ministr_survey` — likely next reads.
    pending_survey_ids: Vec<String>,
}

/// A recorded tool call for intent analysis.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ToolCallSignal {
    tool_name: String,
    key_arg: String,
}

/// Maximum number of recent tool calls to track.
const MAX_INTENT_HISTORY: usize = 20;

impl IntentTracker {
    fn record_call(&mut self, tool_name: &str, key_arg: &str) {
        self.recent_calls.push_back(ToolCallSignal {
            tool_name: tool_name.to_string(),
            key_arg: key_arg.to_string(),
        });
        if self.recent_calls.len() > MAX_INTENT_HISTORY {
            self.recent_calls.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn hit_rate_calculation() {
        let mut metrics = PrefetchMetrics::default();
        assert!((metrics.hit_rate() - 0.0).abs() < f64::EPSILON);

        metrics.hits = 3;
        metrics.misses = 1;
        assert!((metrics.hit_rate() - 0.75).abs() < f64::EPSILON);
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

        engine.prefetch_sequential(Some(next), None);

        let entry = engine.try_serve("doc#s2");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.content_id, "doc#s2");
        assert_eq!(entry.text, "Section two text");
        assert_eq!(entry.resolution, Resolution::Section);
    }

    // The doc-summary pre-warming branch was removed because the daemon's
    // read path only looks up plain section IDs via `try_serve` — the
    // `doc-summary::{id}` entries were dead writes. If a retrieval path
    // for doc summaries lands later, re-add a test here.

    #[test]
    fn try_serve_after_prefetch_is_warm_hit() {
        let mut engine = PrefetchEngine::new(10);
        let next = make_section_record("doc#s2", "doc", "Warm content", 1);
        engine.prefetch_sequential(Some(next), None);

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

        // Fill cache with 3 sequential prefetches — exactly 2 must remain.
        // When all three entries share the same strategy and insert turn,
        // priority eviction breaks ties via HashMap iteration order, so we
        // can't assert *which* specific key survived — only that capacity
        // is enforced and the most-recent insert is always present.
        for i in 0..3 {
            let section =
                make_section_record(&format!("s{i}"), "doc", &format!("text {i}"), i64::from(i));
            engine.prefetch_sequential(Some(section), None);
        }

        assert_eq!(engine.cache().len(), 2);
        assert!(engine.cache().peek("s2").is_some(), "newest must survive");
    }

    #[test]
    fn prefetch_with_no_next_and_no_summary_is_noop() {
        let mut engine = PrefetchEngine::new(10);
        engine.prefetch_sequential(None, None);
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
        let mut cache = PriorityCache::new(10);
        cache.insert_default(
            "seq".to_string(),
            make_entry_with_strategy("seq", "sequential", PrefetchStrategy::Sequential),
        );
        cache.insert_default(
            "top".to_string(),
            make_entry_with_strategy("top", "topical", PrefetchStrategy::Topical),
        );
        cache.insert_default(
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
            cross_session_hits: 0,
            survey_expand_hits: 0,
            agent_plan_hits: 0,
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
        engine.prefetch_sequential(Some(s1), None);

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
        engine.prefetch_sequential(Some(s1), None);

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

    // --- Integration: sequential prediction accuracy ---

    #[test]
    fn sequential_linear_read_achieves_high_hit_rate() {
        // Simulate an agent reading sections 0..9 linearly through a document.
        // After reading section N, we prefetch section N+1.
        // The first read is always cold; subsequent reads should be warm.
        let mut engine = PrefetchEngine::new(20);
        let num_sections = 10;

        // Build all section records up front
        let sections: Vec<_> = (0..num_sections)
            .map(|i| {
                make_section_record(
                    &format!("doc#s{i}"),
                    "doc",
                    &format!("Content of section {i}"),
                    i64::from(i),
                )
            })
            .collect();

        for i in 0..num_sections {
            // Agent requests section i
            let _served = engine.try_serve(&format!("doc#s{i}"));

            // After reading, prefetch the next section (sequential strategy)
            let next = if i + 1 < num_sections {
                Some(sections[usize::try_from(i + 1).unwrap()].clone())
            } else {
                None
            };
            engine.prefetch_sequential(next, None);
        }

        let m = engine.metrics();
        // First read is cold (miss), reads 1..9 should all be warm hits
        assert_eq!(m.hits, 9, "expected 9 warm hits for linear sequential read");
        assert_eq!(m.misses, 1, "expected 1 cold miss (first read)");
        assert_eq!(m.sequential_hits, 9, "all hits should be sequential");
        assert!(
            m.hit_rate() > 0.85,
            "sequential hit rate should be >85%, got {:.2}",
            m.hit_rate()
        );
    }

    #[test]
    fn sequential_skip_pattern_has_lower_hit_rate() {
        // Simulate an agent that reads every other section (0, 2, 4, 6, 8).
        // Sequential prefetch warms N+1, so reading N+2 misses.
        let mut engine = PrefetchEngine::new(20);
        let num_sections = 10;

        let sections: Vec<_> = (0..num_sections)
            .map(|i| {
                make_section_record(
                    &format!("doc#s{i}"),
                    "doc",
                    &format!("Content {i}"),
                    i64::from(i),
                )
            })
            .collect();

        for i in (0..num_sections).step_by(2) {
            let _served = engine.try_serve(&format!("doc#s{i}"));
            let next = if i + 1 < num_sections {
                Some(sections[usize::try_from(i + 1).unwrap()].clone())
            } else {
                None
            };
            engine.prefetch_sequential(next, None);
        }

        let m = engine.metrics();
        // All 5 reads should be misses (we skip the prefetched sections)
        assert_eq!(
            m.misses, 5,
            "skip-reading should miss all prefetched sections"
        );
        assert_eq!(m.hits, 0, "no hits expected for skip pattern");
        assert!(
            (m.hit_rate()).abs() < f64::EPSILON,
            "hit rate should be 0% for skip pattern"
        );
    }

    // --- Integration: structural prediction accuracy ---

    #[test]
    fn structural_sibling_exploration_achieves_high_hit_rate() {
        // Simulate: agent reads section 0, then we prefetch siblings 1,2,3.
        // Agent then reads sections 1,2,3 (exploring siblings).
        let mut engine = PrefetchEngine::new(20);

        // Read section 0 (cold)
        let _ = engine.try_serve("doc#s0");

        // Prefetch siblings 1,2,3 via structural strategy
        let siblings = vec![
            make_section_record("doc#s1", "doc", "Sibling 1", 1),
            make_section_record("doc#s2", "doc", "Sibling 2", 2),
            make_section_record("doc#s3", "doc", "Sibling 3", 3),
        ];
        engine.prefetch_structural(siblings, &HashMap::new());

        // Agent reads the siblings
        let s1 = engine.try_serve("doc#s1");
        let s2 = engine.try_serve("doc#s2");
        let s3 = engine.try_serve("doc#s3");

        assert!(s1.is_some(), "sibling s1 should be warm");
        assert!(s2.is_some(), "sibling s2 should be warm");
        assert!(s3.is_some(), "sibling s3 should be warm");

        let m = engine.metrics();
        assert_eq!(m.hits, 3, "3 sibling reads should be warm hits");
        assert_eq!(m.misses, 1, "only first read is cold");
        assert_eq!(m.structural_hits, 3, "all hits from structural strategy");
        assert!(
            m.hit_rate() >= 0.75,
            "structural hit rate should be >=75%, got {:.2}",
            m.hit_rate()
        );
    }

    #[test]
    fn structural_only_warms_max_siblings() {
        // With 6 siblings available but MAX_STRUCTURAL_PREFETCH=3,
        // only 3 should be warm. Reads beyond that are misses.
        let mut engine = PrefetchEngine::new(20);

        let siblings: Vec<_> = (0..6)
            .map(|i| {
                make_section_record(
                    &format!("doc#s{i}"),
                    "doc",
                    &format!("Sib {i}"),
                    i64::from(i),
                )
            })
            .collect();
        engine.prefetch_structural(siblings, &HashMap::new());

        let mut warm_count = 0;
        for i in 0..6 {
            if engine.try_serve(&format!("doc#s{i}")).is_some() {
                warm_count += 1;
            }
        }

        assert_eq!(
            warm_count, MAX_STRUCTURAL_PREFETCH,
            "only {MAX_STRUCTURAL_PREFETCH} siblings should be pre-warmed"
        );
    }

    // --- Integration: topical prediction accuracy ---

    #[test]
    fn topical_browsing_warms_related_sections() {
        // Simulate: agent reads sections on a topic, topical prefetch warms
        // semantically similar sections. Agent then reads those sections.
        let mut engine = PrefetchEngine::new(20);

        // Agent reads a section and we record its embedding
        let _ = engine.try_serve("doc#intro"); // cold miss
        engine.record_topic_access(vec![1.0, 0.0, 0.0]);

        // Topical prefetch finds related sections
        let candidates = vec![
            make_section_record("doc#related1", "doc", "Related topic 1", 5),
            make_section_record("doc#related2", "doc", "Related topic 2", 6),
        ];
        engine.prefetch_topical(candidates, &HashMap::new());

        // Agent follows the topic and reads related sections
        let r1 = engine.try_serve("doc#related1");
        let r2 = engine.try_serve("doc#related2");

        assert!(r1.is_some(), "topically related s1 should be warm");
        assert!(r2.is_some(), "topically related s2 should be warm");

        let m = engine.metrics();
        assert_eq!(m.topical_hits, 2, "both hits from topical strategy");
        assert_eq!(m.hits, 2);
        assert_eq!(m.misses, 1);
    }

    #[test]
    fn topical_off_topic_reads_miss() {
        // Simulate: topical prefetch warms sections on topic A, but the agent
        // switches to a completely different topic B. All reads miss.
        let mut engine = PrefetchEngine::new(20);

        // Warm sections about topic A
        let candidates = vec![
            make_section_record("topicA#s1", "docA", "Topic A content", 0),
            make_section_record("topicA#s2", "docA", "More A content", 1),
        ];
        engine.prefetch_topical(candidates, &HashMap::new());

        // Agent reads topic B sections instead (not prefetched)
        let b1 = engine.try_serve("topicB#s1");
        let b2 = engine.try_serve("topicB#s2");

        assert!(b1.is_none(), "off-topic read should miss");
        assert!(b2.is_none(), "off-topic read should miss");

        let m = engine.metrics();
        assert_eq!(m.hits, 0);
        assert_eq!(m.misses, 2);
        assert!((m.hit_rate()).abs() < f64::EPSILON);
    }

    // --- Integration: mixed strategy hit rate ---

    #[test]
    fn mixed_strategy_session_tracks_per_strategy_rates() {
        // Simulate a realistic mixed session: sequential reading, then
        // structural exploration, then topical browsing.
        let mut engine = PrefetchEngine::new(30);

        // Phase 1: Sequential reading (sections 0→1→2)
        // Read s0 (cold)
        let _ = engine.try_serve("doc#s0");
        engine.prefetch_sequential(
            Some(make_section_record("doc#s1", "doc", "Section 1", 1)),
            None,
        );

        // Read s1 (warm, sequential)
        let _ = engine.try_serve("doc#s1");
        engine.prefetch_sequential(
            Some(make_section_record("doc#s2", "doc", "Section 2", 2)),
            None,
        );

        // Read s2 (warm, sequential)
        let _ = engine.try_serve("doc#s2");

        // Phase 2: Structural exploration from s2's document
        let siblings = vec![
            make_section_record("doc#s3", "doc", "Sibling 3", 3),
            make_section_record("doc#s4", "doc", "Sibling 4", 4),
        ];
        engine.prefetch_structural(siblings, &HashMap::new());

        // Read s3 (warm, structural)
        let _ = engine.try_serve("doc#s3");

        // Phase 3: Topical jump to a different document
        engine.record_topic_access(vec![0.0, 1.0, 0.0]);
        let topical = vec![make_section_record("other#t1", "other", "Topical match", 0)];
        engine.prefetch_topical(topical, &HashMap::new());

        // Read t1 (warm, topical)
        let _ = engine.try_serve("other#t1");

        // Read something completely unexpected (cold)
        let _ = engine.try_serve("random#unknown");

        let m = engine.metrics();
        // Total: 6 lookups — s0(miss), s1(seq hit), s2(seq hit), s3(struct hit),
        //        t1(topical hit), random(miss)
        assert_eq!(m.hits, 4, "4 warm hits across strategies");
        assert_eq!(m.misses, 2, "2 cold misses");
        assert_eq!(m.sequential_hits, 2, "2 sequential hits");
        assert_eq!(m.structural_hits, 1, "1 structural hit");
        assert_eq!(m.topical_hits, 1, "1 topical hit");

        // Overall hit rate: 4/6 ≈ 0.667
        let rate = m.hit_rate();
        assert!(
            (rate - 4.0 / 6.0).abs() < 1e-5,
            "expected ~66.7% overall hit rate, got {rate:.3}"
        );

        // Per-strategy rates
        assert!(
            (m.strategy_hit_rate(PrefetchStrategy::Sequential) - 2.0 / 6.0).abs() < 1e-5,
            "sequential strategy rate should be ~33.3%"
        );
        assert!(
            (m.strategy_hit_rate(PrefetchStrategy::Structural) - 1.0 / 6.0).abs() < 1e-5,
            "structural strategy rate should be ~16.7%"
        );
        assert!(
            (m.strategy_hit_rate(PrefetchStrategy::Topical) - 1.0 / 6.0).abs() < 1e-5,
            "topical strategy rate should be ~16.7%"
        );
    }

    // --- Integration: cache pressure under mixed workload ---

    #[test]
    fn cache_pressure_prefers_higher_strategy_weight() {
        // With priority-based eviction, higher-weight strategies outlive
        // lower-weight ones when the cache is under pressure. `Sequential`
        // (weight 1.0) should win over `Structural` (weight 0.8).
        let mut engine = PrefetchEngine::new(5);

        // Fill with 5 sequential entries.
        for i in 0..5 {
            let section = make_section_record(
                &format!("doc#s{i}"),
                "doc",
                &format!("Seq content {i}"),
                i64::from(i),
            );
            engine.prefetch_sequential(Some(section), None);
        }
        assert_eq!(engine.cache().len(), 5);

        // Add 3 structural entries under capacity pressure. Priority
        // eviction: once a structural lands, it's the lowest-priority entry
        // in the cache and gets evicted on the next insert. Net effect: at
        // most one structural survives, and sequential entries dominate.
        let siblings = vec![
            make_section_record("doc#str0", "doc", "Structural 0", 10),
            make_section_record("doc#str1", "doc", "Structural 1", 11),
            make_section_record("doc#str2", "doc", "Structural 2", 12),
        ];
        engine.prefetch_structural(siblings, &HashMap::new());

        assert_eq!(engine.cache().len(), 5);

        // At least 4 sequential entries survive (higher weight = preferred).
        let surviving_seq = (0..5)
            .filter(|i| engine.cache().peek(&format!("doc#s{i}")).is_some())
            .count();
        assert!(
            surviving_seq >= 4,
            "expected ≥4 sequentials to survive, got {surviving_seq}",
        );

        // At most 1 structural survives (the most recent insert — everything
        // older got churned out by the evict-lowest selecting it back).
        let surviving_struct = ["doc#str0", "doc#str1", "doc#str2"]
            .iter()
            .filter(|id| engine.cache().peek(id).is_some())
            .count();
        assert!(
            surviving_struct <= 1,
            "expected ≤1 structural to survive, got {surviving_struct}",
        );
    }

    #[test]
    fn cache_pressure_high_throughput_maintains_metrics() {
        // Simulate 100 sequential prefetches with a small cache (10).
        // Only the most recent entries survive, but metrics accumulate all hits/misses.
        let mut engine = PrefetchEngine::new(10);

        // Prefetch sections 0..99
        for i in 0..100_i32 {
            let section = make_section_record(
                &format!("doc#s{i}"),
                "doc",
                &format!("Content {i}"),
                i64::from(i),
            );
            engine.prefetch_sequential(Some(section), None);
        }

        // Try to serve all 100 — only last 10 should be warm
        for i in 0..100_i32 {
            let _ = engine.try_serve(&format!("doc#s{i}"));
        }

        let m = engine.metrics();
        assert_eq!(m.hits, 10, "only 10 entries fit in cache");
        assert_eq!(m.misses, 90, "90 evicted entries are misses");
        assert!(
            (m.hit_rate() - 0.1).abs() < 1e-5,
            "hit rate should be 10% with 10/100 capacity ratio"
        );
    }

    // --- Integration: topic vector evolution ---

    #[test]
    fn topic_vector_evolves_with_access_pattern() {
        // Verify that the topic vector shifts as the agent changes topics,
        // which is essential for topical prefetch accuracy.
        let mut engine = PrefetchEngine::new(20);

        // Phase 1: Agent reads "Rust" content (embedding dimension 0)
        for _ in 0..3 {
            engine.record_topic_access(vec![1.0, 0.0, 0.0]);
        }
        let topic_rust = engine.topic_vector().unwrap();
        assert!(
            topic_rust[0] > 0.9,
            "topic should be dominated by dim 0 (Rust)"
        );

        // Phase 2: Agent switches to "Python" content (embedding dimension 1)
        for _ in 0..5 {
            engine.record_topic_access(vec![0.0, 1.0, 0.0]);
        }
        let topic_python = engine.topic_vector().unwrap();
        assert!(
            topic_python[1] > topic_python[0],
            "after topic shift, dim 1 (Python) should dominate over dim 0 (Rust)"
        );

        // The shift happened because EMA weights recent accesses higher
        assert!(
            topic_python[1] > 0.5,
            "Python dimension should be >50%, got {:.3}",
            topic_python[1]
        );
    }

    #[test]
    fn metrics_reset_allows_per_phase_measurement() {
        // Verify that resetting metrics mid-session allows measuring
        // hit rate for different phases independently.
        let mut engine = PrefetchEngine::new(20);

        // Phase 1: Sequential reading
        let _ = engine.try_serve("s0"); // miss
        engine.prefetch_sequential(Some(make_section_record("s1", "doc", "Next", 1)), None);
        let _ = engine.try_serve("s1"); // hit

        let phase1 = engine.metrics();
        assert_eq!(phase1.hits, 1);
        assert_eq!(phase1.misses, 1);
        assert!((phase1.hit_rate() - 0.5).abs() < 1e-5);

        // Reset for phase 2 measurement
        engine.cache_mut().reset_metrics();

        // Phase 2: Structural exploration (all warm)
        let siblings = vec![
            make_section_record("s2", "doc", "Sib 2", 2),
            make_section_record("s3", "doc", "Sib 3", 3),
        ];
        engine.prefetch_structural(siblings, &HashMap::new());
        let _ = engine.try_serve("s2"); // hit
        let _ = engine.try_serve("s3"); // hit

        let phase2 = engine.metrics();
        assert_eq!(phase2.hits, 2);
        assert_eq!(phase2.misses, 0);
        assert!(
            (phase2.hit_rate() - 1.0).abs() < 1e-5,
            "phase 2 should be 100% hit rate"
        );
    }

    // --- Cross-session prefetch tests ---

    #[test]
    fn prefetch_cross_session_inserts_candidates() {
        let mut engine = PrefetchEngine::new(10);
        let sections = vec![
            make_section_record("s1", "doc1", "Cross-session section 1", 0),
            make_section_record("s2", "doc1", "Cross-session section 2", 1),
        ];
        let claims_counts = std::collections::HashMap::new();

        engine.prefetch_cross_session(sections, &claims_counts);

        assert_eq!(engine.cache().len(), 2);
        assert!(engine.cache().peek("s1").is_some());
        assert!(engine.cache().peek("s2").is_some());
        assert_eq!(
            engine.cache().peek("s1").unwrap().strategy,
            PrefetchStrategy::CrossSession
        );
    }

    #[test]
    fn prefetch_cross_session_skips_cached() {
        let mut engine = PrefetchEngine::new(10);

        // Pre-warm s1 via sequential
        let s1 = make_section_record("s1", "doc1", "Already cached", 0);
        engine.prefetch_sequential(Some(s1), None);

        // Try cross-session with s1 and s2
        let sections = vec![
            make_section_record("s1", "doc1", "Already cached", 0),
            make_section_record("s2", "doc1", "New section", 1),
        ];
        let claims_counts = std::collections::HashMap::new();
        engine.prefetch_cross_session(sections, &claims_counts);

        assert_eq!(engine.cache().len(), 2);
        // s1 should still be sequential strategy (not overwritten)
        assert_eq!(
            engine.cache().peek("s1").unwrap().strategy,
            PrefetchStrategy::Sequential
        );
        assert_eq!(
            engine.cache().peek("s2").unwrap().strategy,
            PrefetchStrategy::CrossSession
        );
    }

    #[test]
    fn cross_session_hit_records_strategy_metric() {
        let mut engine = PrefetchEngine::new(10);
        let sections = vec![make_section_record(
            "s1",
            "doc1",
            "Cross-session content",
            0,
        )];
        let claims_counts = std::collections::HashMap::new();
        engine.prefetch_cross_session(sections, &claims_counts);

        // Serve from cache
        let entry = engine.try_serve("s1");
        assert!(entry.is_some());

        let m = engine.metrics();
        assert_eq!(m.cross_session_hits, 1);
        assert_eq!(m.hits, 1);
    }

    #[test]
    fn cross_session_uses_claims_counts() {
        let mut engine = PrefetchEngine::new(10);
        let sections = vec![make_section_record("s1", "doc1", "With claims", 0)];
        let mut claims_counts = std::collections::HashMap::new();
        claims_counts.insert("s1".to_string(), 7);

        engine.prefetch_cross_session(sections, &claims_counts);

        let entry = engine.cache().peek("s1").unwrap();
        assert_eq!(entry.claims_available, 7);
    }

    // --- Survey-expand prefetch tests ---

    #[test]
    fn survey_expand_prewarms_parent_sections() {
        let mut engine = PrefetchEngine::new(10);

        // Simulate 3 claim hits from 2 different parent sections
        let sections = vec![
            make_section_record("docs/auth.md#tokens", "docs/auth.md", "Token text", 0),
            make_section_record(
                "docs/api.md#rate-limits",
                "docs/api.md",
                "Rate limit text",
                0,
            ),
        ];
        let claims_counts = std::collections::HashMap::new();

        engine.prefetch_survey_expand(sections, &claims_counts);

        // Both parent sections should be in the cache
        assert!(engine.cache().peek("docs/auth.md#tokens").is_some());
        assert!(engine.cache().peek("docs/api.md#rate-limits").is_some());
        assert_eq!(engine.cache().len(), 2);

        // Strategy should be SurveyExpand
        let entry = engine.cache().peek("docs/auth.md#tokens").unwrap();
        assert_eq!(entry.strategy, PrefetchStrategy::SurveyExpand);
    }

    #[test]
    fn survey_expand_skips_already_cached() {
        let mut engine = PrefetchEngine::new(10);

        // Pre-warm one section via structural prefetch
        let pre_existing = vec![make_section_record(
            "docs/auth.md#tokens",
            "docs/auth.md",
            "Already cached",
            0,
        )];
        engine.prefetch_structural(pre_existing, &HashMap::new());
        assert_eq!(engine.cache().len(), 1);

        // Now survey-expand tries to pre-warm same section + a new one
        let sections = vec![
            make_section_record("docs/auth.md#tokens", "docs/auth.md", "Token text", 0),
            make_section_record(
                "docs/api.md#rate-limits",
                "docs/api.md",
                "Rate limit text",
                0,
            ),
        ];
        engine.prefetch_survey_expand(sections, &HashMap::new());

        // Should have 2 total (old one kept, new one added)
        assert_eq!(engine.cache().len(), 2);
        // The pre-existing entry should keep its original strategy
        let existing = engine.cache().peek("docs/auth.md#tokens").unwrap();
        assert_eq!(existing.strategy, PrefetchStrategy::Structural);
    }

    #[test]
    fn survey_expand_hit_tracked_separately() {
        let mut engine = PrefetchEngine::new(10);

        let sections = vec![make_section_record(
            "docs/auth.md#tokens",
            "docs/auth.md",
            "Token text",
            0,
        )];
        engine.prefetch_survey_expand(sections, &HashMap::new());

        // Serve from cache (records a hit)
        let served = engine.try_serve("docs/auth.md#tokens");
        assert!(served.is_some());

        let m = engine.metrics();
        assert_eq!(m.hits, 1);
        assert_eq!(m.survey_expand_hits, 1);
        assert_eq!(m.structural_hits, 0);
    }

    // --- IntentTracker tests ---

    #[test]
    fn intent_tracker_records_calls() {
        let mut engine = PrefetchEngine::with_default_capacity();
        engine.record_tool_call("ministr_survey", "auth middleware");
        engine.record_tool_call("ministr_read", "section-123");
        assert_eq!(engine.intent_tracker.recent_calls.len(), 2);
    }

    #[test]
    fn intent_tracker_caps_history() {
        let mut engine = PrefetchEngine::with_default_capacity();
        for i in 0..30 {
            engine.record_tool_call("ministr_survey", &format!("query-{i}"));
        }
        assert_eq!(engine.intent_tracker.recent_calls.len(), MAX_INTENT_HISTORY);
    }

    #[test]
    fn record_survey_results_replaces_previous() {
        let mut engine = PrefetchEngine::with_default_capacity();
        engine.record_survey_results(vec!["s1".into(), "s2".into()]);
        assert_eq!(engine.predicted_section_ids(), &["s1", "s2"]);

        engine.record_survey_results(vec!["s3".into()]);
        assert_eq!(engine.predicted_section_ids(), &["s3"]);
    }

    #[test]
    fn prefetch_from_intent_inserts_and_caps() {
        use crate::storage::SectionRecord;
        use crate::types::{ContentId, SectionId};

        let mut engine = PrefetchEngine::with_default_capacity();

        let sections: Vec<SectionRecord> = (0..10)
            .map(|i| SectionRecord {
                id: SectionId(format!("s{i}")),
                document_id: ContentId("doc".into()),
                heading_path: vec![format!("Section {i}")],
                depth: 1,
                text: format!("Content of section {i}"),
                summary: None,
                position: i,
            })
            .collect();

        engine.prefetch_from_intent(sections);

        // Should cap at MAX_INTENT_PREFETCH (5)
        let mut hits = 0;
        for i in 0..10 {
            if engine.try_serve(&format!("s{i}")).is_some() {
                hits += 1;
            }
        }
        assert_eq!(hits, 5, "should prefetch at most 5 sections");

        // Verify AgentPlan strategy recorded on hits
        let m = engine.metrics();
        assert_eq!(m.agent_plan_hits, 5);
    }

    #[test]
    fn prefetch_from_intent_skips_cached() {
        use crate::storage::SectionRecord;
        use crate::types::{ContentId, SectionId};

        let mut engine = PrefetchEngine::with_default_capacity();

        // Pre-warm s0
        engine.cache.insert_default(
            "s0".into(),
            CacheEntry {
                content_id: "s0".into(),
                text: "existing".into(),
                token_count: 10,
                heading_path: None,
                summary: None,
                resolution: Resolution::Section,
                claims_available: 0,
                strategy: PrefetchStrategy::Sequential,
            },
        );

        let sections = vec![SectionRecord {
            id: SectionId("s0".into()),
            document_id: ContentId("doc".into()),
            heading_path: vec!["S0".into()],
            depth: 1,
            text: "new content".into(),
            summary: None,
            position: 0,
        }];

        engine.prefetch_from_intent(sections);

        // Should not overwrite existing entry
        let entry = engine.try_serve("s0").unwrap();
        assert_eq!(entry.text, "existing");
        assert_eq!(entry.strategy, PrefetchStrategy::Sequential);
    }
}
