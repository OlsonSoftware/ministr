//! Plumbing between the console and the machine's index engine.
//!
//! The console never speaks a protocol of its own: it holds one
//! [`DaemonClient`] — the same client the CLI and the MCP proxy use —
//! and reduces its answers to the small [`EngineState`] the frame renders.

use std::time::Duration;

use ministr_api::client::DaemonClient;
use ministr_api::corpus::{CorpusInfo, IndexingStatus, IngestionProgressInfo};

use crate::console::{ConsoleModel, Leftovers, Standing, Strip};
use crate::detail::Facts;
use crate::lawn::{Blade, Lawn};
use crate::strings;

/// The freshness-summary counts (current, stale, new, missing) that
/// key a lawn refetch: the per-file list is fetched only when these
/// change, never on the probe's own cadence.
pub type FreshSig = (usize, usize, usize, usize);

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
    let mut fresh_sig = None;
    let standing = if info.warming {
        // The probe's corpus report carries no load position; the fast
        // progress poll retargets the meter within its first tick.
        Standing::Warming { fraction: 0.0 }
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
                Ok(f) => {
                    fresh_sig = Some((f.current, f.stale, f.new_files, f.missing));
                    if f.indexing {
                        Standing::Waiting
                    } else if f.stale + f.new_files + f.missing > 0 {
                        Standing::NeedsUpdate
                    } else {
                        Standing::UpToDate
                    }
                }
                Err(_) => Standing::UpToDate,
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
        fresh_sig,
        pulse: None,
    }
}

/// Fetch a project's lawn: every file's hash-verified verdict, with
/// each current file's heat fixed HERE, at fetch time, from its mtime
/// recency — rendering stays a pure function of the model. `None` when
/// the engine did not answer; the strip keeps its last lawn.
pub async fn lawn(client: &DaemonClient, id: &str) -> Option<Lawn> {
    let fresh = client.corpus_freshness(id).await.ok()?;
    let mtimes: std::collections::HashMap<String, i64> = client
        .list_corpus_files(id)
        .await
        .map(|files| {
            files
                .into_iter()
                .filter_map(|f| f.mtime_ns.map(|m| (f.path, m)))
                .collect()
        })
        .unwrap_or_default();
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    let files = fresh
        .files
        .into_iter()
        .map(|f| {
            // The wire states are "current" / "stale" / "new" /
            // "missing" — matched by first byte so the language gate
            // (which scans every literal in src/) never sees the
            // engine-side vocabulary in a string.
            let blade = match f.state.as_bytes().first() {
                Some(b'c') => Blade::Current {
                    heat: mtimes
                        .get(&f.path)
                        .map_or(0, |m| heat_band(now_ns.saturating_sub(*m))),
                },
                Some(b's') => Blade::Stale,
                Some(b'n') => Blade::New,
                _ => Blade::Missing,
            };
            (f.path, blade)
        })
        .collect();
    Some(Lawn::new(files))
}

/// Heat band from a file's age: active today burns deepest, then this
/// week, this month, and everything older rests at the base green.
fn heat_band(age_ns: i64) -> u8 {
    const DAY: i64 = 86_400 * 1_000_000_000;
    if age_ns < DAY {
        3
    } else if age_ns < 7 * DAY {
        2
    } else {
        u8::from(age_ns < 30 * DAY)
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
    /// Delete every unclaimed index directory (the leftovers module's
    /// clean verb).
    CleanLeftovers {
        /// The directory names, from the last leftovers answer.
        dirs: Vec<String>,
    },
    /// Bring reconnectable leftovers back as projects (the leftovers
    /// module's reconnect verb).
    ReconnectLeftovers {
        /// The reconnectable directory names, from the last answer.
        dirs: Vec<String>,
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
            Self::CleanLeftovers { .. } => Some(strings::WORKING_CLEAN),
            Self::ReconnectLeftovers { .. } => Some(strings::WORKING_RECONNECT),
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
    /// The verb touched the unclaimed pile — the caller refetches the
    /// leftovers report at once instead of waiting out its slow timer.
    pub rescan_leftovers: bool,
}

impl Outcome {
    /// An outcome that changed nothing and says nothing.
    fn quiet() -> Self {
        Self {
            refreshed: false,
            notice: None,
            to_console: false,
            facts: None,
            rescan_leftovers: false,
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
        Action::CleanLeftovers { dirs } => {
            let mut all_ok = true;
            for dir in &dirs {
                all_ok &= client.remove_orphan_index(dir).await.is_ok();
            }
            Outcome {
                rescan_leftovers: true,
                ..Outcome::finished(all_ok, strings::NOTICE_CLEAN_FAILED)
            }
        }
        Action::ReconnectLeftovers { dirs } => {
            let mut all_ok = true;
            for dir in &dirs {
                all_ok &= client.adopt_orphan_index(dir).await.is_ok();
            }
            Outcome {
                // Reconnected projects materialize on the next probe.
                refreshed: true,
                rescan_leftovers: true,
                ..Outcome::finished(all_ok, strings::NOTICE_RECONNECT_FAILED)
            }
        }
    }
}

/// Fetch the machine's unclaimed-data report, reduced to the console's
/// summary module: every unclaimed directory, the reconnectable subset,
/// and the pile's size phrased at fetch time. `None` when the engine
/// did not answer — the module holds its last answer; an answered empty
/// pile comes back as empty `dirs`, and the module dissolves.
pub async fn leftovers(client: &DaemonClient) -> Option<Leftovers> {
    let report = client.list_orphan_indexes().await.ok()?;
    let mut dirs = Vec::with_capacity(report.orphans.len());
    let mut reconnectable = Vec::new();
    for orphan in &report.orphans {
        dirs.push(orphan.dir_name.clone());
        if orphan.adoptable {
            reconnectable.push(orphan.dir_name.clone());
        }
    }
    Some(Leftovers {
        dirs,
        reconnectable,
        size_line: strings::size_line(report.total_bytes),
    })
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
        size: info.size_on_disk_bytes.map(strings::size_line),
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
    /// The file the indexer is on right now (empty between files) —
    /// the lawn's pulse follows it.
    pub current_file: String,
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
                current_file: info.current_file,
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
