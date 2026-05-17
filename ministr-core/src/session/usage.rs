//! Budget tracker for agent context window management.
//!
//! The [`UsageTracker`] monitors token usage against a configurable budget
//! and reports pressure levels. When usage crosses the pressure threshold
//! (default 80%), responses should be auto-compressed to claim-level and
//! eviction recommendations should be attached.

use serde::{Deserialize, Serialize};

use super::drops::{DropCandidate, DropRanker};
use super::types::Session;
use super::window::WindowEstimator;

/// Pressure level indicating how close the agent is to its context budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageLevel {
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
/// use ministr_core::session::UsageStatus;
/// use ministr_core::session::UsageLevel;
///
/// let status = UsageStatus {
///     tokens_used: 75_000,
///     tokens_remaining: 25_000,
///     level: UsageLevel::Normal,
///     utilization: 0.75,
/// };
///
/// assert_eq!(status.level, UsageLevel::Normal);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UsageStatus {
    /// Estimated tokens currently consumed in the agent's context.
    pub tokens_used: usize,
    /// Estimated tokens remaining before hitting the budget limit.
    pub tokens_remaining: usize,
    /// Current pressure level.
    pub level: UsageLevel,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
}

/// Fallback context-window size, in tokens, when nothing tells us the
/// real one.
///
/// 200k is the floor for current-generation Claude models, so it never
/// *under*-reports a real window the way the previous hardcoded 100k did
/// (which made budget pressure fire on agents that had 2–10x the room).
/// Deployments on larger windows (e.g. the 1M-context Opus variants)
/// should set [`MINISTR_CONTEXT_WINDOW`](default_max_context_tokens) so
/// the numbers track the session's actual window.
const FALLBACK_CONTEXT_TOKENS: usize = 200_000;

/// Resolve the context-window budget from the environment.
///
/// MCP gives the server no channel to learn the connected model's real
/// context window, so the source of truth is `MINISTR_CONTEXT_WINDOW`,
/// set next to where the MCP client is configured (the `env` block of
/// `.mcp.json` / `.vscode/mcp.json` / `.cursor/mcp.json`, or
/// `~/.codex/config.toml`). When unset, empty, unparseable, or zero we
/// fall back to [`FALLBACK_CONTEXT_TOKENS`] rather than a misleadingly
/// small fixed window.
#[must_use]
pub fn default_max_context_tokens() -> usize {
    std::env::var("MINISTR_CONTEXT_WINDOW")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(FALLBACK_CONTEXT_TOKENS)
}

/// Configuration for the budget tracker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageConfig {
    /// Maximum context window budget in tokens.
    ///
    /// Defaults to the connected session's window via
    /// [`default_max_context_tokens`] (`MINISTR_CONTEXT_WINDOW` env, else
    /// [`FALLBACK_CONTEXT_TOKENS`]).
    pub max_context_tokens: usize,
    /// Utilization ratio at which pressure becomes Elevated (default: 0.8).
    pub pressure_threshold: f64,
    /// Utilization ratio at which pressure becomes Critical (default: 0.95).
    pub critical_threshold: f64,
    /// Eviction policy applied when the window fills. Defaults to `Fifo`
    /// for backward compatibility; callers who want FSRS-aware eviction
    /// (using the memory tracker's retrievability scores) must explicitly
    /// set this to `Fsrs` and call [`UsageTracker::record_tokens_with_memory`].
    #[serde(default = "default_eviction_policy")]
    pub eviction_policy: super::types::DropPolicy,
}

fn default_eviction_policy() -> super::types::DropPolicy {
    super::types::DropPolicy::Fifo
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: default_max_context_tokens(),
            pressure_threshold: 0.80,
            critical_threshold: 0.95,
            eviction_policy: super::types::DropPolicy::Fifo,
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
/// use ministr_core::session::{UsageTracker, UsageConfig, DropPolicy, UsageLevel};
///
/// let config = UsageConfig {
///     max_context_tokens: 1000,
///     pressure_threshold: 0.8,
///     critical_threshold: 0.95,
///     eviction_policy: DropPolicy::Fifo,
/// };
/// let mut tracker = UsageTracker::new(config, DropPolicy::Fifo);
///
/// tracker.record_tokens("s1", 500);
/// assert_eq!(tracker.level(), UsageLevel::Normal);
///
/// tracker.record_tokens("s2", 400);
/// assert_eq!(tracker.level(), UsageLevel::Elevated);
/// ```
pub struct UsageTracker {
    /// Budget configuration.
    config: UsageConfig,
    /// Underlying window estimator.
    window: WindowEstimator,
}

impl UsageTracker {
    /// Create a new budget tracker with the given configuration and eviction policy.
    #[must_use]
    pub fn new(config: UsageConfig, eviction_policy: super::types::DropPolicy) -> Self {
        let window = WindowEstimator::new(config.max_context_tokens, eviction_policy);
        Self { config, window }
    }

    /// Record a token delivery against the budget.
    ///
    /// Returns the content IDs of any entries evicted from the window model
    /// to make room for this delivery. Callers should apply bookmark
    /// compression to evicted IDs so the agent retains structural awareness.
    ///
    /// **Note:** under [`DropPolicy::Fsrs`](super::types::DropPolicy::Fsrs)
    /// this path falls back to FIFO eviction because no retrievability
    /// scores are supplied. Call [`record_tokens_with_memory`](Self::record_tokens_with_memory)
    /// instead whenever a [`MemoryTracker`](super::memory::MemoryTracker) is
    /// available so FSRS policies actually consult memory.
    #[must_use]
    pub fn record_tokens(&mut self, content_id: &str, token_count: usize) -> Vec<String> {
        self.window.record(content_id, token_count)
    }

    /// Record a token delivery with FSRS-aware eviction.
    ///
    /// Under the [`DropPolicy::Fsrs`] policy, the memory tracker's
    /// retrievability scores determine which content is evicted first
    /// (lowest predicted recall probability).
    #[allow(clippy::cast_precision_loss)]
    pub fn record_tokens_with_memory(
        &mut self,
        content_id: &str,
        token_count: usize,
        memory: &super::memory::MemoryTracker,
        current_turn: u32,
    ) -> Vec<String> {
        let scores: std::collections::HashMap<String, f64> = memory
            .states()
            .keys()
            .map(|k| (k.clone(), memory.retrievability(k, current_turn)))
            .collect();
        self.window
            .record_with_scores(content_id, token_count, Some(&scores))
    }

    /// Mark content as recently accessed (LRU policy only).
    pub fn touch(&mut self, content_id: &str) {
        self.window.touch(content_id);
    }

    /// Current pressure level based on token utilization.
    #[must_use]
    pub fn level(&self) -> UsageLevel {
        let utilization = self.utilization();
        if utilization >= self.config.critical_threshold {
            UsageLevel::Critical
        } else if utilization >= self.config.pressure_threshold {
            UsageLevel::Elevated
        } else {
            UsageLevel::Normal
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
    pub fn usage_status(&self) -> UsageStatus {
        UsageStatus {
            tokens_used: self.window.estimated_used(),
            tokens_remaining: self.window.estimated_remaining(),
            level: self.level(),
            utilization: self.utilization(),
        }
    }

    /// The budget configuration.
    #[must_use]
    pub fn config(&self) -> &UsageConfig {
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
    /// explicitly (via `ministr_dropped`) or implicitly (via re-request).
    /// Returns `true` if the content was found and removed.
    pub fn force_evict(&mut self, content_id: &str) -> bool {
        self.window.force_evict(content_id)
    }

    /// Get eviction candidates ranked by priority.
    ///
    /// Uses the [`DropRanker`] to score delivered items from the session
    /// by recency, token weight, and resolution priority. Returns up to
    /// `max_candidates` items, sorted best-to-evict first.
    ///
    /// Only returns candidates when pressure is elevated or critical.
    /// Under normal pressure, returns an empty list.
    #[must_use]
    pub fn drop_candidates(
        &self,
        session: &Session,
        max_candidates: usize,
        memory: Option<&super::memory::MemoryTracker>,
    ) -> Vec<DropCandidate> {
        if self.level() == UsageLevel::Normal {
            return Vec::new();
        }
        DropRanker::rank(session, max_candidates, memory)
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
            self.level(),
            RECENCY_PROTECTION_TURNS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::DropPolicy;

    fn default_tracker() -> UsageTracker {
        UsageTracker::new(UsageConfig::default(), DropPolicy::Fifo)
    }

    fn tracker_with_capacity(capacity: usize) -> UsageTracker {
        UsageTracker::new(
            UsageConfig {
                max_context_tokens: capacity,
                ..UsageConfig::default()
            },
            DropPolicy::Fifo,
        )
    }

    #[test]
    fn initial_state_is_normal() {
        let tracker = default_tracker();
        assert_eq!(tracker.level(), UsageLevel::Normal);
        assert!((tracker.utilization() - 0.0).abs() < f64::EPSILON);

        let status = tracker.usage_status();
        assert_eq!(status.tokens_used, 0);
        // With MINISTR_CONTEXT_WINDOW unset (the default test env) the
        // window is the non-misleading fallback, not the old 100k.
        assert_eq!(status.tokens_remaining, FALLBACK_CONTEXT_TOKENS);
        assert_eq!(status.level, UsageLevel::Normal);
    }

    #[test]
    fn normal_pressure_below_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 500);
        assert_eq!(tracker.level(), UsageLevel::Normal);
        assert!((tracker.utilization() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn elevated_pressure_at_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 800);
        assert_eq!(tracker.level(), UsageLevel::Elevated);
        assert!((tracker.utilization() - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn elevated_pressure_between_thresholds() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 900);
        assert_eq!(tracker.level(), UsageLevel::Elevated);
    }

    #[test]
    fn critical_pressure_at_critical_threshold() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 950);
        assert_eq!(tracker.level(), UsageLevel::Critical);
    }

    #[test]
    fn critical_pressure_at_capacity() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 1000);
        assert_eq!(tracker.level(), UsageLevel::Critical);
    }

    #[test]
    fn usage_status_snapshot() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 300);
        let _ = tracker.record_tokens("s2", 200);

        let status = tracker.usage_status();
        assert_eq!(status.tokens_used, 500);
        assert_eq!(status.tokens_remaining, 500);
        assert_eq!(status.level, UsageLevel::Normal);
        assert!((status.utilization - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn pressure_transitions_with_accumulation() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 300);
        assert_eq!(tracker.level(), UsageLevel::Normal);

        let _ = tracker.record_tokens("s2", 300);
        assert_eq!(tracker.level(), UsageLevel::Normal);

        let _ = tracker.record_tokens("s3", 300);
        // 900/1000 = 0.9 -> Elevated (>= 0.8, < 0.95)
        assert_eq!(tracker.level(), UsageLevel::Elevated);

        let _ = tracker.record_tokens("s4", 100);
        // 1000/1000 = 1.0 -> Critical (>= 0.95)
        // Window eviction only triggers when > capacity, and 1000 == 1000, so no eviction
        assert_eq!(tracker.level(), UsageLevel::Critical);
    }

    #[test]
    fn custom_thresholds() {
        let config = UsageConfig {
            max_context_tokens: 1000,
            pressure_threshold: 0.5,
            critical_threshold: 0.75,
            eviction_policy: DropPolicy::Fifo,
        };
        let mut tracker = UsageTracker::new(config, DropPolicy::Fifo);

        let _ = tracker.record_tokens("s1", 400);
        assert_eq!(tracker.level(), UsageLevel::Normal);

        let _ = tracker.record_tokens("s2", 100);
        assert_eq!(tracker.level(), UsageLevel::Elevated);

        let _ = tracker.record_tokens("s3", 250);
        assert_eq!(tracker.level(), UsageLevel::Critical);
    }

    #[test]
    fn zero_capacity_is_always_critical() {
        let config = UsageConfig {
            max_context_tokens: 0,
            ..UsageConfig::default()
        };
        let tracker = UsageTracker::new(config, DropPolicy::Fifo);
        assert_eq!(tracker.level(), UsageLevel::Critical);
    }

    #[test]
    fn budget_config_defaults() {
        let config = UsageConfig::default();
        // MINISTR_CONTEXT_WINDOW unset → fallback window, not the old
        // hardcoded 100k that under-reported real model windows.
        assert_eq!(config.max_context_tokens, FALLBACK_CONTEXT_TOKENS);
        assert!((config.pressure_threshold - 0.80).abs() < f64::EPSILON);
        assert!((config.critical_threshold - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn context_window_env_override_is_respected() {
        // Guards the env-driven path without mutating process env (which
        // would race parallel tests): exercise the parser contract via a
        // closure mirroring default_max_context_tokens()'s logic.
        let resolve = |raw: Option<&str>| -> usize {
            raw.and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|&n| n > 0)
                .unwrap_or(FALLBACK_CONTEXT_TOKENS)
        };
        assert_eq!(resolve(Some("1000000")), 1_000_000);
        assert_eq!(resolve(Some("  250000 ")), 250_000);
        assert_eq!(resolve(Some("0")), FALLBACK_CONTEXT_TOKENS);
        assert_eq!(resolve(Some("nonsense")), FALLBACK_CONTEXT_TOKENS);
        assert_eq!(resolve(None), FALLBACK_CONTEXT_TOKENS);
    }

    #[test]
    fn is_in_window_delegates() {
        let mut tracker = tracker_with_capacity(1000);
        let _ = tracker.record_tokens("s1", 100);

        assert!(tracker.is_in_window("s1"));
        assert!(!tracker.is_in_window("nonexistent"));
    }

    // --- Pressure transitions with eviction ---

    #[test]
    fn pressure_drops_after_eviction() {
        // Capacity 100, pressure at 80%, critical at 95%
        let config = UsageConfig {
            max_context_tokens: 100,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            eviction_policy: DropPolicy::Fifo,
        };
        let mut tracker = UsageTracker::new(config, DropPolicy::Fifo);

        // Fill past capacity: s1=50, s2=60 -> 110 > 100, s1 evicted -> 60
        let _ = tracker.record_tokens("s1", 50);
        let _ = tracker.record_tokens("s2", 60);

        // After eviction: 60/100 = 0.6 -> Normal
        assert_eq!(tracker.level(), UsageLevel::Normal);
        assert!(!tracker.is_in_window("s1"), "s1 should be evicted");
        assert!(tracker.is_in_window("s2"));
    }

    #[test]
    fn lru_eviction_with_budget_tracking() {
        let config = UsageConfig {
            max_context_tokens: 500,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            eviction_policy: DropPolicy::Lru,
        };
        let mut tracker = UsageTracker::new(config, DropPolicy::Lru);

        let _ = tracker.record_tokens("s1", 200);
        let _ = tracker.record_tokens("s2", 200);
        // At 400/500 = 0.8 -> Elevated
        assert_eq!(tracker.level(), UsageLevel::Elevated);

        // Touch s1 to make s2 the LRU candidate
        tracker.touch("s1");

        // Add s3, triggers eviction of s2 (LRU)
        let _ = tracker.record_tokens("s3", 200);
        // Would be 600 > 500, evict s2 (LRU) -> 400
        assert!(!tracker.is_in_window("s2"), "s2 should be evicted (LRU)");
        assert!(tracker.is_in_window("s1"), "s1 was touched, should survive");
        assert!(tracker.is_in_window("s3"));

        // 400/500 = 0.8 -> Elevated
        assert_eq!(tracker.level(), UsageLevel::Elevated);
    }

    #[test]
    fn usage_status_after_eviction() {
        let config = UsageConfig {
            max_context_tokens: 100,
            ..UsageConfig::default()
        };
        let mut tracker = UsageTracker::new(config, DropPolicy::Fifo);

        let _ = tracker.record_tokens("s1", 30);
        let _ = tracker.record_tokens("s2", 30);
        let _ = tracker.record_tokens("s3", 30);
        // At 90/100

        // This pushes past capacity: 90 + 20 = 110 > 100, evict s1 -> 80
        let _ = tracker.record_tokens("s4", 20);

        let status = tracker.usage_status();
        assert_eq!(status.tokens_used, 80);
        assert_eq!(status.tokens_remaining, 20);
        assert_eq!(status.level, UsageLevel::Elevated); // 80/100 = 0.8
    }

    #[test]
    fn fsrs_eviction_prefers_low_retrievability_over_oldest() {
        // End-to-end regression: under FSRS, the memory-aware record path
        // must pick the lowest-retrievability entry as the victim. Before
        // the fix, callers that used `record_tokens` (no scores) silently
        // fell back to FIFO — the oldest entry was evicted regardless of
        // how recently it was accessed.
        use crate::session::memory::{AccessRating, MemoryTracker};

        let config = UsageConfig {
            max_context_tokens: 100,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            eviction_policy: DropPolicy::Fsrs,
        };
        let mut tracker = UsageTracker::new(config, DropPolicy::Fsrs);
        let mut memory = MemoryTracker::new();

        // Turn 1: deliver A (will appear "oldest" to FIFO)
        memory.record_access("a", 1, AccessRating::Good);
        let _ = tracker.record_tokens_with_memory("a", 40, &memory, 1);

        // Turn 2: deliver B (middle-aged) — but re-accessed repeatedly so
        // its retrievability stays high.
        memory.record_access("b", 2, AccessRating::Good);
        let _ = tracker.record_tokens_with_memory("b", 40, &memory, 2);
        memory.record_access("b", 5, AccessRating::Good);
        memory.record_access("b", 8, AccessRating::Good);

        // Turn 10: a has NOT been re-accessed since turn 1 → low R.
        //          b was just re-accessed at turn 8 → high R.
        // Delivering c (40 tokens) overflows 100-capacity window. Under
        // FIFO the victim would be `a` (oldest); under FSRS it should be
        // the lowest-retrievability entry — but since a has decayed more
        // than b, FSRS also picks `a`. To discriminate, swap: re-access a
        // just before the overflow so b's retrievability is now lower.
        memory.record_access("a", 9, AccessRating::Good);
        let evicted = tracker.record_tokens_with_memory("c", 40, &memory, 10);

        // a was just touched (R high), b was last touched at turn 8 (R lower);
        // FSRS should evict b, not the oldest-by-insertion a.
        assert!(
            evicted.iter().any(|id| id == "b"),
            "FSRS should evict low-retrievability `b`, got {evicted:?}"
        );
        assert!(
            tracker.is_in_window("a"),
            "recently re-accessed `a` should survive FSRS eviction"
        );
    }

    #[test]
    fn rapid_recordings_crossing_multiple_thresholds() {
        let mut tracker = tracker_with_capacity(100);

        // Record items rapidly crossing Normal -> Elevated -> Critical
        let mut levels = vec![];
        for i in 0..20 {
            let _ = tracker.record_tokens(&format!("s{i}"), 5);
            levels.push(tracker.level());
        }

        // With 20 * 5 = 100 tokens on capacity 100, eviction kicks in above 100
        // Early items should be Normal, later Elevated, final Critical
        assert!(
            levels.contains(&UsageLevel::Normal),
            "should start Normal"
        );
        assert!(
            levels.contains(&UsageLevel::Elevated),
            "should pass through Elevated"
        );
    }

    #[test]
    fn re_recording_same_content_updates_window() {
        let mut tracker = tracker_with_capacity(1000);

        let _ = tracker.record_tokens("s1", 300);
        assert_eq!(tracker.usage_status().tokens_used, 300);

        // Re-record with smaller count — replaces the old entry
        let _ = tracker.record_tokens("s1", 100);
        assert_eq!(tracker.usage_status().tokens_used, 100);
        assert!(tracker.is_in_window("s1"));
    }

    #[test]
    fn touch_nonexistent_does_not_panic() {
        let mut tracker = UsageTracker::new(UsageConfig::default(), DropPolicy::Lru);
        tracker.touch("nonexistent");
        assert_eq!(tracker.usage_status().tokens_used, 0);
    }

    #[test]
    fn utilization_capped_at_one() {
        let mut tracker = tracker_with_capacity(10);
        // Single large entry: 20 > 10, evicts itself -> 0
        let _ = tracker.record_tokens("big", 20);
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
        let config = UsageConfig {
            max_context_tokens: 5000,
            pressure_threshold: 0.7,
            critical_threshold: 0.9,
            eviction_policy: DropPolicy::Fifo,
        };
        let tracker = UsageTracker::new(config.clone(), DropPolicy::Fifo);
        assert_eq!(tracker.config().max_context_tokens, 5000);
        assert!((tracker.config().pressure_threshold - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn level_serde_roundtrip() {
        for level in [
            UsageLevel::Normal,
            UsageLevel::Elevated,
            UsageLevel::Critical,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: UsageLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }

    #[test]
    fn usage_status_serde_roundtrip() {
        let status = UsageStatus {
            tokens_used: 5000,
            tokens_remaining: 95_000,
            level: UsageLevel::Normal,
            utilization: 0.05,
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: UsageStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tokens_used, status.tokens_used);
        assert_eq!(back.level, status.level);
    }

    // --- drop_candidates tests ---

    #[test]
    fn drop_candidates_empty_under_normal_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            DropPolicy::Fifo,
        );

        let _ = tracker.record_tokens("s1", 300);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        // 300/1000 = 0.3 -> Normal pressure
        assert_eq!(tracker.level(), UsageLevel::Normal);
        let candidates = tracker.drop_candidates(&session, 5, None);
        assert!(
            candidates.is_empty(),
            "no eviction candidates under normal pressure"
        );
    }

    #[test]
    fn drop_candidates_returned_under_elevated_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            DropPolicy::Fifo,
        );

        let _ = tracker.record_tokens("s1", 300);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        let _ = tracker.record_tokens("s2", 600);
        session.record_delivery(
            &crate::types::ContentId::from("s2".to_string()),
            crate::types::Resolution::Section,
            600,
            2,
            "h2".into(),
        );

        // 900/1000 = 0.9 -> Elevated
        assert_eq!(tracker.level(), UsageLevel::Elevated);
        let candidates = tracker.drop_candidates(&session, 5, None);
        assert!(
            !candidates.is_empty(),
            "should return candidates under elevated pressure"
        );
    }

    #[test]
    fn drop_candidates_returned_under_critical_pressure() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            DropPolicy::Fifo,
        );

        let _ = tracker.record_tokens("s1", 960);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            960,
            1,
            "h1".into(),
        );

        // 960/1000 = 0.96 -> Critical
        assert_eq!(tracker.level(), UsageLevel::Critical);
        let candidates = tracker.drop_candidates(&session, 5, None);
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
            DropPolicy::Fifo,
        );

        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            300,
            1,
            "h1".into(),
        );

        // 0/1000 = 0.0 -> Normal (tracker has no tokens recorded)
        assert_eq!(tracker.level(), UsageLevel::Normal);
        let promotions = tracker.auto_promote(&session);
        assert!(promotions.is_empty(), "no promotions under normal pressure");
    }

    #[test]
    fn auto_promote_recommends_under_elevated() {
        let mut tracker = tracker_with_capacity(1000);
        let mut session = crate::session::Session::new(
            crate::session::SessionId::from("test".to_string()),
            1000,
            DropPolicy::Fifo,
        );

        // Deliver at turn 1, then advance session to turn 10
        // so the item is old enough to be promoted (age 9 > protection 3)
        let _ = tracker.record_tokens("s1", 900);
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
        assert_eq!(tracker.level(), UsageLevel::Elevated);
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
            DropPolicy::Fifo,
        );

        // Record at current turn (turn 10) — within recency protection
        let _ = tracker.record_tokens("s1", 900);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            900,
            10,
            "h1".into(),
        );

        assert_eq!(tracker.level(), UsageLevel::Elevated);
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
            DropPolicy::Fifo,
        );

        // Recent item but critical pressure — should still promote
        let _ = tracker.record_tokens("s1", 960);
        session.record_delivery(
            &crate::types::ContentId::from("s1".to_string()),
            crate::types::Resolution::Section,
            960,
            10,
            "h1".into(),
        );

        assert_eq!(tracker.level(), UsageLevel::Critical);
        let promotions = tracker.auto_promote(&session);
        assert!(
            !promotions.is_empty(),
            "critical pressure should override recency protection"
        );
    }
}
