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
}
