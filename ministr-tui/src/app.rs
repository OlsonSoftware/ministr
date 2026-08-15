//! Application state and the frame it draws.
//!
//! The frame is S1 — The Console (GUI-BLUEPRINT-v8 §5): title row with
//! machine state top-right, one channel strip per project, the master
//! section, and a foot key-line carrying only the verbs that work.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::console::{self, Standing, Strip};
use crate::ease::Glide;
use crate::engine::{EngineState, ProgressTarget};
use crate::motion::{Motion, Transition};
use crate::palette::ColorDepth;
use crate::strings;

/// The console's whole state.
#[derive(Debug)]
pub struct App {
    /// What the last engine probe said.
    pub engine: EngineState,
    /// The transition playing over the frame, if any (motion law:
    /// one starts only on a real state change and never loops).
    pub motion: Motion,
    /// Which strip the transition plays over, when it targets one.
    motion_strip: Option<usize>,
    /// A removed project's strip, kept in place while its dissolve
    /// plays — dropped the moment the transition completes.
    leaving: Option<(Strip, usize)>,
    /// One eased needle per building project, keyed by [`Strip::id`].
    /// Born on the first live progress report, pruned when the strip
    /// stops building. A building strip without one renders the
    /// probe's own position unchanged.
    meters: HashMap<String, Glide>,
    /// Which strip is selected (index into [`App::strips`]).
    pub selected: usize,
    /// The color ladder rung every widget renders at.
    pub depth: ColorDepth,
    /// Set by [`App::on_key`]; the event loop exits on the next pass.
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// A console that has not heard from the engine yet, reading the
    /// color ladder from the environment.
    #[must_use]
    pub fn new() -> Self {
        Self::with_engine(EngineState::Starting).with_depth(ColorDepth::from_env())
    }

    /// A console with a known engine state at the truecolor rung — the
    /// snapshot harness uses this to render every state
    /// deterministically, whatever the test environment holds.
    #[must_use]
    pub fn with_engine(engine: EngineState) -> Self {
        Self {
            engine,
            motion: Motion::none(),
            motion_strip: None,
            leaving: None,
            meters: HashMap::new(),
            selected: 0,
            depth: ColorDepth::TrueColor,
            should_quit: false,
        }
    }

    /// The same console at a different ladder rung.
    #[must_use]
    pub fn with_depth(mut self, depth: ColorDepth) -> Self {
        self.depth = depth;
        self
    }

    /// Absorb a fresh probe result. Returns whether anything changed
    /// (the pacer draws a frame only then). A project patched in
    /// materializes over its strip; a removed project's strip keeps its
    /// place and dissolves — motion only where something real moves.
    pub fn absorb(&mut self, state: EngineState) -> bool {
        if state == self.engine {
            return false;
        }
        if let (EngineState::Running(old), EngineState::Running(new)) = (&self.engine, &state) {
            let added = new
                .strips
                .iter()
                .position(|s| !old.strips.iter().any(|o| o.name == s.name));
            let removed = old
                .strips
                .iter()
                .position(|s| !new.strips.iter().any(|n| n.name == s.name));
            if let Some(index) = added {
                self.motion = Motion::start(Transition::Materialize);
                self.motion_strip = Some(index);
                self.leaving = None;
            } else if let Some(index) = removed {
                self.leaving = Some((old.strips[index].clone(), index));
                self.motion = Motion::start(Transition::Dissolve);
                self.motion_strip = Some(index);
            }
        }
        self.engine = state;
        if let EngineState::Running(model) = &self.engine {
            // A needle outlives its build by nothing: the moment a
            // strip stops building, its glide goes with it.
            self.meters.retain(|id, _| {
                model
                    .strips
                    .iter()
                    .any(|s| s.id == *id && matches!(s.standing, Standing::Building { .. }))
            });
        } else {
            self.meters.clear();
        }
        self.selected = self.selected.min(self.strips().len().saturating_sub(1));
        true
    }

    /// Absorb one round of live progress reports — the fast poll that
    /// runs only while something is building. Each report points its
    /// strip's needle at the new position; the glide from here to
    /// there is what the frames draw. Returns whether any needle
    /// actually moved (the pacer draws only then).
    pub fn absorb_progress(&mut self, targets: &[ProgressTarget], now: Instant) -> bool {
        let EngineState::Running(model) = &self.engine else {
            return false;
        };
        let mut moved = false;
        for target in targets {
            // Only a building strip has a meter to drive.
            let Some(strip) = model.strips.iter().find(|s| s.id == target.id) else {
                continue;
            };
            let Standing::Building { fraction } = strip.standing else {
                continue;
            };
            let glide = self
                .meters
                .entry(target.id.clone())
                .or_insert_with(|| Glide::new(fraction, now));
            moved |= glide.retarget(target.fraction, now);
        }
        moved
    }

    /// Is anything building? While yes, the event loop runs the fast
    /// progress poll that feeds the live meters.
    #[must_use]
    pub fn building(&self) -> bool {
        let EngineState::Running(model) = &self.engine else {
            return false;
        };
        model
            .strips
            .iter()
            .any(|s| matches!(s.standing, Standing::Building { .. }))
    }

    /// The strips the frame shows: the engine's projects, with a
    /// removed project's ghost held in place while its dissolve plays.
    #[must_use]
    pub fn strips(&self) -> Vec<Strip> {
        let EngineState::Running(model) = &self.engine else {
            return Vec::new();
        };
        let mut strips = model.strips.clone();
        if let Some((ghost, index)) = &self.leaving {
            strips.insert((*index).min(strips.len()), ghost.clone());
        }
        strips
    }

    /// Handle one key press. Left/Right move the selection; `q`, Esc,
    /// and Ctrl-C quit (Esc returns — and the console is the root).
    /// Returns whether state changed (the pacer draws only then).
    /// Enter (open the selected strip) joins with the verbs chunk.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        let ctrl_c =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c');
        if ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
            self.should_quit = true;
            return true;
        }
        match key.code {
            KeyCode::Left if self.selected > 0 => {
                self.selected -= 1;
                true
            }
            KeyCode::Right if self.selected + 1 < self.strips().len() => {
                self.selected += 1;
                true
            }
            _ => false,
        }
    }

    /// Is anything moving at `now` — a transition playing, or a needle
    /// still gliding toward its target? While yes, the event loop
    /// paces frames at the delta-time clock instead of resting; the
    /// moment every needle settles and no transition plays, the
    /// console rests again at zero redraws.
    #[must_use]
    pub fn animating(&self, now: Instant) -> bool {
        self.motion.running() || self.meters.values().any(|glide| !glide.settled(now))
    }

    /// The strips the frame draws: [`App::strips`] with each building
    /// strip's position replaced by its needle's eased position at
    /// `now` — a pure function of the glide state and the instant, so
    /// fixed-instant renders are deterministic. Both the meter and the
    /// percent under it read the same eased value.
    fn eased_strips(&self, now: Instant) -> Vec<Strip> {
        let mut strips = self.strips();
        for strip in &mut strips {
            if matches!(strip.standing, Standing::Building { .. })
                && let Some(glide) = self.meters.get(&strip.id)
            {
                strip.standing = Standing::Building {
                    fraction: glide.at(now),
                };
            }
        }
        strips
    }

    /// Draw one frame at instant `now`: title row, the console body
    /// with its meters eased to `now`, foot key-line, and whatever
    /// transition is playing, advanced by `delta`.
    pub fn draw(&mut self, frame: &mut Frame, delta: Duration, now: Instant) {
        let [title, body, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .horizontal_margin(2)
        .areas(frame.area());

        self.draw_title(frame, title);
        let strips = self.eased_strips(now);
        let layout = self.draw_body(frame, body, &strips);
        let foot_line = if strips.is_empty() {
            strings::FOOT_QUIT
        } else {
            strings::FOOT_CONSOLE
        };
        frame.render_widget(Line::from(foot_line).dim(), foot);

        let target = self
            .motion_strip
            .and_then(|i| layout.strip_rects.get(i).copied())
            .unwrap_or(body);
        self.motion.render(frame, target, delta);
        if !self.motion.running() {
            self.motion_strip = None;
            self.leaving = None;
        }
    }

    /// Name at the head, machine state top-right — plainly worded, and
    /// never color alone (the yellow warning carries its own words).
    fn draw_title(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Line::from(strings::APP_NAME).bold(), area);
        let state = match &self.engine {
            EngineState::Starting => Line::from(strings::ENGINE_STARTING).dim(),
            EngineState::Running { .. } => Line::from(strings::ENGINE_RUNNING),
            EngineState::Unreachable => {
                let line = Line::from(strings::ENGINE_UNREACHABLE);
                if self.depth == ColorDepth::Mono {
                    line
                } else {
                    line.yellow()
                }
            }
        };
        frame.render_widget(state.right_aligned(), area);
    }

    /// The console body when the engine is up; otherwise the body stays
    /// still — the title row already carries the machine state, and
    /// there are no spinners for idle state.
    fn draw_body(&self, frame: &mut Frame, area: Rect, strips: &[Strip]) -> console::ConsoleLayout {
        if let EngineState::Running(model) = &self.engine {
            console::draw(
                frame,
                area,
                strips,
                self.selected,
                self.depth,
                &model.version,
            )
        } else {
            console::ConsoleLayout::default()
        }
    }
}
