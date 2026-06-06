//! `exec_tools` — state + handlers behind the `ministr_run` tool family
//! (exec-mcp-tools).
//!
//! Bundles everything the run tools need into one [`ExecState`] so the
//! server's many constructors initialize it with a single
//! `ExecState::default()`. The [`ministr_daemon::exec::RunEngine`] is
//! created lazily on first use (db at `~/.ministr/exec_runs.db`); the
//! allowed cwd roots are stashed by `prune_tools` / `set_exec_roots`
//! from whatever corpus paths the host wired.
//!
//! The tool methods themselves live in `server/mod.rs` (the one
//! `#[tool_router]` impl) and delegate here, keeping that file thin.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ministr_daemon::exec::{RootsProvider, RunEngine, RunEngineConfig, RunRecord, RunRequest};
use serde::{Deserialize, Serialize};

use crate::run_digest::{RunDigest, digest, next_span};

/// Hard ceiling on a requested timeout.
const MAX_TIMEOUT_SECS: u64 = 3600;
/// Default page size for `ministr_run_logs`.
const DEFAULT_LOG_PAGE_BYTES: usize = 16 * 1024;

/// Shared state behind the `ministr_run` tool family.
#[derive(Clone, Default)]
pub(crate) struct ExecState {
    /// Lazily-created run engine (shared across forked connections).
    engine: Arc<std::sync::Mutex<Option<Arc<RunEngine>>>>,
    /// Allowed cwd roots — stashed from the host's corpus paths.
    roots: Arc<std::sync::Mutex<Vec<PathBuf>>>,
    /// Never-resend cursors: (`session_id`, `run_id`) → delivered byte offset.
    cursors: Arc<std::sync::Mutex<HashMap<(String, String), usize>>>,
    /// Explicit run-store path; defaults to `~/.ministr/exec_runs.db`.
    /// Must be set before the first run (the engine is created once).
    db_path: Arc<std::sync::Mutex<Option<PathBuf>>>,
}

/// [`RootsProvider`] over the shared, late-bound roots list.
struct SharedRoots(Arc<std::sync::Mutex<Vec<PathBuf>>>);

impl RootsProvider for SharedRoots {
    fn allowed_roots(&self) -> Vec<PathBuf> {
        self.0.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

impl ExecState {
    /// Replace the allowed cwd roots (idempotent; host wiring).
    pub(crate) fn set_roots(&self, roots: Vec<PathBuf>) {
        if let Ok(mut guard) = self.roots.lock() {
            *guard = roots;
        }
    }

    /// Override the run-store path (hosts/tests; before the first run).
    pub(crate) fn set_db_path(&self, path: PathBuf) {
        if let Ok(mut guard) = self.db_path.lock() {
            *guard = Some(path);
        }
    }

    fn first_root(&self) -> Option<PathBuf> {
        self.roots.lock().ok().and_then(|r| r.first().cloned())
    }

    /// The engine, created on first use.
    fn engine(&self) -> Result<Arc<RunEngine>, String> {
        let mut guard = self
            .engine
            .lock()
            .map_err(|_| "exec engine lock poisoned".to_string())?;
        if let Some(engine) = guard.as_ref() {
            return Ok(Arc::clone(engine));
        }
        let db_path = self
            .db_path
            .lock()
            .ok()
            .and_then(|p| p.clone())
            .unwrap_or_else(|| ministr_api::daemon_data_dir().join("exec_runs.db"));
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create data dir {}: {e}", parent.display()))?;
        }
        let engine = Arc::new(
            RunEngine::new(
                db_path,
                Arc::new(SharedRoots(Arc::clone(&self.roots))),
                RunEngineConfig::default(),
            )
            .map_err(|e| format!("cannot open exec run store: {e}"))?,
        );
        *guard = Some(Arc::clone(&engine));
        Ok(engine)
    }

    /// Execute (or start) a run; the `ministr_run` body.
    pub(crate) async fn run(
        &self,
        params: RunParams,
        session_id: String,
    ) -> Result<RunResponse, String> {
        let engine = self.engine()?;
        let cwd = match params.cwd {
            Some(c) if !c.is_empty() => PathBuf::from(c),
            _ => self.first_root().ok_or_else(|| {
                "no cwd given and no corpus roots are configured for exec".to_string()
            })?,
        };
        let timeout = params
            .timeout_secs
            .map(|s| Duration::from_secs(s.min(MAX_TIMEOUT_SECS)));
        let request = RunRequest {
            command: params.command,
            cwd,
            session_id: Some(session_id),
            corpus_id: None,
            timeout,
        };
        if params.background.unwrap_or(false) {
            let run_id = engine.start(request).map_err(|e| e.to_string())?;
            return Ok(RunResponse {
                run_id,
                status: "running".to_string(),
                exit_code: None,
                duration_ms: None,
                bytes_total: 0,
                capture_truncated: false,
                digest: None,
            });
        }
        let record = engine.run(request).await.map_err(|e| e.to_string())?;
        Ok(RunResponse {
            digest: Some(digest(&record.log)),
            duration_ms: duration_ms(&record),
            run_id: record.run_id,
            status: record.status.as_str().to_string(),
            exit_code: record.exit_code,
            bytes_total: record.bytes_total,
            capture_truncated: record.truncated,
        })
    }

    /// Page or search a run's captured log; the `ministr_run_logs` body.
    pub(crate) fn logs(
        &self,
        params: RunLogsParams,
        session_id: &str,
    ) -> Result<RunLogsResponse, String> {
        let engine = self.engine()?;
        let record = engine
            .get(&params.run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no such run: {}", params.run_id))?;

        // Query mode: search the whole log, don't advance the cursor.
        if let Some(query) = params.query.filter(|q| !q.is_empty()) {
            let needle = query.to_lowercase();
            let matches: Vec<String> = record
                .log
                .lines()
                .filter(|l| l.to_lowercase().contains(&needle))
                .take(200)
                .map(ToString::to_string)
                .collect();
            return Ok(RunLogsResponse {
                run_id: record.run_id,
                status: record.status.as_str().to_string(),
                chunk: matches.join("\n"),
                next_offset: None,
                remaining_bytes: 0,
                matched_lines: Some(matches.len()),
            });
        }

        // Delta mode: next undelivered span for this session, never resent.
        let key = (session_id.to_string(), params.run_id.clone());
        let cursor = params.from_offset.unwrap_or_else(|| {
            self.cursors
                .lock()
                .ok()
                .and_then(|c| c.get(&key).copied())
                .unwrap_or(0)
        });
        let max = params.max_bytes.unwrap_or(DEFAULT_LOG_PAGE_BYTES);
        let (span, next) = next_span(&record.log, cursor, max);
        if let Ok(mut cursors) = self.cursors.lock() {
            cursors.insert(key, next);
        }
        Ok(RunLogsResponse {
            run_id: record.run_id,
            status: record.status.as_str().to_string(),
            chunk: span.to_string(),
            next_offset: Some(next),
            remaining_bytes: record.log.len().saturating_sub(next),
            matched_lines: None,
        })
    }

    /// Status snapshot; the `ministr_run_status` body.
    pub(crate) fn status(&self, run_id: &str) -> Result<RunStatusResponse, String> {
        let engine = self.engine()?;
        let record = engine
            .get(run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no such run: {run_id}"))?;
        Ok(RunStatusResponse {
            run_id: record.run_id.clone(),
            status: record.status.as_str().to_string(),
            exit_code: record.exit_code,
            duration_ms: duration_ms(&record),
            bytes_total: record.bytes_total,
        })
    }

    /// Cancel a running run; the `ministr_run_kill` body.
    pub(crate) fn kill(&self, run_id: &str) -> Result<RunKillResponse, String> {
        let engine = self.engine()?;
        Ok(RunKillResponse {
            run_id: run_id.to_string(),
            killed: engine.cancel(run_id),
        })
    }
}

fn duration_ms(record: &RunRecord) -> Option<i64> {
    record
        .finished_at_ms
        .map(|end| end.saturating_sub(record.started_at_ms))
}

/// Parameters for `ministr_run`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunParams {
    /// Shell command line to execute.
    #[schemars(description = "Shell command line to execute")]
    pub command: String,
    /// Working directory (must be inside an indexed corpus root). Defaults
    /// to the first corpus root.
    #[serde(default)]
    #[schemars(description = "Working directory; defaults to the first corpus root")]
    pub cwd: Option<String>,
    /// Timeout in seconds (default 600, max 3600).
    #[serde(default)]
    #[schemars(description = "Timeout seconds (default 600, max 3600)")]
    pub timeout_secs: Option<u64>,
    /// Run in the background; poll with `ministr_run_status`.
    #[serde(default)]
    #[schemars(description = "Run in background; poll with ministr_run_status")]
    pub background: Option<bool>,
}

/// Parameters for `ministr_run_logs`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunLogsParams {
    /// Run id from `ministr_run`.
    #[schemars(description = "Run id from ministr_run")]
    pub run_id: String,
    /// Substring filter; returns matching lines instead of paging.
    #[serde(default)]
    #[schemars(description = "Substring filter: return matching lines instead of paging")]
    pub query: Option<String>,
    /// Max bytes per page (default 16384).
    #[serde(default)]
    #[schemars(description = "Max bytes per page (default 16384)")]
    pub max_bytes: Option<usize>,
    /// Explicit byte offset (overrides the session cursor).
    #[serde(default)]
    #[schemars(description = "Explicit byte offset (overrides the session cursor)")]
    pub from_offset: Option<usize>,
}

/// Parameters for `ministr_run_status` / `ministr_run_kill`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunIdParams {
    /// Run id from `ministr_run`.
    #[schemars(description = "Run id from ministr_run")]
    pub run_id: String,
}

/// Response for `ministr_run`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RunResponse {
    /// Run id (use with `ministr_run_logs` / `ministr_run_status`).
    pub run_id: String,
    /// Lifecycle state: `running` | `exited` | `killed` | `timed_out`.
    pub status: String,
    /// Exit code (None while running or signal-killed).
    pub exit_code: Option<i32>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: Option<i64>,
    /// Exact bytes the command produced.
    pub bytes_total: u64,
    /// True when the engine's capture guard dropped middle output.
    pub capture_truncated: bool,
    /// Token-lean digest (None for background starts).
    pub digest: Option<RunDigest>,
}

/// Response for `ministr_run_logs`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RunLogsResponse {
    /// Run id.
    pub run_id: String,
    /// Lifecycle state: `running` | `exited` | `killed` | `timed_out`.
    pub status: String,
    /// The log span (delta mode) or matched lines joined (query mode).
    pub chunk: String,
    /// Cursor for the next page (delta mode only).
    pub next_offset: Option<usize>,
    /// Bytes not yet delivered after this page (delta mode only).
    pub remaining_bytes: usize,
    /// Matched line count (query mode only).
    pub matched_lines: Option<usize>,
}

/// Response for `ministr_run_status`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RunStatusResponse {
    /// Run id.
    pub run_id: String,
    /// Lifecycle state: `running` | `exited` | `killed` | `timed_out`.
    pub status: String,
    /// Exit code (None while running or signal-killed).
    pub exit_code: Option<i32>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: Option<i64>,
    /// Exact bytes produced so far (final after exit).
    pub bytes_total: u64,
}

/// Response for `ministr_run_kill`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RunKillResponse {
    /// Run id.
    pub run_id: String,
    /// True when the run was still active and cancellation was requested.
    pub killed: bool,
}
