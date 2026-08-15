//! ministr-tui — the terminal console for the ministr index engine.
//!
//! Opened with `ministr ui`. This crate owns terminal setup/teardown
//! (raw mode, alternate screen, panic-hook restore), the event loop, the
//! engine plumbing, and every frame. The binding design direction is
//! GUI-BLUEPRINT-v8 (Master-Control Console); the console screens land
//! in the next chunk. What ships here besides the placeholder frame is
//! the rendering infrastructure every screen builds on (Amendment A):
//! synchronized frames ([`sync`]), the color capability ladder
//! ([`palette`]), adaptive frame pacing ([`pacing`]), lawful
//! transitions ([`motion`]), and the sub-cell meter ([`meter`]).
//!
//! Language rule (blueprint §4): every user-facing string lives in
//! [`strings`] and says project / engine / rebuild / needs update — never
//! the internal vocabulary. `tests/language.rs` enforces this mechanically
//! over all string literals in this crate.

pub mod app;
pub mod engine;
pub mod event;
pub mod meter;
pub mod motion;
pub mod pacing;
pub mod palette;
pub mod strings;
pub mod sync;

use std::time::{Duration, Instant};

use ministr_api::client::DaemonClient;
use ratatui::DefaultTerminal;

use crate::app::App;
use crate::pacing::Pacer;

/// How often the frame re-probes the engine for machine state.
const PROBE_INTERVAL: Duration = Duration::from_secs(2);

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
    engine::ensure_engine(&client).await;

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &client).await;
    ratatui::restore();
    result
}

/// Draw when something changed, wait otherwise, repeat until quit.
///
/// Pacing is adaptive (GUI-BLUEPRINT-v8 Amendment A): at rest the loop
/// wakes on input and probe answers but draws nothing; while a
/// transition plays it draws at the frame clock, feeding effects real
/// elapsed time. Every draw goes through [`sync::draw_synced`].
async fn event_loop(terminal: &mut DefaultTerminal, client: &DaemonClient) -> Result<(), UiError> {
    let mut app = App::new();
    let mut events = event::spawn_reader();
    // `interval` fires immediately, so the first frame after this one
    // already carries a real probe result instead of a stuck "starting".
    let mut probe_timer = tokio::time::interval(PROBE_INTERVAL);
    let mut pacer = Pacer::new();
    let mut last_frame = Instant::now();

    loop {
        pacer.set_animating(app.animating());
        if pacer.take_redraw() {
            let delta = last_frame.elapsed().min(pacing::MAX_FRAME_DELTA);
            last_frame = Instant::now();
            // Stdout is the same stream the default backend writes to,
            // so the sync sequences bracket the frame bytes in order.
            sync::draw_synced(&mut std::io::stdout(), terminal, |frame| {
                app.draw(frame, delta);
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
                if state != app.engine {
                    app.engine = state;
                    pacer.mark_dirty();
                }
            }
            // The frame clock: armed only while a transition plays, so
            // the rest state stays fully event-driven.
            () = tokio::time::sleep(pacing::FRAME_INTERVAL), if pacer.is_animating() => {}
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
