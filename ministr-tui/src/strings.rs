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

/// Foot line when there is nothing to select and nothing to add — the
/// engine is not up yet.
pub const FOOT_QUIT: &str = "q quit";

/// Foot line on the console with projects to work on: every verb works.
pub const FOOT_CONSOLE: &str = "← → select   enter open   r rebuild   a add   x remove   q quit";

/// Foot line on the console when the engine is up but holds nothing —
/// add is the only project verb that has anything to act on.
pub const FOOT_CONSOLE_EMPTY: &str = "a add   q quit";

/// Foot line while an inline remove waits for its answer.
pub const FOOT_CONFIRM_REMOVE: &str = "y remove   n keep";

/// Foot line on an opened project.
pub const FOOT_DETAIL: &str = "e edit paths   r rebuild   x remove   esc back   q quit";

/// Foot line while the path set is being edited.
pub const FOOT_EDIT_PATHS: &str = "↑ ↓ path   enter save   esc cancel";

/// Foot line on the patch-in panel.
pub const FOOT_PATCH_IN: &str = "enter add   esc cancel";

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

/// The inline remove question, asked on the strip itself.
pub const CONFIRM_REMOVE: &str = "remove this project?";

/// Detail label: the project's path set.
pub const DETAIL_PATHS: &str = "paths";

/// Detail label: the project's standing.
pub const DETAIL_STANDING: &str = "standing";

/// Detail label: how many files the project's index holds.
pub const DETAIL_FILES: &str = "files";

/// Detail label: how many sections the project's index holds.
pub const DETAIL_SECTIONS: &str = "sections";

/// Detail label: how many code symbols the project's index holds.
pub const DETAIL_SYMBOLS: &str = "symbols";

/// Detail label: when the project was last built.
pub const DETAIL_UPDATED: &str = "last built";

/// Detail value while the slower answers are still on their way.
pub const LOADING: &str = "…";

/// Detail value when the project has never finished a build.
pub const NEVER_BUILT: &str = "not built yet";

/// The blank editor row that grows the path set when typed into.
pub const ADD_PATH_HINT: &str = "add another path";

/// Patch-in panel head.
pub const PATCH_IN_TITLE: &str = "add a project";

/// The one-sentence consequence under the patch-in field.
pub const PATCH_IN_CONSEQUENCE: &str =
    "the engine reads every file under this path and keeps the project up to date";

/// Foot word while an add runs on the engine.
pub const WORKING_ADD: &str = "adding…";

/// Foot word while a rebuild starts on the engine.
pub const WORKING_REBUILD: &str = "starting the rebuild…";

/// Foot word while a remove runs on the engine.
pub const WORKING_REMOVE: &str = "removing…";

/// Foot word while a path-set save runs on the engine.
pub const WORKING_SAVE: &str = "saving…";

/// Notice: the rebuild verb got no answer, or a refusal.
pub const NOTICE_REBUILD_FAILED: &str = "the rebuild didn't start";

/// Notice: the remove verb got no answer, or a refusal.
pub const NOTICE_REMOVE_FAILED: &str = "the project wasn't removed";

/// Notice: the add verb got no answer, or a refusal.
pub const NOTICE_ADD_FAILED: &str = "that folder couldn't be added";

/// Notice: the path-set save got no answer, or a refusal.
pub const NOTICE_PATHS_FAILED: &str = "the paths weren't changed";

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

/// Detail value: how long ago the last build finished, from its age in
/// seconds. Computed at fetch time — never in a draw — so rendering
/// stays a pure function of the model.
#[must_use]
pub fn ago_line(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=119 => "1 minute ago".to_owned(),
        120..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=7199 => "1 hour ago".to_owned(),
        7200..=86399 => format!("{} hours ago", seconds / 3600),
        86400..=172_799 => "1 day ago".to_owned(),
        _ => format!("{} days ago", seconds / 86400),
    }
}

/// Detail line: what needs updating, in plain counts. `None` when the
/// index matches the working tree.
#[must_use]
pub fn attention_line(changed: usize, new_files: usize, missing: usize) -> Option<String> {
    let mut parts = Vec::new();
    match changed {
        0 => {}
        1 => parts.push("1 file changed".to_owned()),
        n => parts.push(format!("{n} files changed")),
    }
    match new_files {
        0 => {}
        1 => parts.push("1 new".to_owned()),
        n => parts.push(format!("{n} new")),
    }
    match missing {
        0 => {}
        1 => parts.push("1 missing".to_owned()),
        n => parts.push(format!("{n} missing")),
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("{} since the last build", parts.join(" · ")))
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
