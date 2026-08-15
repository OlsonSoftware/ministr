//! ministr-tui — the terminal console for the ministr index engine.
//!
//! Opened with `ministr ui`. This crate owns terminal setup/teardown
//! (raw mode, alternate screen, panic-hook restore), the event loop, the
//! engine plumbing, and every frame. The binding design direction is
//! GUI-BLUEPRINT-v8 (Master-Control Console). The home screen is S1 —
//! The Console ([`console`]): one channel strip per project beside a
//! master section, rendered through the rendering infrastructure of
//! Amendment A: synchronized frames ([`sync`]), the color capability
//! ladder ([`palette`]), adaptive frame pacing ([`pacing`]), lawful
//! transitions ([`motion`]), and the sub-cell meter ([`meter`]).
//!
//! Language rule (blueprint §4): every user-facing string lives in
//! [`strings`] and says project / engine / rebuild / needs update — never
//! the internal vocabulary. `tests/language.rs` enforces this mechanically
//! over all string literals in this crate.

pub mod app;
pub mod console;
pub mod detail;
pub mod ease;
pub mod engine;
pub mod event;
pub mod field;
pub mod meter;
pub mod motion;
pub mod pacing;
pub mod palette;
pub mod patchin;
pub mod strings;
pub mod sync;

use std::time::{Duration, Instant};

use ministr_api::client::DaemonClient;
use ratatui::DefaultTerminal;

use crate::app::{App, View};
use crate::engine::Action;
use crate::pacing::Pacer;

/// How often the frame re-probes the engine for machine state.
const PROBE_INTERVAL: Duration = Duration::from_secs(2);

/// How often the live meters poll the engine's progress counters —
/// only while something is building; at rest this poll never runs.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

/// Errors the console can exit with.
#[derive(Debug, thiserror::Error)]
pub enum UiError {
    /// Drawing or terminal control failed.
    #[error("terminal error: {0}")]
    Io(#[from] std::io::Error),
}

/// Open the console, run until quit, restore the terminal.
///
/// The terminal is restored on the quit path here and on the panic path
/// by the hook `ratatui::init` installs (it puts the terminal back before
/// the panic message prints, so a crash never leaves the shell raw).
///
/// # Errors
///
/// Returns [`UiError::Io`] when terminal control or drawing fails.
pub async fn run() -> Result<(), UiError> {
    let client = DaemonClient::new();
    // The engine handshake (spawn if missing, wait until it answers)
    // runs in the background: the console opens at once and the title
    // reads "starting…" honestly until the engine is up — a cold
    // machine never stares at a blank shell.
    let spawning = tokio::spawn(async {
        let client = DaemonClient::new();
        engine::ensure_engine(&client).await;
    });

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &client, spawning).await;
    ratatui::restore();
    result
}

/// Draw when something changed, wait otherwise, repeat until quit.
///
/// Pacing is adaptive (GUI-BLUEPRINT-v8 Amendment A): at rest the loop
/// wakes on input and probe answers but draws nothing; while a
/// transition plays it draws at the frame clock, feeding effects real
/// elapsed time. Every draw goes through [`sync::draw_synced`].
async fn event_loop(
    terminal: &mut DefaultTerminal,
    client: &DaemonClient,
    mut spawning: tokio::task::JoinHandle<()>,
) -> Result<(), UiError> {
    let mut app = App::new();
    let mut events = event::spawn_reader();
    // `interval` fires immediately, so the first frame after this one
    // already carries a real probe result instead of a stuck "starting".
    let mut probe_timer = tokio::time::interval(PROBE_INTERVAL);
    // While nothing builds this timer is not polled and its ticks go
    // missing; skipping them keeps a build's first poll from replaying
    // the whole quiet spell as a burst.
    let mut progress_timer = tokio::time::interval(PROGRESS_INTERVAL);
    progress_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut spawn_done = false;
    let mut pacer = Pacer::new();
    let mut last_frame = Instant::now();

    loop {
        pacer.set_animating(app.animating(Instant::now()));
        if pacer.take_redraw() {
            let now = Instant::now();
            let delta = now.duration_since(last_frame).min(pacing::MAX_FRAME_DELTA);
            last_frame = now;
            // Stdout is the same stream the default backend writes to,
            // so the sync sequences bracket the frame bytes in order.
            sync::draw_synced(&mut std::io::stdout(), terminal, |frame| {
                app.draw(frame, delta, now);
            })?;
        }

        tokio::select! {
            ev = events.recv() => match ev {
                Some(event::Event::Key(key)) => {
                    if app.on_key(key) {
                        pacer.mark_dirty();
                    }
                }
                Some(event::Event::Resize) => pacer.mark_dirty(),
                Some(event::Event::Tick) => {}
                // The reader thread is gone; without input the console
                // can only be quit from outside, so leave cleanly.
                None => app.should_quit = true,
            },
            _ = probe_timer.tick() => {
                let state = engine::probe(client).await;
                // While the background handshake is still bringing the
                // engine up, an unanswered probe is expected — the
                // title keeps "starting…" instead of raising an alarm
                // the machine is about to answer.
                let starting_grace = !spawn_done
                    && matches!(state, engine::EngineState::Unreachable)
                    && matches!(app.engine, engine::EngineState::Starting);
                if !starting_grace && app.absorb(state) {
                    pacer.mark_dirty();
                }
                // An opened project's slower facts ride the same
                // cadence, so its counts and path set stay current.
                if let Some(id) = app.detail_id()
                    && let Some(facts) = engine::detail(client, &id).await
                    && app.absorb_detail(facts)
                {
                    pacer.mark_dirty();
                }
            }
            // The engine handshake finished: probe at once so the
            // title flips the moment the engine is really up.
            _ = &mut spawning, if !spawn_done => {
                spawn_done = true;
                probe_timer.reset_immediately();
            }
            // The fast progress poll: armed only while something is
            // building, so the rest state never polls it.
            _ = progress_timer.tick(), if app.building() => {
                if let Some(targets) = engine::progress(client).await
                    && app.absorb_progress(&targets, Instant::now())
                {
                    pacer.mark_dirty();
                }
            }
            // The frame clock: armed only while a transition plays or
            // a needle glides, so the rest state stays event-driven.
            () = tokio::time::sleep(pacing::FRAME_INTERVAL), if pacer.is_animating() => {}
        }

        // A key press queued a verb: run it against the engine, then
        // probe at once so the console reflects the change (and its
        // transition plays) without waiting out the probe interval.
        if let Some(action) = app.pending.take() {
            if run_action(client, &mut app, action).await {
                probe_timer.reset_immediately();
            }
            pacer.mark_dirty();
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Run one queued verb against the engine. Returns whether the machine
/// may have changed shape — the caller re-probes at once when so. A
/// failed verb leaves a plain-worded notice and changes nothing.
async fn run_action(client: &DaemonClient, app: &mut App, action: Action) -> bool {
    match action {
        Action::OpenDetail { id } => {
            if let Some(facts) = engine::detail(client, &id).await {
                app.absorb_detail(facts);
            }
            false
        }
        Action::Rebuild { id } => settle(
            app,
            client.reindex_corpus(&id).await.is_ok(),
            strings::NOTICE_REBUILD_FAILED,
        ),
        Action::Remove { id } => settle(
            app,
            client.unregister_corpus(&id).await.is_ok(),
            strings::NOTICE_REMOVE_FAILED,
        ),
        Action::PatchIn { path } => {
            let ok = settle(
                app,
                client.register_corpus(&[path]).await.is_ok(),
                strings::NOTICE_ADD_FAILED,
            );
            if ok {
                // Back to the console: the new strip materializes
                // there the moment the next probe reports it.
                app.view = View::Console;
            }
            ok
        }
        Action::SavePaths { id, paths } => {
            let ok = settle(
                app,
                client.update_corpus_paths(&id, &paths).await.is_ok(),
                strings::NOTICE_PATHS_FAILED,
            );
            // The path set changed: refresh the opened panel now.
            if ok && let Some(facts) = engine::detail(client, &id).await {
                app.absorb_detail(facts);
            }
            ok
        }
    }
}

/// A verb's outcome: a success changes the machine, a failure leaves a
/// plain-worded notice and changes nothing.
fn settle(app: &mut App, ok: bool, failure: &str) -> bool {
    if !ok {
        app.notice = Some(failure.to_owned());
    }
    ok
}
