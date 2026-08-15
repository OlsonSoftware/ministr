//! ministr-tui — the terminal console for the ministr index engine.
//!
//! Opened with `ministr ui`. This crate owns terminal setup/teardown
//! (raw mode, alternate screen, panic-hook restore), the event loop, the
//! engine plumbing, and every frame. The binding design direction is
//! GUI-BLUEPRINT-v8 (Master-Control Console); this scaffold ships the
//! placeholder frame only — the console screens land in the next chunk.
//!
//! Language rule (blueprint §4): every user-facing string lives in
//! [`strings`] and says project / engine / rebuild / needs update — never
//! the internal vocabulary. `tests/language.rs` enforces this mechanically
//! over all string literals in this crate.

pub mod app;
pub mod engine;
pub mod event;
pub mod strings;

use std::time::Duration;

use ministr_api::client::DaemonClient;
use ratatui::DefaultTerminal;

use crate::app::App;

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

/// Draw, wait for input or the probe timer, repeat until quit.
async fn event_loop(terminal: &mut DefaultTerminal, client: &DaemonClient) -> Result<(), UiError> {
    let mut app = App::new();
    let mut events = event::spawn_reader();
    // `interval` fires immediately, so the first frame after this one
    // already carries a real probe result instead of a stuck "starting".
    let mut probe_timer = tokio::time::interval(PROBE_INTERVAL);

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        tokio::select! {
            ev = events.recv() => match ev {
                Some(event::Event::Key(key)) => app.on_key(key),
                Some(event::Event::Resize | event::Event::Tick) => {}
                // The reader thread is gone; without input the console
                // can only be quit from outside, so leave cleanly.
                None => app.should_quit = true,
            },
            _ = probe_timer.tick() => {
                app.engine = engine::probe(client).await;
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
