//! Coherence event stream — file-watcher change records for the ministr daemon.
//!
//! Every filesystem event the daemon's per-corpus watcher observes is
//! recorded as a [`CoherenceEvent`] in an in-memory ring buffer. Tauri,
//! CLI, and MCP consumers read back via `GET /coherence-events` or the
//! matching [`crate::client::DaemonClient`] wrapper.
//!
//! Events are denormalized — file path, event kind, and the list of
//! affected section IDs travel together so a consumer can render a row
//! without cross-referencing storage.
//!
//! See also: [`crate::client::DaemonClient::recent_coherence_events`] and
//! the `/coherence-events` route on the daemon.

use serde::{Deserialize, Serialize};

/// Kind of filesystem change the watcher observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CoherenceKind {
    /// A new file appeared in a watched directory.
    Created,
    /// An existing file's contents changed on disk.
    Modified,
    /// A file was removed from a watched directory.
    Removed,
}

impl CoherenceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Removed => "removed",
        }
    }
}

/// A single file-change activity record.
///
/// Unlike [`crate::activity::ActivityEvent`] (a tool call the agent made),
/// a coherence event is a filesystem change the daemon observed — the
/// cache-invalidation side of the observatory.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CoherenceEvent {
    /// Wall-clock timestamp when the watcher observed the change (unix ms).
    pub timestamp_ms: u64,

    /// Corpus the change belongs to.
    pub corpus_id: String,

    /// What kind of change this is.
    pub kind: CoherenceKind,

    /// Absolute path of the file that changed.
    pub path: String,

    /// IDs of sections affected by the change — empty for `Created`
    /// events (no prior index entries to invalidate) and populated for
    /// `Modified` / `Removed` events with the list of pre-change sections.
    #[serde(default)]
    pub affected_sections: Vec<String>,

    /// Wall-clock duration of the re-index triggered by this event, in
    /// milliseconds. `0` if the event was broadcast before re-indexing
    /// started (which is typical — the feed shouldn't block on indexing).
    #[serde(default)]
    pub duration_ms: u64,
}

impl CoherenceEvent {
    /// Construct a minimal event with required fields populated.
    #[must_use]
    pub fn new(
        timestamp_ms: u64,
        corpus_id: impl Into<String>,
        kind: CoherenceKind,
        path: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_ms,
            corpus_id: corpus_id.into(),
            kind,
            path: path.into(),
            affected_sections: Vec::new(),
            duration_ms: 0,
        }
    }
}

/// Response shape for the `GET /coherence-events` route and the
/// `recent_coherence_events` client wrapper.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CoherenceEventsResponse {
    /// Events in newest-first order.
    pub events: Vec<CoherenceEvent>,
    /// Total capacity of the daemon's in-memory ring buffer.
    pub buffer_capacity: usize,
}
