//! Delta-time easing between progress reports — the needle glides,
//! never steps.
//!
//! GUI-BLUEPRINT-v8 Amendment A: the engine reports a build's position
//! every few hundred milliseconds; drawing those reports directly would
//! step the meter. A [`Glide`] holds the last report and eases the
//! displayed position toward it, so the needle moves continuously
//! between reports and settles gently on each one.
//!
//! The displayed position is a pure function of the glide's state and
//! `now` — no clock lives inside. The event loop owns time, and the
//! test harness passes fixed instants, so every rendered frame is
//! deterministic.

use std::time::{Duration, Instant};

/// How long the needle takes to reach a newly reported position. A
/// little longer than the report cadence, so the needle never parks
/// between reports while a build is running.
pub const GLIDE: Duration = Duration::from_millis(700);

/// One meter's eased position: where the needle was when the target
/// last moved, where it is heading, and when it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    /// The displayed position at the moment the target last changed.
    from: f64,
    /// The last reported position — where the needle is heading.
    target: f64,
    /// When the target last changed.
    since: Instant,
}

impl Glide {
    /// A needle resting at `fraction`.
    #[must_use]
    pub fn new(fraction: f64, now: Instant) -> Self {
        Self {
            from: fraction,
            target: fraction,
            since: now,
        }
    }

    /// Point the needle at a newly reported position. The glide
    /// re-bases from the position *displayed* at `now`, so the needle
    /// turns without a step no matter when the report lands. Returns
    /// whether the target actually moved.
    pub fn retarget(&mut self, target: f64, now: Instant) -> bool {
        if (target - self.target).abs() < f64::EPSILON {
            return false;
        }
        self.from = self.at(now);
        self.target = target;
        self.since = now;
        true
    }

    /// The displayed position at `now` — pure in the glide's state and
    /// the instant, easing out cubically so the needle responds at
    /// once and settles gently.
    #[must_use]
    pub fn at(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.since);
        if elapsed >= GLIDE {
            return self.target;
        }
        let t = elapsed.as_secs_f64() / GLIDE.as_secs_f64();
        let eased = 1.0 - (1.0 - t).powi(3);
        (self.target - self.from).mul_add(eased, self.from)
    }

    /// Has the needle reached its target? While no, the frame clock
    /// runs; once every glide settles the console rests again. A
    /// needle already sitting on its target is at rest from birth.
    #[must_use]
    pub fn settled(&self, now: Instant) -> bool {
        (self.target - self.from).abs() < f64::EPSILON
            || now.saturating_duration_since(self.since) >= GLIDE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_glide_rests_at_its_seed() {
        let t0 = Instant::now();
        let glide = Glide::new(0.25, t0);
        assert!((glide.at(t0) - 0.25).abs() < 1e-9);
        assert!(glide.settled(t0 + GLIDE));
        assert!((glide.at(t0 + GLIDE * 3) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn the_needle_glides_and_settles_on_the_target() {
        let t0 = Instant::now();
        let mut glide = Glide::new(0.2, t0);
        assert!(glide.retarget(0.6, t0));
        assert!((glide.at(t0) - 0.2).abs() < 1e-9, "no step at retarget");
        let mid = glide.at(t0 + GLIDE / 2);
        assert!(mid > 0.2 && mid < 0.6, "mid-glide sits between the ends");
        assert!(!glide.settled(t0 + GLIDE / 2));
        assert!((glide.at(t0 + GLIDE) - 0.6).abs() < 1e-9);
        assert!(glide.settled(t0 + GLIDE));
    }

    #[test]
    fn the_same_instant_always_shows_the_same_position() {
        let t0 = Instant::now();
        let mut glide = Glide::new(0.0, t0);
        let _ = glide.retarget(1.0, t0);
        let probe = t0 + Duration::from_millis(350);
        assert!((glide.at(probe) - glide.at(probe)).abs() < f64::EPSILON);
    }

    #[test]
    fn a_mid_glide_retarget_turns_without_a_step() {
        let t0 = Instant::now();
        let mut glide = Glide::new(0.0, t0);
        let _ = glide.retarget(0.8, t0);
        let turn = t0 + GLIDE / 2;
        let displayed = glide.at(turn);
        assert!(glide.retarget(0.3, turn));
        assert!(
            (glide.at(turn) - displayed).abs() < 1e-9,
            "the needle turns from where it is, not from where it was"
        );
    }

    #[test]
    fn an_unchanged_report_neither_moves_nor_wakes_the_needle() {
        let t0 = Instant::now();
        let mut glide = Glide::new(0.5, t0);
        let _ = glide.retarget(0.7, t0);
        let settled_at = t0 + GLIDE;
        assert!(glide.settled(settled_at));
        assert!(!glide.retarget(0.7, settled_at), "same target: no change");
        assert!(glide.settled(settled_at), "and the needle stays at rest");
    }
}
