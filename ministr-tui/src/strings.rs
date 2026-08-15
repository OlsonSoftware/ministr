//! Every user-facing string in the console, in one place.
//!
//! GUI-BLUEPRINT-v8 §4 is binding: plain words only. The map is
//! project (never the index-side term), engine (never the process name),
//! rebuild, needs update, add, remove. No emoji, no exclamation marks,
//! no decorative glyphs. `tests/language.rs` scans every string literal
//! in `src/` mechanically; keeping the words here keeps the whole
//! surface auditable in one read.

/// The console's name, shown at the head of the title row.
pub const APP_NAME: &str = "ministr";

/// Machine state: the engine answered the last probe.
pub const ENGINE_RUNNING: &str = "engine ● running";

/// Machine state: the engine was started and has not answered yet.
pub const ENGINE_STARTING: &str = "engine ○ starting…";

/// Machine state: the engine did not answer the last probe.
pub const ENGINE_UNREACHABLE: &str = "can't reach the engine";

/// Foot line: the quit key, the only verb this scaffold ships.
pub const FOOT_QUIT: &str = "q quit";

/// Body line when the engine holds no projects.
pub const NO_PROJECTS: &str = "no projects yet";

/// Body line: how many projects the engine holds.
#[must_use]
pub fn projects_line(count: usize) -> String {
    match count {
        0 => NO_PROJECTS.to_owned(),
        1 => "1 project".to_owned(),
        n => format!("{n} projects"),
    }
}
