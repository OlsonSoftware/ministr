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

/// Foot line when there is nothing to select.
pub const FOOT_QUIT: &str = "q quit";

/// Foot line on the console: only the verbs that work today. The
/// remaining console verbs (open, rebuild, add, remove) join this line
/// with the chunk that wires them.
pub const FOOT_CONSOLE: &str = "← → select   q quit";

/// Body line when the engine holds no projects.
pub const NO_PROJECTS: &str = "no projects yet";

/// Strip foot: the index matches the working tree.
pub const STANDING_UP_TO_DATE: &str = "up to date";

/// Strip foot: files changed since the index was built. Routine.
pub const STANDING_NEEDS_UPDATE: &str = "needs update";

/// Strip foot: enqueued behind another build.
pub const STANDING_WAITING: &str = "waiting";

/// Strip foot: the engine is loading this project into memory.
pub const STANDING_WARMING: &str = "warming up";

/// Strip foot: the last build failed.
pub const STANDING_FAILED: &str = "failed";

/// Master section head.
pub const MASTER_ENGINE: &str = "engine";

/// Master section state word — the head already says whose state.
pub const STATE_RUNNING: &str = "● running";

/// Strip foot while building: the live word plus how far along.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn updating_line(fraction: f64) -> String {
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).round() as u8;
    format!("updating {percent}%")
}

/// Strip foot: how many files the project's index holds.
#[must_use]
pub fn files_line(count: usize) -> String {
    match count {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    }
}

/// Master section: the engine's version.
#[must_use]
pub fn version_line(version: &str) -> String {
    format!("version {version}")
}

/// Body line: how many projects the engine holds.
#[must_use]
pub fn projects_line(count: usize) -> String {
    match count {
        0 => NO_PROJECTS.to_owned(),
        1 => "1 project".to_owned(),
        n => format!("{n} projects"),
    }
}

/// Edge marker: more projects sit past the left (or top) edge.
#[must_use]
pub fn more_left(count: usize) -> String {
    format!("‹ {count} more")
}

/// Edge marker: more projects sit past the right (or bottom) edge.
#[must_use]
pub fn more_right(count: usize) -> String {
    format!("{count} more ›")
}
