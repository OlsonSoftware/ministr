//! Plumbing between the console and the machine's index engine.
//!
//! The console never speaks a protocol of its own: it holds one
//! [`DaemonClient`] — the same client the CLI and the MCP proxy use —
//! and reduces its answers to the small [`EngineState`] the frame renders.

use std::time::Duration;

use ministr_api::client::DaemonClient;
use ministr_api::corpus::{CorpusInfo, IndexingStatus};

use crate::console::{ConsoleModel, Standing, Strip};

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
        info.id
    } else {
        info.display_name
    };
    Strip {
        name,
        standing,
        files: info.files_indexed,
    }
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
