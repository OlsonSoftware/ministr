//! Budget tracker for agent context window management.
//!
//! The [`BudgetTracker`] monitors token usage against a configurable budget
//! and reports pressure levels. When usage crosses the pressure threshold
//! (default 80%), responses should be auto-compressed to claim-level and
//! eviction recommendations should be attached.

use serde::{Deserialize, Serialize};

use super::window::WindowEstimator;

/// Pressure level indicating how close the agent is to its context budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}
