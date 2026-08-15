//! Application state and the frame it draws.
//!
//! Scaffold only: the placeholder frame carries the title row (name left,
//! machine state right), a quiet body, and the foot key-line — the
//! console's channel strips land in the next chunk.

use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::engine::EngineState;
use crate::motion::Motion;
use crate::strings;

/// The console's whole state.
#[derive(Debug)]
pub struct App {
    /// What the last engine probe said.
    pub engine: EngineState,
    /// The transition playing over the frame, if any (motion law:
    /// one starts only on a real state change and never loops).
    pub motion: Motion,
    /// Set by [`App::on_key`]; the event loop exits on the next pass.
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// A console that has not heard from the engine yet.
    #[must_use]
    pub fn new() -> Self {
        Self::with_engine(EngineState::Starting)
    }

    /// A console with a known engine state — the snapshot harness uses
    /// this to render every state deterministically.
    #[must_use]
    pub fn with_engine(engine: EngineState) -> Self {
        Self {
            engine,
            motion: Motion::none(),
            should_quit: false,
        }
    }

    /// Handle one key press. Quit keys: `q`, Esc, Ctrl-C. Returns
    /// whether state changed (the pacer draws a frame only then).
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
        false
    }

    /// Is a transition playing? While yes, the event loop paces frames
    /// at the delta-time clock instead of resting.
    #[must_use]
    pub fn animating(&self) -> bool {
        self.motion.running()
    }

    /// Draw one frame: title row, body, foot key-line, and whatever
    /// transition is playing, advanced by `delta`.
    pub fn draw(&mut self, frame: &mut Frame, delta: Duration) {
        let [title, body, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .horizontal_margin(2)
        .areas(frame.area());

        self.draw_title(frame, title);
        self.draw_body(frame, body);
        frame.render_widget(Line::from(strings::FOOT_QUIT).dim(), foot);

        let area = frame.area();
        self.motion.render(frame, area, delta);
    }

    /// Name at the head, machine state top-right — plainly worded, and
    /// never color alone (the yellow warning carries its own words).
    fn draw_title(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Line::from(strings::APP_NAME).bold(), area);
        let state = match &self.engine {
            EngineState::Starting => Line::from(strings::ENGINE_STARTING).dim(),
            EngineState::Running { .. } => Line::from(strings::ENGINE_RUNNING),
            EngineState::Unreachable => Line::from(strings::ENGINE_UNREACHABLE).yellow(),
        };
        frame.render_widget(state.right_aligned(), area);
    }

    /// One centered, dim fact when the engine is up; otherwise the body
    /// stays still — no spinners for idle state.
    fn draw_body(&self, frame: &mut Frame, area: Rect) {
        if let EngineState::Running { projects, .. } = &self.engine {
            let [_, middle, _] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .areas(area);
            frame.render_widget(
                Line::from(strings::projects_line(*projects))
                    .dim()
                    .centered(),
                middle,
            );
        }
    }
}
