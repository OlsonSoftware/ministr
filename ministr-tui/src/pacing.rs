//! Adaptive frame pacing — event-driven at rest, a delta-time clock
//! only while something real moves.
//!
//! GUI-BLUEPRINT-v8 Amendment A: at rest the console performs zero
//! redraws (the event loop still wakes on input and probes, but no
//! frame is drawn while nothing changed); while a transition or live
//! meter runs, frames pace at roughly 60 per second.

use std::time::Duration;

/// Target interval between frames while something is animating.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Ceiling on the delta fed to effects in one frame, so a stalled
/// terminal (or the quiet gap before a transition starts) advances an
/// effect by at most a few frames' worth of time instead of jumping it
/// to its end.
pub const MAX_FRAME_DELTA: Duration = Duration::from_millis(50);

/// Decides when the console draws.
///
/// Pure state, no clock inside — the event loop owns time, which keeps
/// every pacing decision unit-testable.
#[derive(Debug)]
pub struct Pacer {
    /// State changed since the last draw; the next pass must draw once.
    dirty: bool,
    /// A transition or live meter is running; every pass draws, paced
    /// by [`FRAME_INTERVAL`].
    animating: bool,
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacer {
    /// A pacer that draws its first frame immediately.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dirty: true,
            animating: false,
        }
    }

    /// Mark that state changed — the next pass draws one frame.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Tell the pacer whether a transition or live meter is running.
    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    /// Is the delta-time clock running?
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// Should this pass draw a frame? Consumes the dirty mark; at rest
    /// with nothing marked, the answer stays no — zero redraws.
    pub fn take_redraw(&mut self) -> bool {
        let redraw = self.dirty || self.animating;
        self.dirty = false;
        redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_draws_then_rest_draws_nothing() {
        let mut pacer = Pacer::new();
        assert!(pacer.take_redraw());
        for _ in 0..100 {
            assert!(!pacer.take_redraw(), "rest state must perform zero redraws");
        }
    }

    #[test]
    fn a_state_change_draws_exactly_one_frame() {
        let mut pacer = Pacer::new();
        let _ = pacer.take_redraw();
        pacer.mark_dirty();
        assert!(pacer.take_redraw());
        assert!(!pacer.take_redraw());
    }

    #[test]
    fn animation_draws_every_pass_until_it_ends() {
        let mut pacer = Pacer::new();
        let _ = pacer.take_redraw();
        pacer.set_animating(true);
        assert!(pacer.is_animating());
        assert!(pacer.take_redraw());
        assert!(pacer.take_redraw());
        pacer.set_animating(false);
        assert!(!pacer.take_redraw());
    }

    #[test]
    fn frame_interval_is_sixty_per_second() {
        assert_eq!(FRAME_INTERVAL, Duration::from_millis(16));
        assert!(MAX_FRAME_DELTA > FRAME_INTERVAL);
    }
}
