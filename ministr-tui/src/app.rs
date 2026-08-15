//! Application state and the frame it draws.
//!
//! The frame is S1 — The Console (GUI-BLUEPRINT-v8 §5): title row with
//! machine state top-right, one channel strip per project, the master
//! section, and a foot key-line carrying only the verbs that work.

use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::console::{self, Strip};
use crate::engine::EngineState;
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
        self.selected = self.selected.min(self.strips().len().saturating_sub(1));
        true
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

    /// Is a transition playing? While yes, the event loop paces frames
    /// at the delta-time clock instead of resting.
    #[must_use]
    pub fn animating(&self) -> bool {
        self.motion.running()
    }

    /// Draw one frame: title row, the console body, foot key-line, and
    /// whatever transition is playing, advanced by `delta`.
    pub fn draw(&mut self, frame: &mut Frame, delta: Duration) {
        let [title, body, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .horizontal_margin(2)
        .areas(frame.area());

        self.draw_title(frame, title);
        let strips = self.strips();
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
