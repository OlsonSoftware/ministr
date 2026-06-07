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
/// How many `exec-runs/` run reports the corpus keeps by default.
const DEFAULT_RUN_RETENTION: usize = 50;
/// Source-path prefix identifying ingested run reports in the corpus.
pub(crate) const RUN_REPORT_PREFIX: &str = "exec-runs/";

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
    /// Run-report retention override; `None` = [`DEFAULT_RUN_RETENTION`].
    retention: Arc<std::sync::Mutex<Option<usize>>>,
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
    ///
    /// Finished foreground runs also yield a [`RunIngest`] report so the
    /// caller can index the run into the corpus (run-log intelligence);
    /// background starts yield `None`.
    pub(crate) async fn run(
        &self,
        params: RunParams,
        session_id: String,
    ) -> Result<(RunResponse, Option<RunIngest>), String> {
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
            return Ok((
                RunResponse {
                    run_id,
                    status: "running".to_string(),
                    exit_code: None,
                    duration_ms: None,
                    bytes_total: 0,
                    capture_truncated: false,
                    digest: None,
                },
                None,
            ));
        }
        let record = engine.run(request).await.map_err(|e| e.to_string())?;
        let report = digest(&record.log);
        let ingest = run_report(&record, &report);
        Ok((
            RunResponse {
                digest: Some(report),
                duration_ms: duration_ms(&record),
                run_id: record.run_id,
                status: record.status.as_str().to_string(),
                exit_code: record.exit_code,
                bytes_total: record.bytes_total,
                capture_truncated: record.truncated,
            },
            Some(ingest),
        ))
    }

    /// Effective run-report retention cap (how many `exec-runs/` reports
    /// the corpus keeps).
    pub(crate) fn retention(&self) -> usize {
        self.retention
            .lock()
            .ok()
            .and_then(|r| *r)
            .unwrap_or(DEFAULT_RUN_RETENTION)
    }

    /// Override the run-report retention cap (hosts/tests).
    pub(crate) fn set_retention(&self, cap: usize) {
        if let Ok(mut guard) = self.retention.lock() {
            *guard = Some(cap);
        }
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

/// A finished run rendered for corpus ingestion (run-log intelligence).
pub(crate) struct RunIngest {
    /// Synthetic source path (`exec-runs/<run_id>.md`) — doubles as the
    /// document id and the retention-sweep ordering key (run ids embed
    /// their spawn timestamp).
    pub source_path: String,
    /// The run report as markdown: identity heading, deduped diagnostics
    /// (the digest's `N× line` collapse), and the head/tail window.
    pub markdown: String,
}

/// Render a finished run as a small markdown report.
///
/// The DIGEST is what gets indexed, not the raw log: diagnostics are
/// already deduped with occurrence counts and the window is bounded, so
/// a pathological run can never bloat the corpus. The heading carries
/// command + status so "what failed last time we ran X?" surveys rank it.
fn run_report(record: &RunRecord, digest: &RunDigest) -> RunIngest {
    use std::fmt::Write as _;
    let mut md = String::new();
    let status = record.status.as_str();
    let exit = record
        .exit_code
        .map_or_else(|| "none".to_string(), |c| c.to_string());
    let _ = writeln!(md, "# Run {status} (exit {exit}): {}", record.command);
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "Recorded shell run `{}` in `{}` (session {}). {} lines, {} bytes total.",
        record.run_id,
        record.cwd,
        record.session_id.as_deref().unwrap_or("unknown"),
        digest.lines_total,
        record.bytes_total,
    );
    if !digest.diagnostics.is_empty() {
        let _ = writeln!(md);
        let _ = writeln!(md, "## Diagnostics");
        let _ = writeln!(md);
        for line in &digest.diagnostics {
            let _ = writeln!(md, "- {line}");
        }
    }
    if !digest.window.is_empty() {
        let _ = writeln!(md);
        let _ = writeln!(md, "## Output");
        let _ = writeln!(md);
        let _ = writeln!(md, "```text");
        let _ = writeln!(md, "{}", digest.window);
        let _ = writeln!(md, "```");
    }
    RunIngest {
        source_path: format!("{RUN_REPORT_PREFIX}{}.md", record.run_id),
        markdown: md,
    }
}

impl crate::server::MinistrServer {
    /// The daemon client to forward exec tool calls through, when this
    /// server runs in single-corpus daemon-backend mode.
    ///
    /// `Some` only for [`crate::backend::Backend::Daemon`] — the
    /// exec engine is machine-wide (one per daemon), so forwarding makes
    /// the app's Run Console see agent runs live and able to kill them.
    /// Local / daemon-multi / cloud keep the in-process engine.
    pub(crate) fn daemon_exec_client(
        &self,
    ) -> Option<&std::sync::Arc<ministr_api::client::DaemonClient>> {
        match &self.backend {
            crate::backend::Backend::Daemon(b) => Some(b.client()),
            _ => None,
        }
    }

    /// Index a finished run's report into the corpus, then sweep old run
    /// reports past the retention cap.
    ///
    /// Best-effort by design: this runs on the `ministr_run` response
    /// path and must NEVER fail the run result — a missing storage /
    /// embedder / index (daemon-forward mode, or no `with_runtime_ingest`
    /// wiring) just skips, and ingest/sweep errors are logged.
    pub(crate) async fn ingest_finished_run(&self, ingest: RunIngest) {
        use ministr_core::storage::Storage as _;
        let (Some(storage), Some(embedder), Some(index)) =
            (&self.storage, &self.embedder, &self.index)
        else {
            tracing::debug!(
                source = %ingest.source_path,
                "run-report ingest skipped: no local storage/embedder/index"
            );
            return;
        };

        if let Err(e) = self
            .ingestion_pipeline
            .ingest_content_with_embeddings(
                &ingest.source_path,
                &ingest.markdown,
                ministr_core::parser::ParserKind::Markdown,
                storage.as_ref(),
                embedder.as_ref(),
                index.as_ref(),
            )
            .await
        {
            tracing::warn!(source = %ingest.source_path, error = %e, "run-report ingest failed");
            return;
        }

        // Retention sweep: keep only the newest N run reports. Run ids
        // embed their spawn timestamp, so lexicographic source_path order
        // is chronological.
        let cap = self.exec.retention();
        let docs = match storage.as_ref().list_documents().await {
            Ok(docs) => docs,
            Err(e) => {
                tracing::warn!(error = %e, "run-report retention sweep: list failed");
                return;
            }
        };
        let mut run_docs: Vec<_> = docs
            .into_iter()
            .filter(|d| d.source_path.starts_with(RUN_REPORT_PREFIX))
            .collect();
        if run_docs.len() <= cap {
            return;
        }
        run_docs.sort_by(|a, b| a.source_path.cmp(&b.source_path));
        let excess = run_docs.len() - cap;
        for doc in run_docs.into_iter().take(excess) {
            if let Err(e) = self
                .ingestion_pipeline
                .remove_document_with_embeddings(&doc.id, storage.as_ref(), index.as_ref())
                .await
            {
                tracing::warn!(doc_id = %doc.id, error = %e, "run-report retention sweep: remove failed");
            }
        }
    }
}

// ── Daemon-forward bodies (exec-mcp-daemon-forward) ─────────────────────────
//
// In daemon-backend mode the tool handlers route here so every run lands
// in the ONE daemon-hosted engine — the shared engine the Tauri app's
// Run Console reads and can kill. The digest is still shaped client-side
// (run_digest over the forwarded record's log), so the agent-facing
// response is identical to the local path. Run-log intelligence ingest
// stays local-only (the daemon owns its own storage).

/// Map a forwarded `ExecRun` to the `ministr_run` digest response.
fn wire_to_response(run: ministr_api::exec::ExecRun) -> RunResponse {
    let digest = (run.status != "running").then(|| digest(&run.log));
    let duration_ms = run
        .finished_at_ms
        .map(|end| end.saturating_sub(run.started_at_ms));
    RunResponse {
        digest,
        duration_ms,
        run_id: run.run_id,
        status: run.status,
        exit_code: run.exit_code,
        bytes_total: run.bytes_total,
        capture_truncated: run.truncated,
    }
}

/// `ministr_run` over a daemon backend.
pub(crate) async fn forward_run(
    client: &ministr_api::client::DaemonClient,
    params: RunParams,
    session_id: String,
) -> Result<RunResponse, String> {
    let req = ministr_api::exec::StartExecRun {
        command: params.command,
        cwd: params.cwd.filter(|c| !c.is_empty()),
        session_id: Some(session_id),
        timeout_secs: params.timeout_secs.map(|s| s.min(MAX_TIMEOUT_SECS)),
        background: params.background.unwrap_or(false),
    };
    client
        .exec_start(&req)
        .await
        .map(wire_to_response)
        .map_err(|e| e.to_string())
}

/// `ministr_run_logs` over a daemon backend.
///
/// The daemon serves the live snapshot for running runs and the
/// persisted log otherwise; cursor-based delta paging stays a local-mode
/// nicety (forwarded logs return the whole available chunk).
pub(crate) async fn forward_logs(
    client: &ministr_api::client::DaemonClient,
    params: RunLogsParams,
) -> Result<RunLogsResponse, String> {
    if let Some(query) = params.query.filter(|q| !q.is_empty()) {
        let logs = client
            .exec_run_logs(&params.run_id)
            .await
            .map_err(|e| e.to_string())?;
        let needle = query.to_lowercase();
        let matches: Vec<&str> = logs
            .log
            .lines()
            .filter(|l| l.to_lowercase().contains(&needle))
            .take(200)
            .collect();
        return Ok(RunLogsResponse {
            run_id: logs.run_id,
            status: logs.status,
            chunk: matches.join("\n"),
            next_offset: None,
            remaining_bytes: 0,
            matched_lines: Some(matches.len()),
        });
    }
    let logs = client
        .exec_run_logs(&params.run_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RunLogsResponse {
        run_id: logs.run_id,
        status: logs.status,
        chunk: logs.log,
        next_offset: None,
        remaining_bytes: 0,
        matched_lines: None,
    })
}

/// `ministr_run_status` over a daemon backend.
pub(crate) async fn forward_status(
    client: &ministr_api::client::DaemonClient,
    run_id: &str,
) -> Result<RunStatusResponse, String> {
    let run = client.exec_run(run_id).await.map_err(|e| e.to_string())?;
    let duration_ms = run
        .finished_at_ms
        .map(|end| end.saturating_sub(run.started_at_ms));
    Ok(RunStatusResponse {
        run_id: run.run_id,
        status: run.status,
        exit_code: run.exit_code,
        duration_ms,
        bytes_total: run.bytes_total,
    })
}

/// `ministr_run_kill` over a daemon backend.
pub(crate) async fn forward_kill(
    client: &ministr_api::client::DaemonClient,
    run_id: &str,
) -> Result<RunKillResponse, String> {
    let resp = client.exec_kill(run_id).await.map_err(|e| e.to_string())?;
    Ok(RunKillResponse {
        run_id: resp.run_id,
        killed: resp.killed,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(status: &str, exit: Option<i32>, log: &str) -> ministr_api::exec::ExecRun {
        ministr_api::exec::ExecRun {
            run_id: "run-1-0".to_string(),
            command: "cargo test".to_string(),
            cwd: "/work".to_string(),
            session_id: Some("s".to_string()),
            corpus_id: None,
            env_fingerprint: "abc".to_string(),
            started_at_ms: 1_000,
            finished_at_ms: if status == "running" {
                None
            } else {
                Some(1_900)
            },
            exit_code: exit,
            status: status.to_string(),
            log: log.to_string(),
            truncated: false,
            bytes_total: log.len() as u64,
        }
    }

    /// A finished forwarded run yields a digest + duration; the digest
    /// carries the same diagnostics the local path would (`run_digest` is
    /// the shared shaper, so daemon mode is response-identical).
    #[test]
    fn wire_to_response_shapes_a_finished_run_with_digest() {
        let r = wire_to_response(wire(
            "exited",
            Some(1),
            "compiling\nerror[E0308]: mismatched types\n",
        ));
        assert_eq!(r.status, "exited");
        assert_eq!(r.exit_code, Some(1));
        assert_eq!(r.duration_ms, Some(900));
        let digest = r.digest.expect("finished run carries a digest");
        assert!(
            digest
                .diagnostics
                .iter()
                .any(|l| l.contains("error[E0308]")),
            "the forwarded log must digest like the local path"
        );
    }

    /// A still-running forwarded run yields NO digest + no duration — the
    /// daemon's persisted log is empty mid-run; the live tail comes from
    /// `ministr_run_logs`, not the run response.
    #[test]
    fn wire_to_response_omits_digest_for_a_running_run() {
        let r = wire_to_response(wire("running", None, ""));
        assert_eq!(r.status, "running");
        assert!(r.digest.is_none());
        assert!(r.duration_ms.is_none());
    }
}
