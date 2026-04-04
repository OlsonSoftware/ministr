//! Tracing infrastructure for iris.
//!
//! Provides `init_tracing()` for CLI/MCP use (stderr only) and
//! `init_tracing_with_file()` for the desktop app (stderr + rolling log file).

use std::path::Path;

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// The environment variable that controls log filtering (e.g. `RUST_LOG=iris_core=debug`).
const ENV_FILTER_VAR: &str = "RUST_LOG";

/// The environment variable that selects output format (`json` or `pretty`).
const LOG_FORMAT_VAR: &str = "IRIS_LOG_FORMAT";

/// Default filter when `RUST_LOG` is not set.
const DEFAULT_FILTER: &str = "iris_core=info,iris_mcp=info,iris_daemon=info,iris_cli=info,warn";

fn build_filter() -> EnvFilter {
    EnvFilter::try_from_env(ENV_FILTER_VAR).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER))
}

fn is_json() -> bool {
    std::env::var(LOG_FORMAT_VAR)
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// Initialize the global tracing subscriber (stderr only).
///
/// Used by the CLI and MCP server where stdout is reserved for transport.
///
/// # Panics
///
/// Panics if a global subscriber has already been set.
pub fn init_tracing() {
    let filter = build_filter();

    if is_json() {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json().with_writer(std::io::stderr))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    }
}

/// Initialize tracing with both stderr and a log file.
///
/// The log file is truncated on startup (keeps only the current session).
/// Used by the desktop tray app so the log viewer tab has content to display.
///
/// # Panics
///
/// Panics if a global subscriber has already been set or if the log file
/// cannot be created.
pub fn init_tracing_with_file(log_path: &Path) {
    let filter = build_filter();

    // Ensure parent directory exists.
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let file = std::fs::File::create(log_path).expect("failed to create log file");

    // File layer: always plain text, no ANSI colors.
    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file));

    // Stderr layer: human-readable with colors.
    let stderr_layer = fmt::layer().with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();
}
