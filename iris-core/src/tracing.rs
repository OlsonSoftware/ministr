//! Tracing infrastructure for iris.
//!
//! Provides a single `init_tracing()` entry point that configures
//! `tracing-subscriber` with `EnvFilter` support and optional JSON output.

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// The environment variable that controls log filtering (e.g. `RUST_LOG=iris_core=debug`).
const ENV_FILTER_VAR: &str = "RUST_LOG";

/// The environment variable that selects output format (`json` or `pretty`).
const LOG_FORMAT_VAR: &str = "IRIS_LOG_FORMAT";

/// Default filter when `RUST_LOG` is not set.
const DEFAULT_FILTER: &str = "iris_core=info,iris_mcp=info,iris_cli=info,warn";

/// Initialize the global tracing subscriber.
///
/// - Reads `RUST_LOG` for filter directives (falls back to `DEFAULT_FILTER`).
/// - Reads `IRIS_LOG_FORMAT`: if set to `json`, outputs structured JSON lines;
///   otherwise outputs human-readable logs.
///
/// # Panics
///
/// Panics if a global subscriber has already been set.
pub fn init_tracing() {
    let filter =
        EnvFilter::try_from_env(ENV_FILTER_VAR).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    let json = std::env::var(LOG_FORMAT_VAR)
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    // Always write to stderr so stdout remains free for MCP stdio transport.
    if json {
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
