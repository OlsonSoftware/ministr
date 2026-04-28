//! Daemon status and health API types.

use serde::{Deserialize, Serialize};

use crate::corpus::CorpusInfo;

/// Overall daemon status response.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DaemonStatus {
    /// Daemon version string.
    pub version: String,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Resident set size in megabytes.
    pub memory_mb: f64,
    /// Embedding model name.
    pub model: String,
    /// Embedding model dimension.
    pub model_dimension: usize,
    /// All registered corpora with their status.
    pub corpora: Vec<CorpusInfo>,
    /// Path to the daemon log file (if file logging is active).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// Total active sessions across all corpora.
    #[serde(default)]
    pub total_sessions: usize,
    /// Whether the desktop tray app is enabled to launch at login.
    /// `None` from the headless daemon (it has no autolaunch concept);
    /// `Some(_)` only when populated by the Tauri `daemon_status` command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autostart_enabled: Option<bool>,
}
