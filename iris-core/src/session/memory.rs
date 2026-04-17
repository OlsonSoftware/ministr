//! FSRS-inspired memory model for context eviction.
//!
//! Tracks per-section memory state (stability, difficulty) and predicts
//! future access probability using the FSRS retrievability formula.
//! Sections accessed frequently develop higher stability and resist eviction.
//!
//! This is a lightweight adaptation of the Free Spaced Repetition Scheduler
//! algorithm for LLM context window management — no external ML crate needed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Default initial stability (in turns). Controls how quickly a new section
/// becomes eligible for eviction after its first access.
const DEFAULT_STABILITY: f64 = 1.0;

/// Default initial difficulty. Lower = easier to retain (stability grows faster).
const DEFAULT_DIFFICULTY: f64 = 0.3;

/// Minimum stability floor to avoid division by zero in retrievability.
const MIN_STABILITY: f64 = 0.01;

/// Per-section memory state tracked across accesses.
///
/// Models how "memorable" a section is to the agent based on access patterns.
/// Higher stability means slower forgetting; lower difficulty means stability
/// grows faster on re-access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryState {
    /// Stability: higher values mean slower forgetting. Initial = 1.0.
    pub stability: f64,
    /// Difficulty: 0.0 (easy/frequently needed) to 1.0 (hard/rarely revisited).
    pub difficulty: f64,
    /// Turn number of the most recent access within the session.
    pub last_access_turn: u32,
    /// Total number of accesses within the session.
    pub access_count: u32,
}

impl MemoryState {
    /// Create a new memory state for a freshly-accessed section.
    #[must_use]
    pub fn new(turn: u32) -> Self {
        Self::new_with_rating(turn, AccessRating::Good)
    }

    /// Create a new memory state using the access rating to seed the initial
    /// stability and difficulty.
    ///
    /// * `Good` → default initial state.
    /// * `Easy` → lower initial difficulty (proactive access implies relevance).
    /// * `Again` → semantically incoherent on a first access (the item was
    ///   never delivered before). Logs a `tracing::warn!` and falls back to
    ///   the `Good` defaults so the caller isn't punished for a bad signal.
    #[must_use]
    pub fn new_with_rating(turn: u32, rating: AccessRating) -> Self {
        let (stability, difficulty) = match rating {
            AccessRating::Good => (DEFAULT_STABILITY, DEFAULT_DIFFICULTY),
            AccessRating::Easy => (DEFAULT_STABILITY, (DEFAULT_DIFFICULTY - 0.1).max(0.0)),
            AccessRating::Again => {
                tracing::warn!(
                    turn,
                    "MemoryState::new_with_rating received Again on first access — \
                     treating as Good (no prior state exists to 'forget')"
                );
                (DEFAULT_STABILITY, DEFAULT_DIFFICULTY)
            }
        };
        Self {
            stability,
            difficulty,
            last_access_turn: turn,
            access_count: 1,
        }
    }

    /// FSRS retrievability: `R(t) = (1 + t/(9*S))^(-1)`.
    ///
    /// Returns a value in `[0.0, 1.0]` where 1.0 means "certainly still relevant"
    /// and 0.0 means "likely forgotten/irrelevant".
    ///
    /// # Examples
    ///
    /// ```
    /// use iris_core::session::MemoryState;
    ///
    /// let state = MemoryState::new(0);
    /// // At turn 0, retrievability is 1.0 (just accessed)
    /// assert!((state.retrievability(0) - 1.0).abs() < f64::EPSILON);
    /// // Retrievability decays over turns
    /// assert!(state.retrievability(10) < 1.0);
    /// ```
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn retrievability(&self, current_turn: u32) -> f64 {
        let t = f64::from(current_turn.saturating_sub(self.last_access_turn));
        let s = self.stability.max(MIN_STABILITY);
        (1.0 + t / (9.0 * s)).powi(-1)
    }

    /// Update memory state after a re-access.
    ///
    /// The rating controls how stability and difficulty are adjusted:
    /// - `Again`: section was re-requested after eviction → stability halved
    /// - `Good`: normal re-access → stability grows based on difficulty
    /// - `Easy`: proactively requested → stability grows faster, difficulty decreases
    #[allow(clippy::cast_precision_loss)]
    pub fn update_on_access(&mut self, current_turn: u32, rating: AccessRating) {
        let r = self.retrievability(current_turn);

        // Difficulty update (clamped to [0.0, 1.0])
        self.difficulty = match rating {
            AccessRating::Again => (self.difficulty + 0.2).min(1.0),
            AccessRating::Good => self.difficulty,
            AccessRating::Easy => (self.difficulty - 0.1).max(0.0),
        };

        // Stability update (simplified FSRS stability growth formula)
        self.stability = match rating {
            AccessRating::Again => (self.stability * 0.5).max(MIN_STABILITY),
            AccessRating::Good | AccessRating::Easy => {
                let d_factor = (11.0 - self.difficulty * 10.0).max(1.0);
                let s_factor = self.stability.powf(-0.2);
                let r_factor = (0.05_f64 * (1.0 - r)).exp() - 1.0;
                let growth = 1.0 + 0.1_f64.exp() * d_factor * s_factor * r_factor;
                self.stability * growth.max(1.01) // ensure stability always grows
            }
        };

        self.last_access_turn = current_turn;
        self.access_count += 1;
    }
}

/// Access quality rating for memory state updates.
///
/// Simplified from FSRS's 4-point scale to 3 ratings relevant to
/// context management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRating {
    /// Section was re-requested after being evicted (it was "forgotten").
    Again,
    /// Section was re-accessed normally (still relevant).
    Good,
    /// Section proactively requested or part of an expanded read (highly relevant).
    Easy,
}

/// Tracks [`MemoryState`] for all sections accessed in a session.
///
/// Used by the eviction ranker to predict future access probability via
/// FSRS retrievability scoring.
#[derive(Debug, Default)]
pub struct MemoryTracker {
    states: HashMap<String, MemoryState>,
}

impl MemoryTracker {
    /// Create a new empty memory tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a section access, updating or creating its memory state.
    pub fn record_access(&mut self, content_id: &str, turn: u32, rating: AccessRating) {
        if let Some(state) = self.states.get_mut(content_id) {
            state.update_on_access(turn, rating);
        } else {
            self.states.insert(
                content_id.to_string(),
                MemoryState::new_with_rating(turn, rating),
            );
        }
    }

    /// Get the FSRS retrievability for a section at the given turn.
    ///
    /// Returns 0.0 for sections not yet tracked (unknown = likely irrelevant).
    #[must_use]
    pub fn retrievability(&self, content_id: &str, current_turn: u32) -> f64 {
        self.states
            .get(content_id)
            .map_or(0.0, |s| s.retrievability(current_turn))
    }

    /// Get salience-adjusted retrievability.
    ///
    /// When `salience` > 0, the effective stability is boosted before computing
    /// retrievability: `S_eff = S * (1 + 2 * salience)`. This means high-salience
    /// items forget slower without mutating the underlying memory state.
    ///
    /// Returns 0.0 for untracked sections (salience alone does not create memory).
    #[must_use]
    pub fn salience_adjusted_retrievability(
        &self,
        content_id: &str,
        current_turn: u32,
        salience: f64,
    ) -> f64 {
        self.states.get(content_id).map_or(0.0, |s| {
            if salience <= 0.0 {
                return s.retrievability(current_turn);
            }
            // Boost stability by up to 3x at full salience
            let boosted_stability = s.stability * (1.0 + 2.0 * salience);
            let t = f64::from(current_turn.saturating_sub(s.last_access_turn));
            let s_eff = boosted_stability.max(MIN_STABILITY);
            (1.0 + t / (9.0 * s_eff)).powi(-1)
        })
    }

    /// Get the memory state for a section, if tracked.
    #[must_use]
    pub fn get_state(&self, content_id: &str) -> Option<&MemoryState> {
        self.states.get(content_id)
    }

    /// All tracked states.
    #[must_use]
    pub fn states(&self) -> &HashMap<String, MemoryState> {
        &self.states
    }

    /// Export states for persistence.
    #[must_use]
    pub fn export_states(&self) -> Vec<(String, MemoryState)> {
        self.states
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Load cross-session memory, adjusting stability based on real-time decay.
    ///
    /// Stability degrades when the user hasn't interacted for a while:
    /// 1 day = no decay, 7 days = halved. This prevents stale importance
    /// scores from dominating a fresh session.
    ///
    /// Negative `hours_since_last_session` values (clock skew, NTP setbacks,
    /// VM clock freeze) are clamped to zero — time-travel is treated as
    /// "no time elapsed" rather than letting `1 / (1 + hours/168)` blow up
    /// to `+inf` or go negative.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn load_from_persisted(
        states: Vec<(String, MemoryState)>,
        hours_since_last_session: f64,
    ) -> Self {
        let hours = hours_since_last_session.max(0.0);
        let decay_factor = 1.0 / (1.0 + hours / 168.0);
        let mut tracker = Self::new();
        for (id, mut state) in states {
            state.stability *= decay_factor;
            state.stability = state.stability.max(MIN_STABILITY);
            state.last_access_turn = 0;
            tracker.states.insert(id, state);
        }
        tracker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrievability_at_zero_elapsed_is_one() {
        let state = MemoryState::new(5);
        let r = state.retrievability(5);
        assert!((r - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn retrievability_decays_with_distance() {
        let state = MemoryState::new(0);
        let r1 = state.retrievability(1);
        let r5 = state.retrievability(5);
        let r20 = state.retrievability(20);
        assert!(r1 > r5, "closer turn should have higher R");
        assert!(r5 > r20, "R should decay monotonically");
        assert!(r20 > 0.0, "R should never reach exactly zero");
    }

    #[test]
    fn high_stability_decays_slower() {
        let low_s = MemoryState {
            stability: 1.0,
            difficulty: 0.3,
            last_access_turn: 0,
            access_count: 1,
        };
        let high_s = MemoryState {
            stability: 10.0,
            difficulty: 0.3,
            last_access_turn: 0,
            access_count: 1,
        };
        let r_low = low_s.retrievability(10);
        let r_high = high_s.retrievability(10);
        assert!(
            r_high > r_low,
            "higher stability should mean slower decay: high={r_high}, low={r_low}"
        );
    }

    #[test]
    fn update_on_good_increases_stability() {
        let mut state = MemoryState::new(0);
        let s_before = state.stability;
        state.update_on_access(5, AccessRating::Good);
        assert!(
            state.stability > s_before,
            "Good rating should increase stability"
        );
    }

    #[test]
    fn update_on_again_halves_stability() {
        let mut state = MemoryState {
            stability: 4.0,
            difficulty: 0.3,
            last_access_turn: 0,
            access_count: 1,
        };
        state.update_on_access(5, AccessRating::Again);
        assert!(
            (state.stability - 2.0).abs() < f64::EPSILON,
            "Again should halve stability: got {}",
            state.stability
        );
    }

    #[test]
    fn difficulty_clamps_to_bounds() {
        let mut low = MemoryState {
            stability: 1.0,
            difficulty: 0.05,
            last_access_turn: 0,
            access_count: 1,
        };
        low.update_on_access(1, AccessRating::Easy);
        assert!(low.difficulty >= 0.0, "difficulty should not go below 0");

        let mut high = MemoryState {
            stability: 1.0,
            difficulty: 0.95,
            last_access_turn: 0,
            access_count: 1,
        };
        high.update_on_access(1, AccessRating::Again);
        assert!(high.difficulty <= 1.0, "difficulty should not exceed 1.0");
    }

    #[test]
    fn tracker_records_new_and_existing() {
        let mut tracker = MemoryTracker::new();
        tracker.record_access("s1", 1, AccessRating::Good);
        assert_eq!(tracker.get_state("s1").unwrap().access_count, 1);

        tracker.record_access("s1", 5, AccessRating::Good);
        assert_eq!(tracker.get_state("s1").unwrap().access_count, 2);
    }

    #[test]
    fn tracker_retrievability_unknown_is_zero() {
        let tracker = MemoryTracker::new();
        assert!((tracker.retrievability("unknown", 10) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tracker_frequently_accessed_has_higher_retrievability() {
        let mut tracker = MemoryTracker::new();

        // Section A: accessed once at turn 0
        tracker.record_access("a", 0, AccessRating::Good);

        // Section B: accessed 5 times (building stability)
        tracker.record_access("b", 0, AccessRating::Good);
        tracker.record_access("b", 2, AccessRating::Good);
        tracker.record_access("b", 4, AccessRating::Good);
        tracker.record_access("b", 6, AccessRating::Good);
        tracker.record_access("b", 8, AccessRating::Good);

        let r_a = tracker.retrievability("a", 15);
        let r_b = tracker.retrievability("b", 15);

        assert!(
            r_b > r_a,
            "frequently accessed section should have higher R: a={r_a}, b={r_b}"
        );
    }

    #[test]
    fn load_from_persisted_applies_wall_clock_decay() {
        let states = vec![(
            "s1".to_string(),
            MemoryState {
                stability: 10.0,
                difficulty: 0.3,
                last_access_turn: 50,
                access_count: 10,
            },
        )];

        // 7 days (168 hours) → stability halved
        let tracker = MemoryTracker::load_from_persisted(states, 168.0);
        let state = tracker.get_state("s1").unwrap();
        assert!(
            (state.stability - 5.0).abs() < 0.01,
            "168 hours should halve stability: got {}",
            state.stability
        );
        assert_eq!(state.last_access_turn, 0, "turn should reset to 0");
    }

    // --- Salience-adjusted retrievability tests ---

    #[test]
    fn salience_adjusted_equals_plain_at_zero_salience() {
        let mut tracker = MemoryTracker::new();
        tracker.record_access("s1", 0, AccessRating::Good);

        let plain = tracker.retrievability("s1", 10);
        let adjusted = tracker.salience_adjusted_retrievability("s1", 10, 0.0);
        assert!(
            (plain - adjusted).abs() < f64::EPSILON,
            "zero salience should match plain: {plain} vs {adjusted}"
        );
    }

    #[test]
    fn salience_adjusted_boosts_retrievability() {
        let mut tracker = MemoryTracker::new();
        tracker.record_access("s1", 0, AccessRating::Good);

        let plain = tracker.retrievability("s1", 10);
        let boosted = tracker.salience_adjusted_retrievability("s1", 10, 1.0);
        assert!(
            boosted > plain,
            "full salience should boost R: plain={plain}, boosted={boosted}"
        );
    }

    #[test]
    fn salience_adjusted_returns_zero_for_unknown() {
        let tracker = MemoryTracker::new();
        let r = tracker.salience_adjusted_retrievability("unknown", 10, 1.0);
        assert!(
            (r - 0.0).abs() < f64::EPSILON,
            "untracked section should return 0 even with salience"
        );
    }

    #[test]
    fn salience_boost_is_proportional() {
        let mut tracker = MemoryTracker::new();
        tracker.record_access("s1", 0, AccessRating::Good);

        let low = tracker.salience_adjusted_retrievability("s1", 10, 0.2);
        let mid = tracker.salience_adjusted_retrievability("s1", 10, 0.5);
        let high = tracker.salience_adjusted_retrievability("s1", 10, 1.0);

        assert!(
            high > mid && mid > low,
            "boost should be proportional: low={low}, mid={mid}, high={high}"
        );
    }
}
