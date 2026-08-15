//! Application state and the frame it draws.
//!
//! The frame is GUI-BLUEPRINT-v8 §5: title row with machine state
//! top-right, the current view's body — S1 The Console, S2 an opened
//! project, or S3 the patch-in panel — and a foot key-line carrying
//! only the verbs that work in that view.
//!
//! Verbs never touch the engine from here: a key press queues one
//! [`Action`] in [`App::pending`] and the event loop drains it against
//! the client, the same seam [`App::should_quit`] uses. That keeps the
//! whole state machine synchronous and snapshot-constructible.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::console::{self, Standing, Strip};
use crate::detail::{self, Detail, Facts, PathsEditor};
use crate::ease::Glide;
use crate::engine::{Action, EngineState, FreshSig, Outcome, ProgressTarget};
use crate::lawn::{self, Lawn};
use crate::motion::{Motion, Transition};
use crate::palette::ColorDepth;
use crate::patchin::{self, PatchIn};
use crate::strings;

/// Which screen the body shows.
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    /// S1 — the console.
    Console,
    /// S2 — one project, opened.
    Detail(Detail),
    /// S3 — the patch-in panel over the console.
    PatchIn(PatchIn),
}

/// The console's whole state.
#[derive(Debug)]
pub struct App {
    /// What the last engine probe said.
    pub engine: EngineState,
    /// Which screen the body shows.
    pub view: View,
    /// The verb the event loop should run against the engine, queued
    /// by [`App::on_key`] and drained once per pass.
    pub pending: Option<Action>,
    /// Is the inline remove question up, on the selected strip or the
    /// opened project?
    pub confirming_remove: bool,
    /// A verb's plain-worded failure, shown on the foot row until the
    /// next key press.
    pub notice: Option<String>,
    /// The verb running on the engine right now, as the plain word the
    /// foot row shows — verbs run on a background task so a slow
    /// answer never freezes a frame. Cleared when the outcome lands.
    pub working: Option<&'static str>,
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
    /// Each project's lawn, keyed by [`Strip::id`] — filled by
    /// background fetches the event loop runs when a strip's
    /// freshness signature changes.
    lawns: HashMap<String, Lawn>,
    /// The freshness signature each cached lawn was fetched at.
    lawn_sigs: HashMap<String, FreshSig>,
    /// The lawn's processing pulse per building project: the file the
    /// indexer last reported and when the report landed. Pruned with
    /// the meters when a strip stops building.
    pulses: HashMap<String, (String, Instant)>,
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
            view: View::Console,
            pending: None,
            confirming_remove: false,
            notice: None,
            working: None,
            motion: Motion::none(),
            motion_strip: None,
            leaving: None,
            meters: HashMap::new(),
            lawns: HashMap::new(),
            lawn_sigs: HashMap::new(),
            pulses: HashMap::new(),
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
            // strip stops building, its glide goes with it — and so
            // does the lawn's marching pulse.
            self.meters.retain(|id, _| {
                model
                    .strips
                    .iter()
                    .any(|s| s.id == *id && matches!(s.standing, Standing::Building { .. }))
            });
            // A pulse outlives its project by nothing; its 250ms decay
            // handles the end of a build on its own.
            self.pulses
                .retain(|id, _| model.strips.iter().any(|s| s.id == *id));
            // Lawns are deliberately NOT pruned when a strip vanishes:
            // a rebuild unregisters the project for a moment, and a
            // probe landing in that window would drop the cached lawn
            // exactly when the returning build's march needs it. The
            // cache is bounded by the projects seen this session.
            // An opened project tracks its strip; if the strip left
            // (removed elsewhere), the console takes the dissolve.
            if let View::Detail(open) = &mut self.view {
                if let Some(strip) = model.strips.iter().find(|s| s.id == open.id) {
                    open.name.clone_from(&strip.name);
                    open.standing = strip.standing;
                    open.files = strip.files;
                } else {
                    self.view = View::Console;
                }
            }
        } else {
            self.meters.clear();
            self.pulses.clear();
            self.lawns.clear();
            self.lawn_sigs.clear();
            // The engine went away: every deeper view loses its
            // subject, and the console carries the machine state.
            self.view = View::Console;
            self.confirming_remove = false;
        }
        self.selected = self.selected.min(self.strips().len().saturating_sub(1));
        true
    }

    /// Absorb the opened project's slower facts. Returns whether the
    /// panel changed (an identical refetch draws nothing).
    pub fn absorb_detail(&mut self, facts: Facts) -> bool {
        if let View::Detail(open) = &mut self.view
            && open.id == facts.id
            && open.facts.as_ref() != Some(&facts)
        {
            open.facts = Some(facts);
            return true;
        }
        false
    }

    /// Absorb a finished verb's outcome, delivered by the background
    /// task that ran it. Returns whether the machine may have changed
    /// shape — the caller re-probes at once when so.
    pub fn absorb_outcome(&mut self, outcome: Outcome) -> bool {
        self.working = None;
        if let Some(notice) = outcome.notice {
            self.notice = Some(notice.to_owned());
        }
        if outcome.to_console {
            self.view = View::Console;
        }
        if let Some(facts) = outcome.facts {
            self.absorb_detail(facts);
        }
        outcome.refreshed
    }

    /// The opened project's identifier, while S2 is up — the event loop
    /// refreshes its facts on every probe.
    #[must_use]
    pub fn detail_id(&self) -> Option<String> {
        if let View::Detail(open) = &self.view {
            Some(open.id.clone())
        } else {
            None
        }
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
            let Some(strip) = model.strips.iter().find(|s| s.id == target.id) else {
                continue;
            };
            // The lawn's pulse follows the file the indexer is on: a
            // new file births a new flash; the old one keeps decaying.
            // A report naming a file is processing evidence in its own
            // right — the 500ms poll outruns the 2s probe, so the
            // strip's standing may not say Building yet.
            if !target.current_file.is_empty()
                && self
                    .pulses
                    .get(&target.id)
                    .is_none_or(|(path, _)| *path != target.current_file)
            {
                self.pulses
                    .insert(target.id.clone(), (target.current_file.clone(), now));
                moved = true;
            }
            // Only a building strip has a meter to drive.
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

    /// Absorb a background lawn fetch. The signature is recorded even
    /// when the fetch failed upstream, so a transient miss never spins
    /// the fetcher; the lawn itself only replaces a differing one.
    /// Returns whether the frame changed.
    pub fn absorb_lawn(&mut self, id: &str, sig: FreshSig, fetched: Option<Lawn>) -> bool {
        self.lawn_sigs.insert(id.to_owned(), sig);
        if let Some(fresh) = fetched
            && self.lawns.get(id) != Some(&fresh)
        {
            self.lawns.insert(id.to_owned(), fresh);
            return true;
        }
        false
    }

    /// The lawns the event loop should fetch now: every strip whose
    /// probe-reported freshness signature differs from the one its
    /// cached lawn was fetched at.
    #[must_use]
    pub fn lawn_wants(&self) -> Vec<(String, FreshSig)> {
        let EngineState::Running(model) = &self.engine else {
            return Vec::new();
        };
        model
            .strips
            .iter()
            .filter_map(|s| {
                let sig = s.fresh_sig?;
                (self.lawn_sigs.get(&s.id) != Some(&sig)).then(|| (s.id.clone(), sig))
            })
            .collect()
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

    /// Handle one key press, routed to the view it lands in. Esc pops
    /// toward the console; at the console it quits (the console is the
    /// root). Ctrl-C quits from anywhere, even mid-edit. Returns
    /// whether state changed (the pacer draws only then).
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return true;
        }
        // A notice lives until the next key press.
        let cleared = self.notice.take().is_some();
        let changed = match &self.view {
            View::Console => self.on_key_console(key),
            View::Detail(_) => self.on_key_detail(key),
            View::PatchIn(_) => self.on_key_patchin(key),
        };
        changed || cleared
    }

    /// S1 keys: select, open, rebuild, add, remove (inline confirm),
    /// quit.
    fn on_key_console(&mut self, key: KeyEvent) -> bool {
        if self.confirming_remove {
            self.confirming_remove = false;
            if key.code == KeyCode::Char('y')
                && let Some(strip) = self.strips().get(self.selected)
            {
                self.pending = Some(Action::Remove {
                    id: strip.id.clone(),
                });
            }
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
                true
            }
            KeyCode::Left if self.selected > 0 => {
                self.selected -= 1;
                true
            }
            KeyCode::Right if self.selected + 1 < self.strips().len() => {
                self.selected += 1;
                true
            }
            KeyCode::Enter => self.open_selected(),
            KeyCode::Char('r') => self.rebuild_selected(),
            KeyCode::Char('a') if matches!(self.engine, EngineState::Running(_)) => {
                let here = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.view = View::PatchIn(PatchIn::new(&here));
                true
            }
            KeyCode::Char('x') if !self.strips().is_empty() => {
                self.confirming_remove = true;
                true
            }
            _ => false,
        }
    }

    /// Open the selected strip as S2: seed the panel from what the
    /// strip already knows, queue the facts fetch, sweep the surface
    /// open.
    fn open_selected(&mut self) -> bool {
        let strips = self.strips();
        let Some(strip) = strips.get(self.selected) else {
            return false;
        };
        self.pending = Some(Action::OpenDetail {
            id: strip.id.clone(),
        });
        self.view = View::Detail(Detail::seeded(strip));
        self.motion = Motion::start(Transition::SweepOpen);
        self.motion_strip = None;
        true
    }

    /// Queue a rebuild of the selected (or opened) project.
    fn rebuild_selected(&mut self) -> bool {
        let strips = self.strips();
        let Some(strip) = strips.get(self.selected) else {
            return false;
        };
        self.pending = Some(Action::Rebuild {
            id: strip.id.clone(),
        });
        true
    }

    /// S2 keys: edit paths, rebuild, remove (inline confirm), back,
    /// quit — and the editor's own keys while the path set is open.
    fn on_key_detail(&mut self, key: KeyEvent) -> bool {
        if self.confirming_remove {
            self.confirming_remove = false;
            if key.code == KeyCode::Char('y')
                && let View::Detail(open) = &self.view
            {
                self.pending = Some(Action::Remove {
                    id: open.id.clone(),
                });
                // The console takes the dissolve as the strip leaves.
                self.view = View::Console;
            }
            return true;
        }
        let View::Detail(open) = &mut self.view else {
            return false;
        };
        if let Some(editor) = &mut open.editing {
            return match key.code {
                KeyCode::Esc => {
                    open.editing = None;
                    true
                }
                KeyCode::Enter => {
                    self.pending = Some(Action::SavePaths {
                        id: open.id.clone(),
                        paths: editor.paths(),
                    });
                    open.editing = None;
                    true
                }
                KeyCode::Up => editor.up(),
                KeyCode::Down => editor.down(),
                KeyCode::Left => editor.field_mut().left(),
                KeyCode::Right => editor.field_mut().right(),
                KeyCode::Backspace => editor.field_mut().backspace(),
                KeyCode::Char(c) => {
                    editor.field_mut().insert(c);
                    editor.ensure_trailing_blank();
                    true
                }
                _ => false,
            };
        }
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                true
            }
            KeyCode::Esc => {
                self.view = View::Console;
                true
            }
            KeyCode::Char('e') => {
                if let Some(facts) = &open.facts {
                    open.editing = Some(PathsEditor::new(&facts.paths));
                    true
                } else {
                    false
                }
            }
            KeyCode::Char('r') => {
                self.pending = Some(Action::Rebuild {
                    id: open.id.clone(),
                });
                true
            }
            KeyCode::Char('x') => {
                self.confirming_remove = true;
                true
            }
            _ => false,
        }
    }

    /// S3 keys: the path field's editing keys, confirm, cancel. Plain
    /// letters type — `q` here is a character, not a verb.
    fn on_key_patchin(&mut self, key: KeyEvent) -> bool {
        let View::PatchIn(form) = &mut self.view else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.view = View::Console;
                true
            }
            KeyCode::Enter => {
                let path = form.path.text().trim().to_owned();
                if path.is_empty() {
                    false
                } else {
                    self.pending = Some(Action::PatchIn { path });
                    true
                }
            }
            KeyCode::Left => form.path.left(),
            KeyCode::Right => form.path.right(),
            KeyCode::Backspace => form.path.backspace(),
            KeyCode::Char(c) => {
                form.path.insert(c);
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
        self.motion.running()
            || self.meters.values().any(|glide| !glide.settled(now))
            || self
                .pulses
                .values()
                .any(|(_, born)| lawn::pulse_intensity(*born, now) > 0.0)
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
            // The lawn's pulse at this instant, while it still has any
            // light left — not gated on the standing, which can lag
            // the fresher progress reports by a whole probe interval.
            if let Some((path, born)) = self.pulses.get(&strip.id) {
                let intensity = lawn::pulse_intensity(*born, now);
                if intensity > 0.0 {
                    strip.pulse = Some((path.clone(), intensity));
                }
            }
        }
        strips
    }

    /// Draw one frame at instant `now`: title row, the current view's
    /// body with its meters eased to `now`, foot key-line, and whatever
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
        let target = match &self.view {
            View::Console => {
                let strips = self.eased_strips(now);
                let layout = self.draw_body(frame, body, &strips);
                self.motion_strip
                    .and_then(|i| layout.strip_rects.get(i).copied())
                    .unwrap_or(body)
            }
            View::Detail(open) => {
                let mut open = open.clone();
                if let Standing::Building { .. } = open.standing
                    && let Some(glide) = self.meters.get(&open.id)
                {
                    open.standing = Standing::Building {
                        fraction: glide.at(now),
                    };
                }
                detail::draw(frame, body, &open, self.confirming_remove, self.depth);
                body
            }
            View::PatchIn(form) => {
                let strips = self.eased_strips(now);
                self.draw_body(frame, body, &strips);
                patchin::draw(frame, body, form);
                body
            }
        };
        self.draw_foot(frame, foot);

        self.motion.render(frame, target, delta);
        if !self.motion.running() {
            self.motion_strip = None;
            self.leaving = None;
        }
    }

    /// The foot row: the view's key-line, and — right-aligned — either
    /// the notice a verb left behind or the word for the verb still
    /// running. The words carry the state; yellow only where the rung
    /// has it, and only for a failure.
    fn draw_foot(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(Line::from(self.foot_text()).dim(), area);
        if let Some(notice) = &self.notice {
            let line = Line::from(notice.as_str());
            let line = if self.depth == ColorDepth::Mono {
                line
            } else {
                line.yellow()
            };
            frame.render_widget(line.right_aligned(), area);
        } else if let Some(working) = self.working {
            frame.render_widget(Line::from(working).dim().right_aligned(), area);
        }
    }

    /// The key-line for the current view and state — only verbs that
    /// work right now.
    fn foot_text(&self) -> &'static str {
        if self.confirming_remove {
            return strings::FOOT_CONFIRM_REMOVE;
        }
        match &self.view {
            View::Console => match &self.engine {
                EngineState::Running(model) if model.strips.is_empty() => {
                    strings::FOOT_CONSOLE_EMPTY
                }
                EngineState::Running(_) => strings::FOOT_CONSOLE,
                _ => strings::FOOT_QUIT,
            },
            View::Detail(open) => {
                if open.editing.is_some() {
                    strings::FOOT_EDIT_PATHS
                } else {
                    strings::FOOT_DETAIL
                }
            }
            View::PatchIn(_) => strings::FOOT_PATCH_IN,
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
                &console::Body {
                    strips,
                    selected: self.selected,
                    depth: self.depth,
                    version: &model.version,
                    confirming: self.confirming_remove && matches!(self.view, View::Console),
                    lawns: &self.lawns,
                },
            )
        } else {
            console::ConsoleLayout::default()
        }
    }
}
