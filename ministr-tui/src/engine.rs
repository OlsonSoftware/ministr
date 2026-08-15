//! Plumbing between the console and the machine's index engine.
//!
//! The console never speaks a protocol of its own: it holds one
//! [`DaemonClient`] — the same client the CLI and the MCP proxy use —
//! and reduces its answers to the small [`EngineState`] the frame renders.

use std::time::Duration;

use ministr_api::client::DaemonClient;
use ministr_api::corpus::{CorpusInfo, IndexingStatus, IngestionProgressInfo};

use crate::console::{ConsoleModel, Standing, Strip};
use crate::detail::Facts;
use crate::strings;

/// How long to wait for a freshly spawned engine to answer.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the console knows about the engine right now.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineState {
    /// No probe has answered yet.
    Starting,
    /// The engine answered the last probe: the full console reduction.
    Running(ConsoleModel),
    /// The engine did not answer the last probe.
    Unreachable,
}

/// Make sure an engine is up before the console opens, spawning a
/// detached one if none is running — the same handshake the MCP proxy
/// uses, so closing the console never takes the engine down with it.
///
/// Failure is deliberately not fatal: the console opens anyway and the
/// title row reports the machine state honestly.
pub async fn ensure_engine(client: &DaemonClient) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = client.ensure_daemon_spawned(&exe, SPAWN_TIMEOUT).await;
    }
}

/// One status probe, reduced to the state the frame renders: engine
/// version plus one strip per project.
pub async fn probe(client: &DaemonClient) -> EngineState {
    match client.status().await {
        Ok(status) => {
            let mut strips = Vec::with_capacity(status.corpora.len());
            for info in status.corpora {
                strips.push(reduce_strip(client, info).await);
            }
            EngineState::Running(ConsoleModel {
                version: status.version,
                strips,
            })
        }
        Err(_) => EngineState::Unreachable,
    }
}

/// Reduce one project's report to its strip. An idle project's standing
/// needs the cheap counts-only summary poll; if that call fails (a
/// remove racing the probe), the strip reads "up to date" until the
/// next probe corrects it two seconds later.
async fn reduce_strip(client: &DaemonClient, info: CorpusInfo) -> Strip {
    let standing = if info.warming {
        Standing::Warming
    } else {
        match &info.status {
            IndexingStatus::Indexing {
                files_done,
                files_total,
            } => Standing::Building {
                fraction: fraction_of(*files_done, *files_total),
            },
            IndexingStatus::Queued => Standing::Waiting,
            IndexingStatus::Error { .. } => Standing::Failed,
            IndexingStatus::Idle => match client.corpus_freshness_summary(&info.id).await {
                Ok(f) if f.indexing => Standing::Waiting,
                Ok(f) if f.stale + f.new_files + f.missing > 0 => Standing::NeedsUpdate,
                _ => Standing::UpToDate,
            },
        }
    };
    let name = if info.display_name.is_empty() {
        info.id.clone()
    } else {
        info.display_name
    };
    Strip {
        id: info.id,
        name,
        standing,
        files: info.files_indexed,
    }
}

/// A verb the frame asked the engine to run. [`crate::app::App`] queues
/// exactly one; the event loop drains and runs it against the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Fetch the opened project's slower facts (S2).
    OpenDetail {
        /// The engine's identifier for the project.
        id: String,
    },
    /// Rebuild the project's index from scratch.
    Rebuild {
        /// The engine's identifier for the project.
        id: String,
    },
    /// Remove the project from the engine (the index directory stays).
    Remove {
        /// The engine's identifier for the project.
        id: String,
    },
    /// Add a project by path (S3).
    PatchIn {
        /// The path to add.
        path: String,
    },
    /// Replace the opened project's path set.
    SavePaths {
        /// The engine's identifier for the project.
        id: String,
        /// The new path set, blanks already dropped.
        paths: Vec<String>,
    },
}

impl Action {
    /// The word the foot row shows while this verb runs — verbs run on
    /// a background task so the console never freezes, and the word
    /// says so honestly. The quiet facts fetch shows nothing.
    #[must_use]
    pub fn working_word(&self) -> Option<&'static str> {
        match self {
            Self::OpenDetail { .. } => None,
            Self::Rebuild { .. } => Some(strings::WORKING_REBUILD),
            Self::Remove { .. } => Some(strings::WORKING_REMOVE),
            Self::PatchIn { .. } => Some(strings::WORKING_ADD),
            Self::SavePaths { .. } => Some(strings::WORKING_SAVE),
        }
    }
}

/// What a finished verb left behind, applied to the app by
/// [`crate::app::App::absorb_outcome`] when the background task
/// delivers it.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The machine may have changed shape — the caller re-probes at
    /// once when so.
    pub refreshed: bool,
    /// The plain-worded failure to show, when the verb failed.
    pub notice: Option<&'static str>,
    /// The panel that queued the verb is finished (a patch-in landed).
    pub to_console: bool,
    /// Fresh facts for the opened project, when the verb fetched them.
    pub facts: Option<Facts>,
}

impl Outcome {
    /// An outcome that changed nothing and says nothing.
    fn quiet() -> Self {
        Self {
            refreshed: false,
            notice: None,
            to_console: false,
            facts: None,
        }
    }

    /// A plain verb's outcome: refreshed on success, the failure word
    /// otherwise.
    fn finished(ok: bool, failure: &'static str) -> Self {
        Self {
            refreshed: ok,
            notice: (!ok).then_some(failure),
            ..Self::quiet()
        }
    }
}

/// Run one queued verb against the engine. Runs on its own task — a
/// slow answer (a big project registering, a rebuild purging) must
/// never hold up a frame.
pub async fn run(client: &DaemonClient, action: Action) -> Outcome {
    match action {
        Action::OpenDetail { id } => Outcome {
            facts: detail(client, &id).await,
            ..Outcome::quiet()
        },
        Action::Rebuild { id } => Outcome::finished(
            client.reindex_corpus(&id).await.is_ok(),
            strings::NOTICE_REBUILD_FAILED,
        ),
        Action::Remove { id } => Outcome::finished(
            client.unregister_corpus(&id).await.is_ok(),
            strings::NOTICE_REMOVE_FAILED,
        ),
        Action::PatchIn { path } => {
            let ok = client.register_corpus(&[path]).await.is_ok();
            Outcome {
                // Back to the console: the new strip materializes
                // there the moment the next probe reports it.
                to_console: ok,
                ..Outcome::finished(ok, strings::NOTICE_ADD_FAILED)
            }
        }
        Action::SavePaths { id, paths } => {
            let ok = client.update_corpus_paths(&id, &paths).await.is_ok();
            Outcome {
                // The path set changed: bring the opened panel current.
                facts: if ok { detail(client, &id).await } else { None },
                ..Outcome::finished(ok, strings::NOTICE_PATHS_FAILED)
            }
        }
    }
}

/// Fetch the opened project's slower facts: the full path set, section
/// and symbol counts, when the last build finished, and what needs
/// updating. Every wall-clock phrase is rendered here, at fetch time,
/// so drawing stays a pure function of the model. `None` when the
/// engine did not answer — the panel keeps its quiet ellipsis and the
/// next probe retries.
pub async fn detail(client: &DaemonClient, id: &str) -> Option<Facts> {
    let info = client.corpus_status(id).await.ok()?;
    let attention = match client.corpus_freshness_summary(id).await {
        Ok(f) => strings::attention_line(f.stale, f.new_files, f.missing),
        Err(_) => None,
    };
    Some(Facts {
        id: info.id,
        paths: info.paths,
        sections: info.sections_count,
        symbols: info.symbols_count,
        updated: info
            .last_indexed
            .map(|ts| strings::ago_line(age_seconds(ts))),
        attention,
    })
}

/// Seconds elapsed since unix timestamp `ts`, clamped at zero.
fn age_seconds(ts: i64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now.saturating_sub(u64::try_from(ts).unwrap_or(0))
}

/// One project's live build position, from the engine's progress
/// counters.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressTarget {
    /// The engine's identifier for the project — [`Strip::id`].
    pub id: String,
    /// The reported position, `0.0..=1.0`.
    pub fraction: f64,
}

/// Poll the engine's progress counters — the fast poll that feeds the
/// live meters between status probes. `None` when the engine did not
/// answer; the meters simply hold until the next answer.
pub async fn progress(client: &DaemonClient) -> Option<Vec<ProgressTarget>> {
    let infos = client.ingestion_progress().await.ok()?;
    Some(
        infos
            .into_iter()
            .map(|info| ProgressTarget {
                fraction: progress_fraction(&info),
                id: info.corpus_id,
            })
            .collect(),
    )
}

/// A build's overall position: parsed files and generated embeddings
/// counted as one pool of work, so the needle keeps moving through the
/// whole build instead of parking at full while the tail finishes.
/// A finished report reads full regardless of its counters.
fn progress_fraction(info: &IngestionProgressInfo) -> f64 {
    const COMPLETE: u8 = 2;
    if info.status == COMPLETE {
        return 1.0;
    }
    fraction_of(
        info.files_done + info.embeddings_done,
        info.files_total + info.embeddings_total,
    )
}

/// `done / total`, honest at zero.
#[allow(clippy::cast_precision_loss)]
fn fraction_of(done: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        done as f64 / total as f64
    }
}
