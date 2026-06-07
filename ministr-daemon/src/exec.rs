//! exec — daemon-side run engine (the recording substrate for `exec-epic`).
//!
//! Spawns shell commands with piped capture and persists every run as an
//! audit-grade record (command, cwd, env fingerprint, timestamps, exit
//! code, bounded interleaved log) in a `SQLite` database alongside the
//! daemon's other data. The engine is deliberately decoupled from the
//! registry: callers inject a [`RootsProvider`] so the cwd policy follows
//! whatever corpus roots the daemon currently manages.
//!
//! Design notes:
//! - **Capture is bounded** (head + tail): runaway output can never OOM
//!   the daemon, and the head (where compiler errors usually start) plus
//!   the tail (where summaries land) both survive. Total byte/line counts
//!   are always exact even when the middle is dropped.
//! - **Cancellation kills the whole process group** on unix
//!   (`process_group(0)` at spawn + `killpg` on cancel/timeout), so a
//!   `cargo test` that forked children leaves no orphans. On Windows only
//!   the direct child is killed — an honest, documented gap until a job
//!   object implementation lands.
//! - **Persistence mirrors [`crate::persistence`]**: plain `rusqlite`
//!   functions against an explicit db path, no pool. A `running` row is
//!   inserted at spawn and finalized at exit, so a daemon crash leaves a
//!   visible `running` tombstone rather than silence.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Default cap on the preserved log head, in bytes.
pub const DEFAULT_HEAD_CAP: usize = 128 * 1024;
/// Default cap on the preserved log tail, in bytes.
pub const DEFAULT_TAIL_CAP: usize = 128 * 1024;
/// Default wall-clock timeout for a run.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Errors from the run engine.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The requested cwd is outside every allowed corpus root.
    #[error("cwd not permitted by exec policy: {0}")]
    PolicyDenied(String),
    /// The command could not be spawned.
    #[error("failed to spawn command: {0}")]
    Spawn(#[from] std::io::Error),
    /// A persistence operation failed.
    #[error("exec run storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    /// No run with the given id exists.
    #[error("no such run: {0}")]
    NotFound(String),
}

/// Lifecycle state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Spawned and not yet finished (or the daemon died mid-run).
    Running,
    /// Exited on its own; see `exit_code`.
    Exited,
    /// Killed by an explicit cancel.
    Killed,
    /// Killed because the timeout elapsed.
    TimedOut,
}

impl RunStatus {
    /// Stable wire form (matches the serde `snake_case` rename).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Killed => "killed",
            Self::TimedOut => "timed_out",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "exited" => Self::Exited,
            "killed" => Self::Killed,
            "timed_out" => Self::TimedOut,
            _ => Self::Running,
        }
    }
}

/// A request to execute one shell command.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The shell command line (run via `sh -c` / `cmd /C`).
    pub command: String,
    /// Working directory; must resolve under an allowed root.
    pub cwd: PathBuf,
    /// Originating agent session, if known (activity-attribution parity).
    pub session_id: Option<String>,
    /// Corpus this run is associated with, if known.
    pub corpus_id: Option<String>,
    /// Wall-clock timeout; [`DEFAULT_TIMEOUT`] when `None`.
    pub timeout: Option<Duration>,
}

/// One persisted run record — the audit unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// Unique id (`run-<ms>-<seq>`).
    pub run_id: String,
    /// The command line as requested.
    pub command: String,
    /// Canonicalized working directory the command ran in.
    pub cwd: String,
    /// Originating agent session, if provided.
    pub session_id: Option<String>,
    /// Associated corpus, if provided.
    pub corpus_id: Option<String>,
    /// Short fingerprint of the spawn environment (sorted-env SHA-256).
    pub env_fingerprint: String,
    /// Spawn time, unix milliseconds.
    pub started_at_ms: i64,
    /// Finish time, unix milliseconds (`None` while running).
    pub finished_at_ms: Option<i64>,
    /// Process exit code (`None` while running or when signal-killed).
    pub exit_code: Option<i32>,
    /// Lifecycle state.
    pub status: RunStatus,
    /// Captured interleaved stdout+stderr (head + tail when truncated).
    pub log: String,
    /// True when the middle of the log was dropped by the output guard.
    pub truncated: bool,
    /// Exact total bytes the command produced (counted past the caps).
    pub bytes_total: u64,
}

/// Source of the allowed cwd roots — typically the registry's corpus
/// roots. Injected so the engine has no registry dependency.
pub trait RootsProvider: Send + Sync {
    /// Directories under which a run's cwd is permitted.
    fn allowed_roots(&self) -> Vec<PathBuf>;
}

/// Fixed list of allowed roots (tests, simple wiring).
pub struct StaticRoots(pub Vec<PathBuf>);

impl RootsProvider for StaticRoots {
    fn allowed_roots(&self) -> Vec<PathBuf> {
        self.0.clone()
    }
}

/// Filter for [`RunEngine::list`].
#[derive(Debug, Clone, Default)]
pub struct RunsFilter {
    /// Only runs attributed to this session.
    pub session_id: Option<String>,
    /// Only runs associated with this corpus.
    pub corpus_id: Option<String>,
    /// Only runs started strictly after this unix-ms timestamp.
    pub since_ms: Option<i64>,
    /// Maximum records returned (default 50), newest first.
    pub limit: Option<usize>,
}

/// Bounded head+tail capture buffer.
///
/// The first `head_cap` bytes are kept verbatim; everything after flows
/// through a `tail_cap` ring. The exact total byte count is maintained
/// regardless, so the guard never lies about volume.
struct CaptureBuf {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    total: u64,
}

impl CaptureBuf {
    fn new(head_cap: usize, tail_cap: usize) -> Self {
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            head_cap,
            tail_cap,
            total: 0,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len() as u64;
        let mut rest = chunk;
        if self.head.len() < self.head_cap {
            let take = (self.head_cap - self.head.len()).min(rest.len());
            self.head.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
        }
        for &b in rest {
            if self.tail.len() == self.tail_cap {
                self.tail.pop_front();
            }
            self.tail.push_back(b);
        }
    }

    /// Render the captured log; `true` when the middle was dropped.
    fn render(&self) -> (String, bool) {
        let kept = u64::try_from(self.head.len() + self.tail.len()).unwrap_or(u64::MAX);
        let truncated = self.total > kept;
        let mut bytes = self.head.clone();
        if truncated {
            let dropped = self.total.saturating_sub(kept);
            bytes.extend_from_slice(
                format!("\n…[output guard: {dropped} bytes dropped]…\n").as_bytes(),
            );
        }
        bytes.extend(self.tail.iter().copied());
        (String::from_utf8_lossy(&bytes).into_owned(), truncated)
    }
}

/// Engine tuning knobs.
#[derive(Debug, Clone)]
pub struct RunEngineConfig {
    /// Cap on the preserved log head, bytes.
    pub head_cap: usize,
    /// Cap on the preserved log tail, bytes.
    pub tail_cap: usize,
    /// Timeout applied when a request specifies none.
    pub default_timeout: Duration,
}

impl Default for RunEngineConfig {
    fn default() -> Self {
        Self {
            head_cap: DEFAULT_HEAD_CAP,
            tail_cap: DEFAULT_TAIL_CAP,
            default_timeout: DEFAULT_TIMEOUT,
        }
    }
}

struct RunningEntry {
    cancel: CancellationToken,
    /// Live view of the run's captured output (shared with the readers).
    capture: Arc<Mutex<CaptureBuf>>,
}

/// A mid-run view of a running run's output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSnapshot {
    /// Output captured so far (head + tail when over the caps).
    pub log: String,
    /// True when the middle has already been dropped by the guard.
    pub truncated: bool,
    /// Exact bytes produced so far.
    pub bytes_total: u64,
}

/// The daemon-side run engine: spawn + capture + persist + cancel.
pub struct RunEngine {
    db_path: PathBuf,
    roots: Arc<dyn RootsProvider>,
    config: RunEngineConfig,
    /// `Arc` so each run's supervisor task can remove its own entry
    /// after the final DB write without borrowing the engine.
    running: Arc<Mutex<HashMap<String, RunningEntry>>>,
    seq: AtomicU64,
}

impl RunEngine {
    /// Create an engine persisting to `db_path`, with cwd policy from
    /// `roots`.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::Storage`] if the runs table cannot be created.
    pub fn new(
        db_path: impl Into<PathBuf>,
        roots: Arc<dyn RootsProvider>,
        config: RunEngineConfig,
    ) -> Result<Self, RunError> {
        let db_path = db_path.into();
        let conn = rusqlite::Connection::open(&db_path)?;
        store::ensure_tables(&conn)?;
        Ok(Self {
            db_path,
            roots,
            config,
            running: Arc::new(Mutex::new(HashMap::new())),
            seq: AtomicU64::new(0),
        })
    }

    /// Execute a command to completion and return its finished record.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::PolicyDenied`] when the cwd is outside every
    /// allowed root, [`RunError::Spawn`] when the process cannot start,
    /// or [`RunError::Storage`] on a persistence failure.
    pub async fn run(&self, req: RunRequest) -> Result<RunRecord, RunError> {
        let run_id = self.start(req)?;
        loop {
            // The spawn task removes the running entry strictly after the
            // final DB update, so absent-from-map ⇒ the record is final.
            let active = self.running.lock().contains_key(&run_id);
            if !active {
                return self.get(&run_id)?.ok_or_else(|| RunError::NotFound(run_id));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Spawn a command in the background; returns its run id immediately.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::PolicyDenied`] when the cwd is outside every
    /// allowed root, [`RunError::Spawn`] when the process cannot start,
    /// or [`RunError::Storage`] on a persistence failure.
    pub fn start(&self, req: RunRequest) -> Result<String, RunError> {
        let cwd = check_cwd(&req.cwd, &self.roots.allowed_roots())?;
        let run_id = format!(
            "run-{}-{}",
            now_ms(),
            self.seq.fetch_add(1, Ordering::Relaxed)
        );
        let timeout = req.timeout.unwrap_or(self.config.default_timeout);

        let mut child = spawn_child(&req.command, &cwd)?;
        let record = RunRecord {
            run_id: run_id.clone(),
            command: req.command,
            cwd: cwd.to_string_lossy().into_owned(),
            session_id: req.session_id,
            corpus_id: req.corpus_id,
            env_fingerprint: env_fingerprint(),
            started_at_ms: now_ms(),
            finished_at_ms: None,
            exit_code: None,
            status: RunStatus::Running,
            log: String::new(),
            truncated: false,
            bytes_total: 0,
        };
        {
            let conn = rusqlite::Connection::open(&self.db_path)?;
            store::insert_running(&conn, &record)?;
        }

        let cancel = CancellationToken::new();
        let capture = Arc::new(Mutex::new(CaptureBuf::new(
            self.config.head_cap,
            self.config.tail_cap,
        )));
        // The capture handle lives in the running map so callers can
        // render a LIVE snapshot of a run's output mid-flight.
        self.running.lock().insert(
            run_id.clone(),
            RunningEntry {
                cancel: cancel.clone(),
                capture: Arc::clone(&capture),
            },
        );

        let mut readers = tokio::task::JoinSet::new();
        if let Some(stdout) = child.stdout.take() {
            readers.spawn(read_into(stdout, Arc::clone(&capture)));
        }
        if let Some(stderr) = child.stderr.take() {
            readers.spawn(read_into(stderr, Arc::clone(&capture)));
        }

        self.spawn_supervisor(run_id.clone(), child, readers, capture, cancel, timeout);
        Ok(run_id)
    }

    /// Detached per-run supervisor: waits for exit / cancel / timeout,
    /// kills the process group when needed, drains the readers, persists
    /// the final record, and only then removes the run from the running
    /// map (so absent-from-map ⇒ record is final).
    fn spawn_supervisor(
        &self,
        run_id: String,
        mut child: tokio::process::Child,
        mut readers: tokio::task::JoinSet<()>,
        capture: Arc<Mutex<CaptureBuf>>,
        cancel: CancellationToken,
        timeout: Duration,
    ) {
        let pgid = child.id().and_then(|id| i32::try_from(id).ok());
        let db_path = self.db_path.clone();
        let running = Arc::clone(&self.running);
        tokio::spawn(async move {
            let (status, exit_code) = tokio::select! {
                res = child.wait() => {
                    let code = res.ok().and_then(|s| s.code());
                    (RunStatus::Exited, code)
                }
                () = cancel.cancelled() => {
                    kill_group(&mut child, pgid).await;
                    (RunStatus::Killed, None)
                }
                () = tokio::time::sleep(timeout) => {
                    kill_group(&mut child, pgid).await;
                    (RunStatus::TimedOut, None)
                }
            };
            // Readers end on pipe EOF (which group-kill guarantees).
            while readers.join_next().await.is_some() {}
            let (log, truncated, bytes_total) = {
                let buf = capture.lock();
                let (log, truncated) = buf.render();
                (log, truncated, buf.total)
            };
            let finished = store::Finish {
                run_id: run_id.clone(),
                status,
                exit_code,
                finished_at_ms: now_ms(),
                log,
                truncated,
                bytes_total,
            };
            let persisted = tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open(&db_path)?;
                store::finish(&conn, &finished)
            })
            .await;
            match persisted {
                Ok(Ok(())) => debug!(run_id = %run_id, ?status, "exec run finished"),
                Ok(Err(e)) => warn!(run_id = %run_id, error = %e, "exec run finish persist failed"),
                Err(e) => warn!(run_id = %run_id, error = %e, "exec run finish task failed"),
            }
            // Remove from the running map only after the final DB write,
            // so `run()`'s absent-from-map check is a completion barrier.
            running.lock().remove(&run_id);
        });
    }

    /// Render a live snapshot of a RUNNING run's output.
    ///
    /// Returns `None` once the run has finished (the supervisor removes
    /// the entry after the final DB write — read the persisted record
    /// via [`Self::get`] instead).
    #[must_use]
    pub fn live_snapshot(&self, run_id: &str) -> Option<LiveSnapshot> {
        let map = self.running.lock();
        let entry = map.get(run_id)?;
        let buf = entry.capture.lock();
        let (log, truncated) = buf.render();
        Some(LiveSnapshot {
            log,
            truncated,
            bytes_total: buf.total,
        })
    }

    /// Request cancellation of a running run.
    ///
    /// Returns `true` when the run was still active. The record is
    /// finalized (status `killed`) asynchronously by the supervisor.
    pub fn cancel(&self, run_id: &str) -> bool {
        let map = self.running.lock();
        if let Some(entry) = map.get(run_id) {
            entry.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Fetch one run record by id.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::Storage`] on a database failure.
    pub fn get(&self, run_id: &str) -> Result<Option<RunRecord>, RunError> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        Ok(store::get(&conn, run_id)?)
    }

    /// List run records, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`RunError::Storage`] on a database failure.
    pub fn list(&self, filter: &RunsFilter) -> Result<Vec<RunRecord>, RunError> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        Ok(store::list(&conn, filter)?)
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX)
}

/// Short fingerprint of the current process environment (the environment
/// child runs inherit): SHA-256 over sorted `k=v` pairs, first 16 hex.
fn env_fingerprint() -> String {
    let mut pairs: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort();
    let mut hasher = Sha256::new();
    for p in &pairs {
        hasher.update(p.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    }
}

/// Build + spawn the piped child for a run.
fn spawn_child(command: &str, cwd: &Path) -> Result<tokio::process::Child, RunError> {
    let mut cmd = shell_command(command);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        // New process group so cancel/timeout can kill descendants.
        cmd.process_group(0);
    }
    Ok(cmd.spawn()?)
}

/// Validate and canonicalize a requested cwd against the allowed roots.
fn check_cwd(cwd: &Path, roots: &[PathBuf]) -> Result<PathBuf, RunError> {
    let canon = cwd
        .canonicalize()
        .map_err(|e| RunError::PolicyDenied(format!("{}: {e}", cwd.display())))?;
    let permitted = roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|root_canon| canon.starts_with(&root_canon))
    });
    if permitted {
        Ok(canon)
    } else {
        Err(RunError::PolicyDenied(format!(
            "{} is outside every allowed root",
            canon.display()
        )))
    }
}

async fn read_into(
    mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    capture: Arc<Mutex<CaptureBuf>>,
) {
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => capture.lock().push(&buf[..n]),
        }
    }
}

/// Kill the child's whole process group (unix) or the child (windows),
/// then reap it.
async fn kill_group(child: &mut tokio::process::Child, pgid: Option<i32>) {
    #[cfg(unix)]
    if let Some(pgid) = pgid {
        // `kill -9 -<pgid>`: a negative pid operand targets the whole
        // process group. The external `kill` binary keeps the workspace
        // unsafe-free (the lint denies a direct `libc::killpg`); failure
        // is tolerated — the group may already be gone.
        let _ = tokio::process::Command::new("kill")
            .arg("-9")
            .arg(format!("-{pgid}"))
            .status()
            .await;
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Lazily-initialized engine holder for hosts (the daemon's `AppState`).
///
/// Bundles the engine slot, a late-bound shared roots list (refreshed
/// from the corpus registry by the exec route handlers before each
/// spawn), and an optional db-path override (tests / custom data dirs).
/// One `Default` construction per host; `Clone`-shared via `Arc`.
#[derive(Default)]
pub struct EngineCell {
    engine: Mutex<Option<Arc<RunEngine>>>,
    roots: Arc<Mutex<Vec<PathBuf>>>,
    db_path: Mutex<Option<PathBuf>>,
}

impl EngineCell {
    /// Replace the allowed cwd roots the engine validates against.
    pub fn set_roots(&self, roots: Vec<PathBuf>) {
        *self.roots.lock() = roots;
    }

    /// Current allowed roots (the first doubles as the default cwd).
    #[must_use]
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.lock().clone()
    }

    /// Override the run-store path (before the first engine use).
    pub fn set_db_path(&self, path: PathBuf) {
        *self.db_path.lock() = Some(path);
    }

    /// The engine, created on first use (db defaults to
    /// `~/.ministr/exec_runs.db`).
    ///
    /// # Errors
    ///
    /// Returns [`RunError::Storage`] if the run store cannot be opened.
    pub fn engine(&self) -> Result<Arc<RunEngine>, RunError> {
        let mut guard = self.engine.lock();
        if let Some(engine) = guard.as_ref() {
            return Ok(Arc::clone(engine));
        }
        let db_path = self
            .db_path
            .lock()
            .clone()
            .unwrap_or_else(|| ministr_api::daemon_data_dir().join("exec_runs.db"));
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let engine = Arc::new(RunEngine::new(
            db_path,
            Arc::new(SharedRootsList(Arc::clone(&self.roots))),
            RunEngineConfig::default(),
        )?);
        *guard = Some(Arc::clone(&engine));
        Ok(engine)
    }
}

/// [`RootsProvider`] over the cell's late-bound roots list.
struct SharedRootsList(Arc<Mutex<Vec<PathBuf>>>);

impl RootsProvider for SharedRootsList {
    fn allowed_roots(&self) -> Vec<PathBuf> {
        self.0.lock().clone()
    }
}

/// Persistence for run records (mirrors [`crate::persistence`]'s shape).
mod store {
    use super::{RunRecord, RunStatus, RunsFilter};

    /// Finalization payload for a finished run.
    pub(super) struct Finish {
        pub run_id: String,
        pub status: RunStatus,
        pub exit_code: Option<i32>,
        pub finished_at_ms: i64,
        pub log: String,
        pub truncated: bool,
        pub bytes_total: u64,
    }

    pub(super) fn ensure_tables(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS exec_runs (
                run_id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                session_id TEXT,
                corpus_id TEXT,
                env_fingerprint TEXT NOT NULL,
                started_at_ms INTEGER NOT NULL,
                finished_at_ms INTEGER,
                exit_code INTEGER,
                status TEXT NOT NULL,
                log TEXT NOT NULL DEFAULT '',
                truncated INTEGER NOT NULL DEFAULT 0,
                bytes_total INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_exec_runs_session
                ON exec_runs(session_id);
            CREATE INDEX IF NOT EXISTS idx_exec_runs_started
                ON exec_runs(started_at_ms);",
        )
    }

    pub(super) fn insert_running(
        conn: &rusqlite::Connection,
        r: &RunRecord,
    ) -> Result<(), rusqlite::Error> {
        conn.execute(
            "INSERT INTO exec_runs
                (run_id, command, cwd, session_id, corpus_id,
                 env_fingerprint, started_at_ms, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                r.run_id,
                r.command,
                r.cwd,
                r.session_id,
                r.corpus_id,
                r.env_fingerprint,
                r.started_at_ms,
                r.status.as_str(),
            ],
        )?;
        Ok(())
    }

    pub(super) fn finish(conn: &rusqlite::Connection, f: &Finish) -> Result<(), rusqlite::Error> {
        conn.execute(
            "UPDATE exec_runs SET
                finished_at_ms = ?2, exit_code = ?3, status = ?4,
                log = ?5, truncated = ?6, bytes_total = ?7
             WHERE run_id = ?1",
            rusqlite::params![
                f.run_id,
                f.finished_at_ms,
                f.exit_code,
                f.status.as_str(),
                f.log,
                f.truncated,
                i64::try_from(f.bytes_total).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    fn row_to_record(row: &rusqlite::Row<'_>) -> Result<RunRecord, rusqlite::Error> {
        let status: String = row.get("status")?;
        let bytes_total: i64 = row.get("bytes_total")?;
        Ok(RunRecord {
            run_id: row.get("run_id")?,
            command: row.get("command")?,
            cwd: row.get("cwd")?,
            session_id: row.get("session_id")?,
            corpus_id: row.get("corpus_id")?,
            env_fingerprint: row.get("env_fingerprint")?,
            started_at_ms: row.get("started_at_ms")?,
            finished_at_ms: row.get("finished_at_ms")?,
            exit_code: row.get("exit_code")?,
            status: RunStatus::parse(&status),
            log: row.get("log")?,
            truncated: row.get("truncated")?,
            bytes_total: u64::try_from(bytes_total).unwrap_or_default(),
        })
    }

    pub(super) fn get(
        conn: &rusqlite::Connection,
        run_id: &str,
    ) -> Result<Option<RunRecord>, rusqlite::Error> {
        let mut stmt = conn.prepare("SELECT * FROM exec_runs WHERE run_id = ?1")?;
        let mut rows = stmt.query_map([run_id], row_to_record)?;
        rows.next().transpose()
    }

    pub(super) fn list(
        conn: &rusqlite::Connection,
        filter: &RunsFilter,
    ) -> Result<Vec<RunRecord>, rusqlite::Error> {
        use std::fmt::Write as _;
        let mut sql = String::from("SELECT * FROM exec_runs WHERE 1=1");
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(session) = &filter.session_id {
            params.push(Box::new(session.clone()));
            let _ = write!(sql, " AND session_id = ?{}", params.len());
        }
        if let Some(corpus) = &filter.corpus_id {
            params.push(Box::new(corpus.clone()));
            let _ = write!(sql, " AND corpus_id = ?{}", params.len());
        }
        if let Some(since) = filter.since_ms {
            params.push(Box::new(since));
            let _ = write!(sql, " AND started_at_ms > ?{}", params.len());
        }
        sql.push_str(" ORDER BY started_at_ms DESC, run_id DESC");
        let limit = filter.limit.unwrap_or(50);
        params.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));
        let _ = write!(sql, " LIMIT ?{}", params.len());

        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();
        let rows = stmt.query_map(refs.as_slice(), row_to_record)?;
        rows.collect()
    }
}
