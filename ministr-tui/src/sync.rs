//! Synchronized frames — a frame appears atomically or not at all.
//!
//! GUI-BLUEPRINT-v8 Amendment A: every draw is wrapped in the terminal's
//! synchronized-update mode (DEC mode 2026), so the terminal holds the
//! partial frame back and presents it in one piece — zero flicker, zero
//! tearing. Terminals that do not know the mode ignore the sequences,
//! so there is no capability check and nothing to degrade.

use std::io::{self, Write};

use ratatui::backend::Backend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::{Frame, Terminal};

/// Draw one frame inside a synchronized update.
///
/// `sink` must be the same stream the terminal's backend writes to —
/// stdout for the real console, so the begin/end sequences land in
/// order around the frame bytes (each `execute!` flushes, and the draw
/// flushes in between). It is a separate parameter because ratatui
/// keeps its backend writer private behind an unstable feature.
///
/// The update is ended even when drawing fails — an error must never
/// leave the terminal holding frames back.
///
/// # Errors
///
/// Returns the first error from the begin sequence, the draw, or the
/// end sequence.
pub fn draw_synced<W, B, F>(sink: &mut W, terminal: &mut Terminal<B>, render: F) -> io::Result<()>
where
    W: Write,
    B: Backend,
    F: FnOnce(&mut Frame),
    io::Error: From<B::Error>,
{
    execute!(sink, BeginSynchronizedUpdate)?;
    let drawn = terminal.draw(render).map(|_| ());
    let ended = execute!(sink, EndSynchronizedUpdate);
    drawn?;
    ended
}
