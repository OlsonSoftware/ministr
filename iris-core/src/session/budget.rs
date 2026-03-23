//! Budget tracker for agent context window management.
//!
//! The [`BudgetTracker`] monitors token usage against a configurable budget
//! and reports pressure levels. When usage crosses the pressure threshold
//! (default 80%), responses should be auto-compressed to claim-level and
//! eviction recommendations should be attached.

use serde::{Deserialize, Serialize};

use super::eviction::{EvictionCandidate, EvictionRanker};
use super::types::Session;
use super::window::WindowEstimator;

/// Pressure level indicating how close the agent is to its context budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PressureLevel {
    /// Usage below pressure threshold — deliver at requested resolution.
    Normal,
    /// Usage between pressure threshold and critical threshold — compress
    /// responses to claim-level and attach eviction recommendations.
    Elevated,
    /// Usage at or above capacity — only deliver compressed summaries,
    /// strongly recommend eviction.
    Critical,
}

/// Snapshot of the current budget state.
///
/// Included in every tool response so the agent can make informed decisions
/// about what to keep in context and what to evict.
///
/// # Examples
///
/// ```
/// use iris_core::session::BudgetStatus;
/// use iris_core::session::PressureLevel;
///
/// let status = BudgetStatus {
///     tokens_used: 75_000,
///     tokens_remaining: 25_000,
///     pressure_level: PressureLevel::Normal,
///     utilization: 0.75,
/// };
///
/// assert_eq!(status.pressure_level, PressureLevel::Normal);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BudgetStatus {
    /// Estimated tokens currently consumed in the agent's context.
    pub tokens_used: usize,
    /// Estimated tokens remaining before hitting the budget limit.
    pub tokens_remaining: usize,
    /// Current pressure level.
    pub pressure_level: PressureLevel,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
}

/// Configuration for the budget tracker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Maximum context window budget in tokens.
    pub max_context_tokens: usize,
    /// Utilization ratio at which pressure becomes Elevated (default: 0.8).
    pub pressure_threshold: f64,
    /// Utilization ratio at which pressure becomes Critical (default: 0.95).
    pub critical_threshold: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 100_000,
            pressure_threshold: 0.80,
            critical_threshold: 0.95,
        }
    }
}

/// Tracks token budget and reports pressure levels.
///
/// Wraps a [`WindowEstimator`] and adds threshold-based pressure mode
/// transitions. The budget tracker is the primary interface for determining
/// how aggressively to compress responses.
///
/// # Examples
///
/// ```
/// use iris_core::session::{BudgetTracker, BudgetConfig, EvictionPolicy, PressureLevel};
///
/// let config = BudgetConfig {
///     max_context_tokens: 1000,
///     pressure_threshold: 0.8,
///     critical_threshold: 0.95,
/// };
/// let mut tracker = BudgetTracker::new(config, EvictionPolicy::Fifo);
///
/// tracker.record_tokens("s1", 500);
/// assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
///
/// tracker.record_tokens("s2", 400);
/// assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
/// ```
pub struct BudgetTracker {
    /// Budget configuration.
    config: BudgetConfig,
    /// Underlying window estimator.
    window: WindowEstimator,
}

impl BudgetTracker {
    /// Create a new budget tracker with the given configuration and eviction policy.
    #[must_use]
    pub fn new(config: BudgetConfig, eviction_policy: super::types::EvictionPolicy) -> Self {
        let window = WindowEstimator::new(config.max_context_tokens, eviction_policy);
        Self { config, window }
    }

    /// Record a token delivery against the budget.
    pub fn record_tokens(&mut self, content_id: &str, token_count: usize) {
        self.window.record(content_id, token_count);
    }

    /// Mark content as recently accessed (LRU policy only).
    pub fn touch(&mut self, content_id: &str) {
        self.window.touch(content_id);
    }

    /// Current pressure level based on token utilization.
    #[must_use]
    pub fn pressure_level(&self) -> PressureLevel {
        let utilization = self.utilization();
        if utilization >= self.config.critical_threshold {
            PressureLevel::Critical
        } else if utilization >= self.config.pressure_threshold {
            PressureLevel::Elevated
        } else {
            PressureLevel::Normal
        }
    }

    /// Current utilization ratio (0.0–1.0).
    #[must_use]
    pub fn utilization(&self) -> f64 {
        if self.config.max_context_tokens == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = self.window.estimated_used() as f64 / self.config.max_context_tokens as f64;
        ratio.min(1.0)
    }

    /// Get a full budget status snapshot for inclusion in tool responses.
    #[must_use]
    pub fn budget_status(&self) -> BudgetStatus {
        BudgetStatus {
            tokens_used: self.window.estimated_used(),
            tokens_remaining: self.window.estimated_remaining(),
            pressure_level: self.pressure_level(),
            utilization: self.utilization(),
        }
    }

    /// The budget configuration.
    #[must_use]
    pub fn config(&self) -> &BudgetConfig {
        &self.config
    }

    /// Access the underlying window estimator.
    #[must_use]
    pub fn window(&self) -> &WindowEstimator {
        &self.window
    }

    /// Check whether a content ID is still in the estimated window.
    #[must_use]
    pub fn is_in_window(&self, content_id: &str) -> bool {
        self.window.is_in_window(content_id)
    }

    /// Force-evict a content ID from the budget tracker's window.
    ///
    /// Used when the agent signals that content has been dropped, either
    /// explicitly (via `iris_evicted`) or implicitly (via re-request).
    /// Returns `true` if the content was found and removed.
    pub fn force_evict(&mut self, content_id: &str) -> bool {
        self.window.force_evict(content_id)
    }

    /// Get eviction candidates ranked by priority.
    ///
    /// Uses the [`EvictionRanker`] to score delivered items from the session
    /// by recency, token weight, and resolution priority. Returns up to
    /// `max_candidates` items, sorted best-to-evict first.
    ///
    /// Only returns candidates when pressure is elevated or critical.
    /// Under normal pressure, returns an empty list.
    #[must_use]
    pub fn eviction_candidates(
        &self,
        session: &Session,
        max_candidates: usize,
    ) -> Vec<EvictionCandidate> {
        if self.pressure_level() == PressureLevel::Normal {
            return Vec::new();
        }
        EvictionRanker::rank(session, max_candidates)
    }

    /// Recommend automatic tier promotions based on current pressure.
    ///
    /// Delegates to [`CompressionPipeline::recommend_promotions`] using the
    /// current pressure level. Recently accessed items (within the last 3
    /// turns) are protected from promotion under elevated pressure, but
    /// all items are eligible under critical pressure.
    ///
    /// Returns promotions sorted by estimated tokens freed (largest first).
    #[must_use]
    pub fn auto_promote(&self, session: &Session) -> Vec<super::compression::TierPromotion> {
        const RECENCY_PROTECTION_TURNS: u32 = 3;
        super::compression::CompressionPipeline::recommend_promotions(
            session,
            self.pressure_level(),
            RECENCY_PROTECTION_TURNS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::EvictionPolicy;

    fn default_tracker() -> BudgetTracker {
        BudgetTracker::new(BudgetConfig::default(), EvictionPolicy::Fifo)
    }

    fn tracker_with_capacity(capacity: usize) -> BudgetTracker {
        BudgetTracker::new(
            BudgetConfig {
                max_context_tokens: capacity,
                ..BudgetConfig::default()
            },
            EvictionPolicy::Fifo,
        )
    }

    #[test]
    fn initial_state_is_normal() {
        let tracker = default_tracker();
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
        assert!((tracker.utilization() - 0.0).abs() < f64::EPSILON);

        let status = tracker.budget_status();
        assert_eq!(status.tokens_used, 0);
        assert_eq!(status.tokens_remaining, 100_000);
        assert_eq!(status.pressure_level, PressureLevel::Normal);
    }

    #[test]
    fn normal_pressure_below_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 500);
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
        assert!((tracker.utilization() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn elevated_pressure_at_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 800);
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
        assert!((tracker.utilization() - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn elevated_pressure_between_thresholds() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 900);
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
    }

    #[test]
    fn critical_pressure_at_critical_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 950);
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
    }

    #[test]
    fn critical_pressure_at_capacity() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 1000);
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
    }

    #[test]
    fn budget_status_snapshot() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 300);
        tracker.record_tokens("s2", 200);

        let status = tracker.budget_status();
        assert_eq!(status.tokens_used, 500);
        assert_eq!(status.tokens_remaining, 500);
        assert_eq!(status.pressure_level, PressureLevel::Normal);
        assert!((status.utilization - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn pressure_transitions_with_accumulation() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 300);
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);

        tracker.record_tokens("s2", 300);
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);

        tracker.record_tokens("s3", 300);
        // 900/1000 = 0.9 -> Elevated (>= 0.8, < 0.95)
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);

        tracker.record_tokens("s4", 100);
        // 1000/1000 = 1.0 -> Critical (>= 0.95)
        // Window eviction only triggers when > capacity, and 1000 == 1000, so no eviction
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
    }

    #[test]
    fn custom_thresholds() {
        let config = BudgetConfig {
            max_context_tokens: 1000,
            pressure_threshold: 0.5,
            critical_threshold: 0.75,
        };
        let mut tracker = BudgetTracker::new(config, EvictionPolicy::Fifo);

        tracker.record_tokens("s1", 400);
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);

        tracker.record_tokens("s2", 100);
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);

        tracker.record_tokens("s3", 250);
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
    }

    #[test]
    fn zero_capacity_is_always_critical() {
        let config = BudgetConfig {
            max_context_tokens: 0,
            ..BudgetConfig::default()
        };
        let tracker = BudgetTracker::new(config, EvictionPolicy::Fifo);
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
    }

    #[test]
    fn budget_config_defaults() {
        let config = BudgetConfig::default();
        assert_eq!(config.max_context_tokens, 100_000);
        assert!((config.pressure_threshold - 0.80).abs() < f64::EPSILON);
        assert!((config.critical_threshold - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn is_in_window_delegates() {
        let mut tracker = tracker_with_capacity(1000);
        tracker.record_tokens("s1", 100);

        assert!(tracker.is_in_window("s1"));
        assert!(!tracker.is_in_window("nonexistent"));
    }

    // --- Pressure transitions with eviction ---

    #[test]
    fn pressure_drops_after_eviction() {
        // Capacity 100, pressure at 80%, critical at 95%
        let config = BudgetConfig {
            max_context_tokens: 100,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
        };
        let mut tracker = BudgetTracker::new(config, EvictionPolicy::Fifo);

        // Fill past capacity: s1=50, s2=60 -> 110 > 100, s1 evicted -> 60
        tracker.record_tokens("s1", 50);
        tracker.record_tokens("s2", 60);

        // After eviction: 60/100 = 0.6 -> Normal
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
        assert!(!tracker.is_in_window("s1"), "s1 should be evicted");
        assert!(tracker.is_in_window("s2"));
    }

    #[test]
    fn lru_eviction_with_budget_tracking() {
        let config = BudgetConfig {
            max_context_tokens: 500,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
        };
        let mut tracker = BudgetTracker::new(config, EvictionPolicy::Lru);

        tracker.record_tokens("s1", 200);
        tracker.record_tokens("s2", 200);
        // At 400/500 = 0.8 -> Elevated
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);

        // Touch s1 to make s2 the LRU candidate
        tracker.touch("s1");

        // Add s3, triggers eviction of s2 (LRU)
        tracker.record_tokens("s3", 200);
        // Would be 600 > 500, evict s2 (LRU) -> 400
        assert!(!tracker.is_in_window("s2"), "s2 should be evicted (LRU)");
        assert!(tracker.is_in_window("s1"), "s1 was touched, should survive");
        assert!(tracker.is_in_window("s3"));

        // 400/500 = 0.8 -> Elevated
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
    }

    #[test]
    fn budget_status_after_eviction() {
        let config = BudgetConfig {
            max_context_tokens: 100,
            ..BudgetConfig::default()
        };
        let mut tracker = BudgetTracker::new(config, EvictionPolicy::Fifo);

        tracker.record_tokens("s1", 30);
        tracker.record_tokens("s2", 30);
        tracker.record_tokens("s3", 30);
        // At 90/100

        // This pushes past capacity: 90 + 20 = 110 > 100, evict s1 -> 80
        tracker.record_tokens("s4", 20);

        let status = tracker.budget_status();
        assert_eq!(status.tokens_used, 80);
        assert_eq!(status.tokens_remaining, 20);
        assert_eq!(status.pressure_level, PressureLevel::Elevated); // 80/100 = 0.8
    }

    #[test]
    fn rapid_recordings_crossing_multiple_thresholds() {
        let mut tracker = tracker_with_capacity(100);

        // Record items rapidly crossing Normal -> Elevated -> Critical
        let mut levels = vec![];
        for i in 0..20 {
            tracker.record_tokens(&format!("s{i}"), 5);
            levels.push(tracker.pressure_level());
        }

        // With 20 * 5 = 100 tokens on capacity 100, eviction kicks in above 100
        // Early items should be Normal, later Elevated, final Critical
        assert!(
            levels.contains(&PressureLevel::Normal),
            "should start Normal"
        );
        assert!(
            levels.contains(&PressureLevel::Elevated),
            "should pass through Elevated"
        );
    }

    #[test]
    fn re_recording_same_content_updates_window() {
        let mut tracker = tracker_with_capacity(1000);

        tracker.record_tokens("s1", 300);
        assert_eq!(tracker.budget_status().tokens_used, 300);

        // Re-record with smaller count — replaces the old entry
        tracker.record_tokens("s1", 100);
        assert_eq!(tracker.budget_status().tokens_used, 100);
        assert!(tracker.is_in_window("s1"));
    }

    #[test]
    fn touch_nonexistent_does_not_panic() {
        let mut tracker = BudgetTracker::new(BudgetConfig::default(), EvictionPolicy::Lru);
        tracker.touch("nonexistent");
        assert_eq!(tracker.budget_status().tokens_used, 0);
    }

    #[test]
    fn utilization_capped_at_one() {
        let mut tracker = tracker_with_capacity(10);
        // Single large entry: 20 > 10, evicts itself -> 0
        tracker.record_tokens("big", 20);
        // After eviction, utilization should be 0
        assert!(tracker.utilization() <= 1.0);
    }

    #[test]
    fn window_accessor() {
        let tracker = tracker_with_capacity(1000);
        let window = tracker.window();
        assert_eq!(window.capacity(), 1000);
        assert_eq!(window.estimated_used(), 0);
    }

    #[test]
    fn config_accessor() {
        let config = BudgetConfig {
            max_context_tokens: 5000,
            pressure_threshold: 0.7,
            critical_threshold: 0.9,
        };
        let tracker = BudgetTracker::new(config.clone(), EvictionPolicy::Fifo);
        assert_eq!(tracker.config().max_context_tokens, 5000);
        assert!((tracker.config().pressure_threshold - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn pressure_level_serde_roundtrip() {
        for level in [
            PressureLevel::Normal,
            PressureLevel::Elevated,
            PressureLevel::Critical,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: PressureLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }

    #[test]
    fn budget_status_serde_roundtrip() {
        let status = BudgetStatus {
            tokens_used: 5000,
            tokens_remaining: 95_000,
            pressure_level: PressureLevel::Normal,
            utilization: 0.05,
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: BudgetStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tokens_used, status.tokens_used);
        assert_eq!(back.pressure_level, status.pressure_level);
    }

    // --- eviction_candidates tests ---

    #[test]
    fn eviction_candidates_empty_under_normal_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        tracker.record_tokens("s1", 300);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        // 300/1000 = 0.3 -> Normal pressure
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
        let candidates = tracker.eviction_candidates(&session, 5);
        assert!(
            candidates.is_empty(),
            "no eviction candidates under normal pressure"
        );
    }

    #[test]
    fn eviction_candidates_returned_under_elevated_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        tracker.record_tokens("s1", 300);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        tracker.record_tokens("s2", 600);
        session.record_delivery(
            &crate::types::ContentId::from("s2".to_string()),
            crate::types::Resolution::Section,
            600,
            2,
            "h2".into(),
        );

        // 900/1000 = 0.9 -> Elevated
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
        let candidates = tracker.eviction_candidates(&session, 5);
        assert!(
            !candidates.is_empty(),
            "should return candidates under elevated pressure"
        );
    }

    #[test]
    fn eviction_candidates_returned_under_critical_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        tracker.record_tokens("s1", 960);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            960,
            1,
            "h1".into(),
        );

        // 960/1000 = 0.96 -> Critical
        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
        let candidates = tracker.eviction_candidates(&session, 5);
        assert!(
            !candidates.is_empty(),
            "should return candidates under critical pressure"
        );
    }

    // --- auto_promote tests ---

    #[test]
    fn auto_promote_empty_under_normal_pressure() {
        let tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        // 0/1000 = 0.0 -> Normal (tracker has no tokens recorded)
        assert_eq!(tracker.pressure_level(), PressureLevel::Normal);
        let promotions = tracker.auto_promote(&session);
        assert!(promotions.is_empty(), "no promotions under normal pressure");
    }

    #[test]
    fn auto_promote_recommends_under_elevated() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        // Deliver at turn 1, then advance session to turn 10
        // so the item is old enough to be promoted (age 9 > protection 3)
        tracker.record_tokens("s1", 900);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            900,
            1,
            "h1".into(),
        );
        // Advance turn by delivering a small item at turn 10
        session.record_delivery(
            &crate::types::ContentId::from("marker".to_string()),
            crate::types::Resolution::Claim,
            10,
            10,
            "hm".into(),
        );

        // 900/1000 = 0.9 -> Elevated
        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
        let promotions = tracker.auto_promote(&session);
        // s1 is old (turn 1, age 9 > 3), should be promoted
        let s1_promo = promotions.iter().find(|p| p.content_id == "s1");
        assert!(
            s1_promo.is_some(),
            "old item should be recommended for promotion under elevated pressure"
        );
        assert_eq!(
            s1_promo.unwrap().recommended_tier,
            crate::session::CompressionTier::Extractive
        );
    }

    #[test]
    fn auto_promote_protects_recent_under_elevated() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        // Record at current turn (turn 10) — within recency protection
        tracker.record_tokens("s1", 900);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            900,
            10,
            "h1".into(),
        );

        assert_eq!(tracker.pressure_level(), PressureLevel::Elevated);
        let promotions = tracker.auto_promote(&session);
        assert!(
            promotions.is_empty(),
            "recently accessed items should be protected under elevated pressure"
        );
    }

    #[test]
    fn auto_promote_does_not_protect_under_critical() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            EvictionPolicy::Fifo,
        );

        // Recent item but critical pressure — should still promote
        tracker.record_tokens("s1", 960);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            960,
            10,
            "h1".into(),
        );

        assert_eq!(tracker.pressure_level(), PressureLevel::Critical);
        let promotions = tracker.auto_promote(&session);
        assert!(
            !promotions.is_empty(),
            "critical pressure should override recency protection"
        );
    }
}
