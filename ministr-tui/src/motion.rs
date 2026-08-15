//! Transitions inside the motion law.
//!
//! GUI-BLUEPRINT-v8 §3, extended by Amendment A: motion only where
//! something real moves — and a state change is something real moving.
//! This wrapper is the only way console code reaches tachyonfx, and it
//! enforces the law by construction: every transition is eased, runs at
//! most [`MAX_TRANSITION`], and can never loop (the looping and
//! never-completing tachyonfx constructors are simply not reachable
//! from here).

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::{Effect, EffectRenderer, EffectTimer, Interpolation, fx};

/// The motion law's hard ceiling: no transition runs longer than this.
pub const MAX_TRANSITION: Duration = Duration::from_millis(250);

/// The transitions the console may play — each one a real state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// New content materializes in place (a project patched in).
    Materialize,
    /// Content dissolves away (a project removed).
    Dissolve,
    /// A surface sweeps open left to right (a strip opened).
    SweepOpen,
}

/// One playing transition. When its time is up it is done and renders
/// nothing — there is no way to make it loop.
pub struct Motion {
    effect: Option<Effect>,
}

impl std::fmt::Debug for Motion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Motion")
            .field("running", &self.running())
            .finish()
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::none()
    }
}

impl Motion {
    /// No transition playing — the console at rest.
    #[must_use]
    pub fn none() -> Self {
        Self { effect: None }
    }

    /// Start a transition. Timing is fixed by the law: eased, and done
    /// within [`MAX_TRANSITION`].
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn start(kind: Transition) -> Self {
        let timer = EffectTimer::from_ms(MAX_TRANSITION.as_millis() as u32, Interpolation::QuadOut);
        let effect = match kind {
            Transition::Materialize => fx::coalesce(timer),
            Transition::Dissolve => fx::dissolve(timer),
            // Randomness 0 keeps the sweep deterministic, which the
            // snapshot harness depends on.
            Transition::SweepOpen => {
                fx::sweep_in(tachyonfx::Motion::LeftToRight, 8, 0, Color::Black, timer)
            }
        };
        Self {
            effect: Some(effect),
        }
    }

    /// Is the transition still playing?
    #[must_use]
    pub fn running(&self) -> bool {
        self.effect.is_some()
    }

    /// Advance the transition by `delta` and composite it over `area`.
    /// A finished transition is dropped on the spot; rendering when
    /// nothing plays is a no-op.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, delta: Duration) {
        if let Some(effect) = self.effect.as_mut() {
            frame.render_effect(effect, area, to_fx_duration(delta));
            if effect.done() {
                self.effect = None;
            }
        }
    }
}

/// std duration to tachyonfx's millisecond duration.
fn to_fx_duration(d: Duration) -> tachyonfx::Duration {
    tachyonfx::Duration::from_millis(u32::try_from(d.as_millis()).unwrap_or(u32::MAX))
}
