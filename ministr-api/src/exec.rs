//! Wire types for the daemon's exec (recorded shell) routes.
//!
//! The daemon hosts one shared run engine (exec-epic); these are the
//! request/response shapes its `/exec/runs*` routes speak and
//! [`crate::client::DaemonClient`]'s exec methods return. Mirrors the
//! engine's `RunRecord` field-for-field — the daemon converts at the
//! route boundary (`ministr-api` cannot depend on `ministr-daemon`).

use serde::{Deserialize, Serialize};

/// Request body for `POST /exec/runs`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartExecRun {
    /// Shell command line to execute.
    pub command: String,
    /// Working directory; defaults to the first allowed corpus root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Originating agent session, for audit attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Timeout in seconds (engine default when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// When true, return immediately with the run id.
    #[serde(default)]
    pub background: bool,
}

/// One recorded run, as the daemon serves it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRun {
    /// Unique run id (`run-<ms>-<seq>`).
    pub run_id: String,
    /// The command line as requested.
    pub command: String,
    /// Canonicalized working directory.
    pub cwd: String,
    /// Originating agent session, if attributed.
    pub session_id: Option<String>,
    /// Associated corpus, if attributed.
    pub corpus_id: Option<String>,
    /// Short fingerprint of the spawn environment.
    pub env_fingerprint: String,
    /// Spawn time, unix milliseconds.
    pub started_at_ms: i64,
    /// Finish time, unix milliseconds (`None` while running).
    pub finished_at_ms: Option<i64>,
    /// Process exit code (`None` while running or signal-killed).
    pub exit_code: Option<i32>,
    /// Lifecycle: `running` | `exited` | `killed` | `timed_out`.
    pub status: String,
    /// Captured log (persisted form; empty while running).
    pub log: String,
    /// True when the output guard dropped the middle.
    pub truncated: bool,
    /// Exact total bytes produced.
    pub bytes_total: u64,
}

/// Response for `GET /exec/runs/{id}/logs` — the persisted log for
/// finished runs, or a LIVE mid-run snapshot for running ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecLogs {
    /// Run id.
    pub run_id: String,
    /// Lifecycle at snapshot time.
    pub status: String,
    /// Captured output (live snapshot while running).
    pub log: String,
    /// True when the output guard dropped the middle.
    pub truncated: bool,
    /// Exact bytes produced so far (final after exit).
    pub bytes_total: u64,
    /// True when this is a mid-run snapshot rather than the final log.
    pub live: bool,
}

/// Response for `POST /exec/runs/{id}/kill`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecKill {
    /// Run id.
    pub run_id: String,
    /// True when the run was still active and cancellation was requested.
    pub killed: bool,
}
