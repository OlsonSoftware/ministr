//! Activity event stream — per-tool-call records for the iris daemon.
//!
//! Every `iris_*` MCP tool call that reaches the daemon gets recorded as an
//! `ActivityEvent` in an in-memory ring buffer. Tauri / CLI / MCP consumers
//! read back via `GET /activity` or the matching `DaemonClient` wrapper.
//!
//! Events are strictly informational — dropping an event never fails a tool
//! call. The ring buffer is bounded, so old events age out as new calls
//! arrive.
//!
//! See also: [`crate::client::DaemonClient::recent_activity`] and the
//! `/activity` route on the daemon.

use serde::{Deserialize, Serialize};

/// A single tool-call activity record.
///
/// Fields are deliberately denormalized so a consumer can render a row
/// without cross-referencing session or corpus state it may not hold.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ActivityEvent {
    /// Wall-clock timestamp when the call completed (unix milliseconds).
    pub timestamp_ms: u64,

    /// Tool name, e.g. `"iris_survey"` or `"iris_read"`.
    pub tool: String,

    /// Corpus the call ran against.
    pub corpus_id: String,

    /// Session the call belonged to, if any. `None` for tools without a
    /// session context (e.g. administrative `iris_fetch` / `iris_clone`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// One-line human blurb suitable for a UI row. Typically an argument
    /// echo like `"src/auth.rs#logout"` or `"authentication middleware"`.
    #[serde(default)]
    pub summary: String,

    /// Tokens served / delta on this call when it can be measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_delta: Option<u64>,

    /// Pressure level observed on the session after this call:
    /// `"normal"`, `"elevated"`, or `"critical"`. `None` for calls
    /// without a session context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<String>,

    /// `true` when the call resolved from a warm cache (dedup, delta,
    /// or prefetch hit). Defaults to `false`.
    #[serde(default)]
    pub cache_hit: bool,

    /// Resolution level the call served at, if applicable —
    /// `"document"`, `"section"`, `"claim"`, or `"symbol"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,

    /// Wall-clock duration of the call in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
}

impl ActivityEvent {
    /// Construct a minimal event with the required fields populated and the
    /// rest defaulted. Callers typically use the daemon-side guard type that
    /// fills in the remaining fields via `with_*` methods.
    #[must_use]
    pub fn new(timestamp_ms: u64, tool: impl Into<String>, corpus_id: impl Into<String>) -> Self {
        Self {
            timestamp_ms,
            tool: tool.into(),
            corpus_id: corpus_id.into(),
            session_id: None,
            summary: String::new(),
            tokens_delta: None,
            pressure: None,
            cache_hit: false,
            resolution: None,
            duration_ms: 0,
        }
    }
}

/// Response shape for the `GET /activity` route and the
/// `recent_activity` client wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ActivityResponse {
    /// Events in newest-first order.
    pub events: Vec<ActivityEvent>,
    /// Total capacity of the daemon's in-memory ring buffer.
    /// Older events age out as new ones arrive.
    pub buffer_capacity: usize,
}
