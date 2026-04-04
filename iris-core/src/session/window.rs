//! Context window estimation model.
//!
//! The [`WindowEstimator`] tracks cumulative tokens delivered to an agent and
//! models how the agent's context window fills up. It supports configurable
//! eviction assumptions (FIFO or LRU) to estimate which previously-delivered
//! content may have been dropped from the agent's working context.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::types::EvictionPolicy;

/// A record of a single delivery in the window estimator.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowEntry {
    /// Identifier for the delivered content.
    content_id: String,
    /// Token cost of this delivery.
    token_count: usize,
    /// Monotonic sequence number for ordering.
    sequence: u64,
}

/// Estimates the agent's context window usage based on delivered content.
///
/// Maintains an ordered queue of deliveries and tracks cumulative token usage.
/// When the estimated usage exceeds the window capacity, entries are evicted
/// according to the configured [`EvictionPolicy`].
///
/// # Examples
///
/// ```
/// use iris_core::session::{WindowEstimator, EvictionPolicy};
///
/// let mut estimator = WindowEstimator::new(1000, EvictionPolicy::Fifo);
///
/// estimator.record("s1", 300);
/// estimator.record("s2", 400);
/// assert_eq!(estimator.estimated_used(), 700);
/// assert_eq!(estimator.estimated_remaining(), 300);
///
/// // Recording more content causes FIFO eviction of oldest
/// estimator.record("s3", 500);
/// assert!(estimator.estimated_used() <= 1000);
/// ```
pub struct WindowEstimator {
    /// Maximum context window capacity in tokens.
    capacity: usize,
    /// Eviction policy to apply when capacity is exceeded.
    policy: EvictionPolicy,
    /// Ordered queue of deliveries (front = oldest).
    entries: VecDeque<WindowEntry>,
    /// Current total token count in the estimated window.
    current_tokens: usize,
    /// Monotonic sequence counter.
    next_sequence: u64,
    /// Number of content entries evicted from the estimated window.
    evicted_count: usize,
}

/// Summary of the estimated window state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowStatus {
    /// Total capacity in tokens.
    pub capacity: usize,
    /// Estimated tokens currently in the window.
    pub used: usize,
    /// Estimated tokens remaining.
    pub remaining: usize,
    /// Number of active entries in the window.
    pub entry_count: usize,
    /// Total entries evicted since session start.
    pub evicted_count: usize,
}

impl WindowEstimator {
    /// Create a new window estimator with the given capacity and eviction policy.
    #[must_use]
    pub fn new(capacity: usize, policy: EvictionPolicy) -> Self {
        Self {
            capacity,
            policy,
            entries: VecDeque::new(),
            current_tokens: 0,
            next_sequence: 0,
            evicted_count: 0,
        }
    }

    /// Record a content delivery in the window model.
    ///
    /// If the new delivery would push total tokens over capacity, existing
    /// entries are evicted according to the eviction policy until there is
    /// room or the queue is empty.
    ///
    /// Returns the content IDs of any entries evicted to make room.
    pub fn record(&mut self, content_id: &str, token_count: usize) -> Vec<String> {
        self.record_with_scores(content_id, token_count, None)
    }

    /// Record a delivery with optional FSRS retrievability scores.
    ///
    /// Under [`EvictionPolicy::Fsrs`], eviction selects the entry with the
    /// lowest retrievability score instead of the oldest entry.
    /// FIFO and LRU policies ignore the scores.
    pub fn record_with_scores(
        &mut self,
        content_id: &str,
        token_count: usize,
        scores: Option<&std::collections::HashMap<String, f64>>,
    ) -> Vec<String> {
        // If this content was already delivered, remove the old entry first
        if let Some(pos) = self.entries.iter().position(|e| e.content_id == content_id) {
            if let Some(old) = self.entries.remove(pos) {
                self.current_tokens = self.current_tokens.saturating_sub(old.token_count);
            }
        }

        let entry = WindowEntry {
            content_id: content_id.to_string(),
            token_count,
            sequence: self.next_sequence,
        };
        self.next_sequence += 1;

        self.current_tokens += token_count;
        self.entries.push_back(entry);

        // Evict until we're within capacity
        self.evict_to_capacity(scores)
    }

    /// Mark a content ID as recently accessed (LRU only).
    ///
    /// Moves the entry to the back of the queue so it won't be evicted soon.
    /// No-op under FIFO policy.
    pub fn touch(&mut self, content_id: &str) {
        if self.policy != EvictionPolicy::Lru {
            return;
        }

        if let Some(pos) = self.entries.iter().position(|e| e.content_id == content_id) {
            if let Some(mut entry) = self.entries.remove(pos) {
                entry.sequence = self.next_sequence;
                self.next_sequence += 1;
                self.entries.push_back(entry);
            }
        }
    }

    /// Estimated tokens currently in the agent's context window.
    #[must_use]
    pub fn estimated_used(&self) -> usize {
        self.current_tokens
    }

    /// Estimated tokens remaining in the agent's context window.
    #[must_use]
    pub fn estimated_remaining(&self) -> usize {
        self.capacity.saturating_sub(self.current_tokens)
    }

    /// The window capacity in tokens.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether the window is at or over capacity.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.current_tokens >= self.capacity
    }

    /// Get a snapshot of the current window state.
    #[must_use]
    pub fn status(&self) -> WindowStatus {
        WindowStatus {
            capacity: self.capacity,
            used: self.current_tokens,
            remaining: self.estimated_remaining(),
            entry_count: self.entries.len(),
            evicted_count: self.evicted_count,
        }
    }

    /// Number of content entries evicted from the estimated window.
    #[must_use]
    pub fn evicted_count(&self) -> usize {
        self.evicted_count
    }

    /// Check whether a content ID is currently in the estimated window.
    #[must_use]
    pub fn is_in_window(&self, content_id: &str) -> bool {
        self.entries.iter().any(|e| e.content_id == content_id)
    }

    /// Force-evict a specific content ID from the window.
    ///
    /// Used when the agent signals that content has been dropped from its context
    /// (either explicitly via `iris_evicted` or implicitly via re-request).
    /// Returns `true` if the content was found and removed.
    pub fn force_evict(&mut self, content_id: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.content_id == content_id) {
            if let Some(entry) = self.entries.remove(pos) {
                self.current_tokens = self.current_tokens.saturating_sub(entry.token_count);
                self.evicted_count += 1;
                return true;
            }
        }
        false
    }

    /// Evict entries until we're within capacity.
    ///
    /// Under FIFO/LRU, evicts from the front of the queue.
    /// Under FSRS, evicts the entry with the lowest retrievability score.
    ///
    /// Returns the content IDs of evicted entries so callers can apply
    /// compression (e.g. bookmarks) instead of losing the content entirely.
    fn evict_to_capacity(
        &mut self,
        scores: Option<&std::collections::HashMap<String, f64>>,
    ) -> Vec<String> {
        let mut evicted_ids = Vec::new();
        while self.current_tokens > self.capacity {
            let victim = match (&self.policy, scores) {
                (EvictionPolicy::Fsrs, Some(s)) if !self.entries.is_empty() => {
                    // Find the entry with the lowest retrievability
                    let min_idx = self
                        .entries
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| {
                            let ra = s.get(&a.content_id).copied().unwrap_or(0.0);
                            let rb = s.get(&b.content_id).copied().unwrap_or(0.0);
                            ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|(i, _)| i);
                    min_idx.and_then(|i| self.entries.remove(i))
                }
                _ => self.entries.pop_front(),
            };
            if let Some(evicted) = victim {
                self.current_tokens = self.current_tokens.saturating_sub(evicted.token_count);
                self.evicted_count += 1;
                evicted_ids.push(evicted.content_id);
            } else {
                break;
            }
        }
        evicted_ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_estimator_is_empty() {
        let est = WindowEstimator::new(1000, EvictionPolicy::Fifo);
        assert_eq!(est.estimated_used(), 0);
        assert_eq!(est.estimated_remaining(), 1000);
        assert_eq!(est.capacity(), 1000);
        assert!(!est.is_full());
        assert_eq!(est.evicted_count(), 0);
    }

    #[test]
    fn record_tracks_tokens() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);

        est.record("s1", 300);
        assert_eq!(est.estimated_used(), 300);
        assert_eq!(est.estimated_remaining(), 700);

        est.record("s2", 400);
        assert_eq!(est.estimated_used(), 700);
        assert_eq!(est.estimated_remaining(), 300);
    }

    #[test]
    fn fifo_eviction_when_over_capacity() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 200);
        est.record("s2", 200);
        // Now at 400/500

        est.record("s3", 300);
        // Would be 700 > 500, so s1 (200) gets evicted -> 500

        assert!(!est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
        assert!(est.is_in_window("s3"));
        assert_eq!(est.estimated_used(), 500);
        assert_eq!(est.evicted_count(), 1);
    }

    #[test]
    fn fifo_evicts_multiple_entries() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 100);
        est.record("s2", 100);
        est.record("s3", 100);
        // At 300/500

        est.record("s4", 400);
        // Would be 700 > 500, need to evict s1 (100) -> 600 > 500, evict s2 (100) -> 500

        assert!(!est.is_in_window("s1"));
        assert!(!est.is_in_window("s2"));
        assert!(est.is_in_window("s3"));
        assert!(est.is_in_window("s4"));
        assert_eq!(est.estimated_used(), 500);
        assert_eq!(est.evicted_count(), 2);
    }

    #[test]
    fn lru_touch_prevents_eviction() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Lru);

        est.record("s1", 200);
        est.record("s2", 200);
        // At 400/500

        // Touch s1, making s2 the LRU candidate
        est.touch("s1");

        est.record("s3", 300);
        // Would be 700 > 500, evict LRU (front of queue) = s2

        assert!(est.is_in_window("s1"), "s1 was touched, should survive");
        assert!(!est.is_in_window("s2"), "s2 was LRU, should be evicted");
        assert!(est.is_in_window("s3"));
        assert_eq!(est.evicted_count(), 1);
    }

    #[test]
    fn touch_is_noop_for_fifo() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 200);
        est.record("s2", 200);
        est.touch("s1"); // Should do nothing for FIFO

        est.record("s3", 300);
        // FIFO evicts s1 first regardless of touch

        assert!(!est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
    }

    #[test]
    fn re_recording_updates_existing_entry() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);

        est.record("s1", 300);
        est.record("s2", 200);
        assert_eq!(est.estimated_used(), 500);

        // Re-record s1 with different token count
        est.record("s1", 150);
        assert_eq!(est.estimated_used(), 350); // 200 + 150
        assert!(est.is_in_window("s1"));
    }

    #[test]
    fn status_snapshot() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);
        est.record("s1", 200);
        est.record("s2", 200);

        let status = est.status();
        assert_eq!(status.capacity, 500);
        assert_eq!(status.used, 400);
        assert_eq!(status.remaining, 100);
        assert_eq!(status.entry_count, 2);
        assert_eq!(status.evicted_count, 0);
    }

    #[test]
    fn status_after_eviction() {
        let mut est = WindowEstimator::new(300, EvictionPolicy::Fifo);
        est.record("s1", 200);
        est.record("s2", 200);

        let status = est.status();
        assert_eq!(status.used, 200);
        assert_eq!(status.entry_count, 1);
        assert_eq!(status.evicted_count, 1);
    }

    #[test]
    fn is_in_window_for_absent_content() {
        let est = WindowEstimator::new(1000, EvictionPolicy::Fifo);
        assert!(!est.is_in_window("nonexistent"));
    }

    #[test]
    fn large_single_entry_evicts_everything() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);
        est.record("s1", 100);
        est.record("s2", 100);

        // This single entry exceeds capacity. Eviction removes s1, s2,
        // then "big" itself since 600 > 500 — the window ends up empty.
        est.record("big", 600);
        assert!(!est.is_in_window("s1"));
        assert!(!est.is_in_window("s2"));
        assert!(!est.is_in_window("big"));
        assert_eq!(est.estimated_used(), 0);
        assert_eq!(est.evicted_count(), 3);
    }

    #[test]
    fn touch_nonexistent_is_noop() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Lru);
        est.record("s1", 100);
        est.touch("nonexistent"); // Should not panic
        assert_eq!(est.estimated_used(), 100);
    }

    #[test]
    fn window_status_serializes() {
        let status = WindowStatus {
            capacity: 1000,
            used: 500,
            remaining: 500,
            entry_count: 3,
            evicted_count: 1,
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: WindowStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, status);
    }

    // --- Additional exhaustive tests ---

    #[test]
    fn lru_multiple_touches_reorder() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Lru);

        est.record("s1", 150);
        est.record("s2", 150);
        est.record("s3", 150);
        // At 450/500, queue order: s1, s2, s3

        // Touch s1, then s2 — order becomes s3, s1, s2
        est.touch("s1");
        est.touch("s2");

        // Adding s4 should evict s3 (now the LRU front)
        est.record("s4", 200);
        // Would be 650 > 500, evict s3 (150) -> 500

        assert!(!est.is_in_window("s3"), "s3 should be evicted as LRU");
        assert!(est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
        assert!(est.is_in_window("s4"));
    }

    #[test]
    fn re_recording_moves_to_back() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 150);
        est.record("s2", 150);
        // Queue: s1, s2

        // Re-record s1 — removes old entry and appends new one
        est.record("s1", 100);
        // Queue: s2, s1 (new)

        // Add large entry to trigger eviction
        est.record("s3", 400);
        // Would be 100 + 150 + 400 = 650 > 500
        // Evict s2 (front) -> 500

        assert!(!est.is_in_window("s2"), "s2 should be evicted first (FIFO)");
        assert!(
            est.is_in_window("s1"),
            "s1 was re-recorded, should be at back"
        );
        assert!(est.is_in_window("s3"));
    }

    #[test]
    fn exact_capacity_no_eviction() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 200);
        est.record("s2", 300);
        // Exactly 500 == capacity, no eviction needed

        assert!(est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
        assert_eq!(est.estimated_used(), 500);
        assert!(est.is_full());
        assert_eq!(est.evicted_count(), 0);
    }

    #[test]
    fn one_over_capacity_triggers_eviction() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);

        est.record("s1", 200);
        est.record("s2", 301);
        // 501 > 500, evict s1 (200) -> 301

        assert!(!est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
        assert_eq!(est.estimated_used(), 301);
    }

    #[test]
    fn zero_capacity_evicts_everything() {
        let mut est = WindowEstimator::new(0, EvictionPolicy::Fifo);

        est.record("s1", 10);
        // 10 > 0, evict s1 -> 0

        assert!(!est.is_in_window("s1"));
        assert_eq!(est.estimated_used(), 0);
        assert_eq!(est.evicted_count(), 1);
    }

    #[test]
    fn many_small_entries_then_one_large() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);

        for i in 0..10 {
            est.record(&format!("s{i}"), 10);
        }
        assert_eq!(est.estimated_used(), 100);
        assert_eq!(est.evicted_count(), 0);

        // One large entry evicts many small ones
        est.record("big", 80);
        // Would be 180 > 100, evict s0..s7 (80 tokens) -> 100

        assert_eq!(est.estimated_used(), 100);
        // s0 through s7 evicted (8 * 10 = 80 tokens freed)
        assert!(
            est.evicted_count() >= 8,
            "should evict at least 8 entries: {}",
            est.evicted_count()
        );
    }

    #[test]
    fn window_status_consistency_after_many_operations() {
        let mut est = WindowEstimator::new(200, EvictionPolicy::Fifo);

        // Series of records and re-records
        est.record("a", 50);
        est.record("b", 50);
        est.record("c", 50);
        est.record("a", 30); // re-record
        est.record("d", 80);
        // After re-record of a: b=50, c=50, a=30, then d=80
        // Total = 50+50+30+80 = 210 > 200, evict b -> 160

        let status = est.status();
        assert_eq!(status.used + status.remaining, status.capacity);
        assert_eq!(status.used, est.estimated_used());
        assert_eq!(status.remaining, est.estimated_remaining());
        assert_eq!(status.evicted_count, est.evicted_count());
    }

    #[test]
    fn lru_touch_then_fifo_eviction_order() {
        // Touch should be no-op for FIFO
        let mut est = WindowEstimator::new(300, EvictionPolicy::Fifo);

        est.record("s1", 100);
        est.record("s2", 100);
        est.touch("s1"); // no-op for FIFO

        est.record("s3", 200);
        // 400 > 300, evict s1 (FIFO front) -> 300

        assert!(!est.is_in_window("s1"), "FIFO ignores touch");
        assert!(est.is_in_window("s2"));
        assert!(est.is_in_window("s3"));
    }

    #[test]
    fn zero_token_record() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);

        est.record("empty", 0);
        assert!(est.is_in_window("empty"));
        assert_eq!(est.estimated_used(), 0);
        assert_eq!(est.status().entry_count, 1);
    }

    #[test]
    fn estimated_remaining_saturates_at_zero() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);

        est.record("s1", 100);
        assert_eq!(est.estimated_remaining(), 0);

        // Even after capacity is exactly met, remaining should be 0 not negative
        assert_eq!(est.estimated_remaining(), 0);
    }

    // --- force_evict tests ---

    #[test]
    fn force_evict_removes_entry_and_frees_tokens() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);

        est.record("s1", 300);
        est.record("s2", 200);
        assert_eq!(est.estimated_used(), 500);

        let removed = est.force_evict("s1");
        assert!(removed);
        assert!(!est.is_in_window("s1"));
        assert!(est.is_in_window("s2"));
        assert_eq!(est.estimated_used(), 200);
        assert_eq!(est.evicted_count(), 1);
    }

    #[test]
    fn force_evict_nonexistent_returns_false() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);
        est.record("s1", 100);

        let removed = est.force_evict("nonexistent");
        assert!(!removed);
        assert_eq!(est.estimated_used(), 100);
    }

    #[test]
    fn force_evict_already_evicted_returns_false() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);
        est.record("s1", 60);
        est.record("s2", 60);
        // s1 auto-evicted (120 > 100)
        assert!(!est.is_in_window("s1"));

        let removed = est.force_evict("s1");
        assert!(!removed, "already evicted content should not be found");
    }

    #[test]
    fn force_evict_middle_entry() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);
        est.record("s1", 100);
        est.record("s2", 200);
        est.record("s3", 300);

        est.force_evict("s2");
        assert!(est.is_in_window("s1"));
        assert!(!est.is_in_window("s2"));
        assert!(est.is_in_window("s3"));
        assert_eq!(est.estimated_used(), 400);
    }

    #[test]
    fn force_evict_updates_status() {
        let mut est = WindowEstimator::new(500, EvictionPolicy::Fifo);
        est.record("s1", 200);
        est.record("s2", 200);

        est.force_evict("s1");

        let status = est.status();
        assert_eq!(status.used, 200);
        assert_eq!(status.remaining, 300);
        assert_eq!(status.entry_count, 1);
        assert_eq!(status.evicted_count, 1);
    }

    #[test]
    fn record_returns_evicted_ids() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);

        // Fill to capacity — no eviction yet
        let evicted = est.record("s1", 60);
        assert!(evicted.is_empty());
        let evicted = est.record("s2", 30);
        assert!(evicted.is_empty());

        // This pushes over capacity, evicting s1
        let evicted = est.record("s3", 50);
        assert_eq!(evicted, vec!["s1"]);
        assert_eq!(est.estimated_used(), 80); // s2(30) + s3(50)
    }

    #[test]
    fn record_returns_multiple_evicted_ids() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);

        est.record("s1", 30);
        est.record("s2", 30);
        est.record("s3", 30);

        // Insert a large item that evicts both s1 and s2
        let evicted = est.record("big", 80);
        assert_eq!(evicted, vec!["s1", "s2", "s3"]);
        assert_eq!(est.estimated_used(), 80);
    }

    #[test]
    fn evict_to_capacity_returns_empty_when_under_capacity() {
        let mut est = WindowEstimator::new(1000, EvictionPolicy::Fifo);
        est.record("s1", 100);
        // Under capacity — no eviction
        let evicted = est.record("s2", 100);
        assert!(evicted.is_empty());
    }

    // --- FSRS eviction policy tests ---

    #[test]
    fn fsrs_evicts_lowest_retrievability_first() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
        est.record("a", 40);
        est.record("b", 40);

        let mut scores = std::collections::HashMap::new();
        scores.insert("a".to_string(), 0.9); // high R — should survive
        scores.insert("b".to_string(), 0.1); // low R — should be evicted
        scores.insert("c".to_string(), 1.0); // just accessed — highest R

        // Insert c (40 tokens) → pushes over capacity (120 > 100), evicts b
        let evicted = est.record_with_scores("c", 40, Some(&scores));
        assert_eq!(evicted, vec!["b"], "should evict entry with lowest R");
    }

    #[test]
    fn fsrs_without_scores_falls_back_to_fifo() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
        est.record("a", 40);
        est.record("b", 40);

        // No scores → falls back to pop_front (FIFO)
        let evicted = est.record_with_scores("c", 40, None);
        assert_eq!(evicted, vec!["a"], "without scores, should evict oldest");
    }

    #[test]
    fn fifo_ignores_scores() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);
        est.record("a", 40);
        est.record("b", 40);

        let mut scores = std::collections::HashMap::new();
        scores.insert("a".to_string(), 0.9); // high R — FIFO should still evict first
        scores.insert("b".to_string(), 0.1);

        let evicted = est.record_with_scores("c", 40, Some(&scores));
        assert_eq!(
            evicted,
            vec!["a"],
            "FIFO should evict oldest regardless of scores"
        );
    }

    #[test]
    fn fsrs_unknown_content_gets_zero_retrievability() {
        let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
        est.record("known", 40);
        est.record("unknown", 40);

        let mut scores = std::collections::HashMap::new();
        scores.insert("known".to_string(), 0.5); // R=0.5
        // "unknown" not in scores → defaults to 0.0 → evicted first

        let evicted = est.record_with_scores("c", 40, Some(&scores));
        assert_eq!(
            evicted,
            vec!["unknown"],
            "unscored content should be evicted first"
        );
    }
}
