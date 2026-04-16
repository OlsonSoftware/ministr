//! Priority-scored prefetch cache with confidence-weighted eviction.
//!
//! Replaces simple LRU eviction with a priority formula that considers
//! strategy confidence, recency decay, and access frequency. Based on
//! scheduling patterns from SolidAttention (USENIX FAST '26).
//!
//! Priority formula:
//!   `(confidence * strategy_weight) * 0.95^turns_since_insert * (1 + log2(1 + access_count))`

use std::collections::HashMap;

use super::{CacheEntry, PrefetchMetrics, PrefetchStrategy};

/// Default strategy weights for priority computation.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyWeights {
    pub sequential: f32,
    pub structural: f32,
    pub topical: f32,
    pub cross_session: f32,
    pub survey_expand: f32,
    pub agent_plan: f32,
}

impl Default for StrategyWeights {
    fn default() -> Self {
        Self {
            sequential: 1.0,
            structural: 0.8,
            topical: 0.7,
            cross_session: 0.6,
            survey_expand: 0.9,
            agent_plan: 0.5,
        }
    }
}

impl StrategyWeights {
    /// Get the weight for a strategy.
    #[must_use]
    pub fn weight_for(&self, strategy: PrefetchStrategy) -> f32 {
        match strategy {
            PrefetchStrategy::Sequential => self.sequential,
            PrefetchStrategy::Topical => self.topical,
            PrefetchStrategy::Structural => self.structural,
            PrefetchStrategy::CrossSession => self.cross_session,
            PrefetchStrategy::SurveyExpand => self.survey_expand,
            PrefetchStrategy::AgentPlan => self.agent_plan,
        }
    }
}

/// A cache entry with priority metadata.
#[derive(Debug, Clone)]
pub struct PriorityCacheEntry {
    /// The underlying cache entry.
    pub entry: CacheEntry,
    /// Computed priority score (higher = more valuable, evict last).
    pub priority: f64,
    /// Turn number when this entry was inserted.
    pub insert_turn: u32,
    /// Number of times this entry has been accessed (cache hits).
    pub access_count: u32,
    /// Confidence score from the strategy that produced this entry (0.0–1.0).
    pub confidence: f32,
}

/// Priority-scored prefetch cache.
///
/// Evicts the entry with the lowest priority score when at capacity, instead
/// of simple LRU. When all priorities are equal, behaves like FIFO for
/// backward compatibility with the old `PrefetchCache`.
pub struct PriorityCache {
    entries: HashMap<String, PriorityCacheEntry>,
    capacity: usize,
    metrics: PrefetchMetrics,
    current_turn: u32,
    weights: StrategyWeights,
}

/// Recency decay base — 0.95^turns means ~60% priority after 10 turns.
const RECENCY_DECAY: f64 = 0.95;

impl PriorityCache {
    /// Create a new priority cache with the given capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            metrics: PrefetchMetrics::default(),
            current_turn: 0,
            weights: StrategyWeights::default(),
        }
    }

    /// Create with custom strategy weights.
    #[must_use]
    pub fn with_weights(capacity: usize, weights: StrategyWeights) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            capacity,
            metrics: PrefetchMetrics::default(),
            current_turn: 0,
            weights,
        }
    }

    /// Advance the internal turn counter (call after each agent interaction).
    pub fn advance_turn(&mut self) {
        self.current_turn += 1;
    }

    /// Current turn number.
    #[must_use]
    pub fn current_turn(&self) -> u32 {
        self.current_turn
    }

    /// Look up an entry, boosting its access count and priority.
    ///
    /// Records a hit or miss in the metrics.
    pub fn get(&mut self, key: &str) -> Option<&CacheEntry> {
        if let Some(entry) = self.entries.get_mut(key) {
            self.metrics.hits += 1;
            match entry.entry.strategy {
                PrefetchStrategy::Sequential => self.metrics.sequential_hits += 1,
                PrefetchStrategy::Topical => self.metrics.topical_hits += 1,
                PrefetchStrategy::Structural => self.metrics.structural_hits += 1,
                PrefetchStrategy::CrossSession => self.metrics.cross_session_hits += 1,
                PrefetchStrategy::SurveyExpand => self.metrics.survey_expand_hits += 1,
                PrefetchStrategy::AgentPlan => self.metrics.agent_plan_hits += 1,
            }
            entry.access_count += 1;
            entry.priority = compute_priority(
                &self.weights, self.current_turn,
                entry.confidence, entry.entry.strategy,
                entry.insert_turn, entry.access_count,
            );
            Some(&entry.entry)
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    /// Look up an entry without updating metrics or priority.
    #[must_use]
    pub fn peek(&self, key: &str) -> Option<&CacheEntry> {
        self.entries.get(key).map(|e| &e.entry)
    }

    /// Insert an entry with confidence score.
    ///
    /// If at capacity, evicts the lowest-priority entry.
    pub fn insert(&mut self, key: String, entry: CacheEntry, confidence: f32) {
        if self.capacity == 0 {
            return;
        }

        if self.entries.contains_key(&key) {
            let existing = self.entries.get_mut(&key).unwrap();
            existing.entry = entry;
            existing.confidence = confidence;
            existing.priority = compute_priority(
                &self.weights, self.current_turn,
                confidence, existing.entry.strategy, existing.insert_turn, existing.access_count,
            );
            return;
        }

        // Evict lowest priority if at capacity.
        if self.entries.len() >= self.capacity {
            self.evict_lowest();
        }

        let priority = compute_priority(
            &self.weights, self.current_turn,
            confidence, entry.strategy, self.current_turn, 0,
        );
        self.entries.insert(key, PriorityCacheEntry {
            entry,
            priority,
            insert_turn: self.current_turn,
            access_count: 0,
            confidence,
        });
    }

    /// Insert with default confidence (1.0) for backward compatibility.
    pub fn insert_default(&mut self, key: String, entry: CacheEntry) {
        self.insert(key, entry, 1.0);
    }

    /// Remove an entry by key.
    pub fn remove(&mut self, key: &str) -> Option<CacheEntry> {
        self.entries.remove(key).map(|e| e.entry)
    }

    /// Remove entries for stale section IDs (coherence invalidation).
    pub fn invalidate(&mut self, stale_ids: &[String]) {
        for id in stale_ids {
            self.entries.remove(id);
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

    /// Reset metrics counters.
    pub fn reset_metrics(&mut self) {
        self.metrics = PrefetchMetrics::default();
    }

    /// Iterate over cached content IDs.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Evict the entry with the lowest priority score.
    fn evict_lowest(&mut self) {
        if let Some(key) = self.lowest_priority_key() {
            self.entries.remove(&key);
        }
    }

    /// Find the key with the lowest priority score.
    fn lowest_priority_key(&self) -> Option<String> {
        self.entries
            .iter()
            .min_by(|a, b| {
                a.1.priority
                    .partial_cmp(&b.1.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(k, _)| k.clone())
    }
}

/// Compute priority score for an entry (free function to avoid borrow conflicts).
#[allow(clippy::cast_precision_loss)]
fn compute_priority(
    weights: &StrategyWeights,
    current_turn: u32,
    confidence: f32,
    strategy: PrefetchStrategy,
    insert_turn: u32,
    access_count: u32,
) -> f64 {
    let weight = f64::from(weights.weight_for(strategy));
    let conf = f64::from(confidence);
    let turns_since = current_turn.saturating_sub(insert_turn) as f64;
    let recency = RECENCY_DECAY.powf(turns_since);
    let frequency = 1.0 + (1.0 + f64::from(access_count)).log2();
    conf * weight * recency * frequency
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Resolution;

    fn make_entry(id: &str, strategy: PrefetchStrategy) -> CacheEntry {
        CacheEntry {
            content_id: id.to_string(),
            text: format!("text for {id}"),
            token_count: 5,
            heading_path: None,
            summary: None,
            resolution: Resolution::Section,
            claims_available: 0,
            strategy,
        }
    }

    #[test]
    fn eviction_removes_lowest_priority() {
        let mut cache = PriorityCache::new(2);
        cache.insert("low".into(), make_entry("low", PrefetchStrategy::AgentPlan), 0.3);
        cache.insert("high".into(), make_entry("high", PrefetchStrategy::Sequential), 1.0);

        // Cache is full — inserting a third should evict "low"
        cache.insert("mid".into(), make_entry("mid", PrefetchStrategy::Structural), 0.7);

        assert!(cache.peek("high").is_some(), "high-priority should survive");
        assert!(cache.peek("mid").is_some(), "mid-priority should be present");
        assert!(cache.peek("low").is_none(), "low-priority should be evicted");
    }

    #[test]
    fn access_count_boosts_priority() {
        let mut cache = PriorityCache::new(3);
        cache.insert("a".into(), make_entry("a", PrefetchStrategy::Topical), 0.5);
        cache.insert("b".into(), make_entry("b", PrefetchStrategy::Topical), 0.5);

        // Access "a" several times to boost its priority
        for _ in 0..5 {
            cache.get("a");
        }

        // Now insert enough entries to cause eviction — "b" should be evicted first
        cache.insert("c".into(), make_entry("c", PrefetchStrategy::Sequential), 1.0);
        // Cache is full (a, b, c). Insert one more:
        cache.insert("d".into(), make_entry("d", PrefetchStrategy::Sequential), 1.0);

        assert!(cache.peek("a").is_some(), "frequently-accessed 'a' should survive");
        assert!(cache.peek("b").is_none(), "rarely-accessed 'b' should be evicted");
    }

    #[test]
    fn recency_decay_reduces_old_entries() {
        let mut cache = PriorityCache::new(2);
        cache.insert("old".into(), make_entry("old", PrefetchStrategy::Sequential), 1.0);

        // Advance many turns
        for _ in 0..20 {
            cache.advance_turn();
        }

        cache.insert("new".into(), make_entry("new", PrefetchStrategy::Sequential), 1.0);

        // Cache is full. Insert one more — "old" should be evicted due to recency decay
        cache.insert("newer".into(), make_entry("newer", PrefetchStrategy::Sequential), 1.0);

        assert!(cache.peek("old").is_none(), "old entry should be evicted by recency decay");
        assert!(cache.peek("new").is_some());
        assert!(cache.peek("newer").is_some());
    }

    #[test]
    fn invalidation_removes_by_id() {
        let mut cache = PriorityCache::new(10);
        cache.insert("a".into(), make_entry("a", PrefetchStrategy::Sequential), 1.0);
        cache.insert("b".into(), make_entry("b", PrefetchStrategy::Sequential), 1.0);
        cache.insert("c".into(), make_entry("c", PrefetchStrategy::Sequential), 1.0);

        cache.invalidate(&["a".into(), "c".into()]);

        assert!(cache.peek("a").is_none());
        assert!(cache.peek("b").is_some());
        assert!(cache.peek("c").is_none());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn zero_capacity_accepts_nothing() {
        let mut cache = PriorityCache::new(0);
        cache.insert("a".into(), make_entry("a", PrefetchStrategy::Sequential), 1.0);
        assert!(cache.is_empty());
    }

    #[test]
    fn metrics_track_hits_and_misses() {
        let mut cache = PriorityCache::new(10);
        cache.insert("a".into(), make_entry("a", PrefetchStrategy::Sequential), 1.0);

        cache.get("a"); // hit
        cache.get("b"); // miss

        let m = cache.metrics();
        assert_eq!(m.hits, 1);
        assert_eq!(m.misses, 1);
        assert_eq!(m.sequential_hits, 1);
    }
}
