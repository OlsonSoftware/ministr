//! Input events, read on a dedicated thread so the draw loop stays async.
//!
//! crossterm's blocking `poll`/`read` pair runs on a plain thread and
//! feeds a tokio channel; the event loop `select!`s over that channel and
//! the engine-probe timer. The thread exits when the receiver is dropped.

use std::time::Duration;

use ratatui::crossterm::event::{self, Event as TermEvent, KeyEvent};
use tokio::sync::mpsc;

/// How long the reader waits for input before emitting a tick.
const TICK: Duration = Duration::from_millis(250);

/// One console event.
#[derive(Debug)]
pub enum Event {
    /// A key was pressed.
    Key(KeyEvent),
    /// The terminal was resized; the next draw picks up the new size.
    Resize,
    /// Nothing happened for a tick interval.
    Tick,
}

/// Spawn the input-reader thread and return its channel.
#[must_use]
pub fn spawn_reader() -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel(64);
    std::thread::spawn(move || {
        loop {
            let ready = event::poll(TICK).unwrap_or(false);
            let ev = if ready {
                match event::read() {
                    Ok(TermEvent::Key(key)) => Event::Key(key),
                    Ok(TermEvent::Resize(_, _)) => Event::Resize,
                    Ok(_) => continue,
                    Err(_) => return,
                }
            } else {
                Event::Tick
            };
            if tx.blocking_send(ev).is_err() {
                return;
            }
        }
    });
    rx
}
