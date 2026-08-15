//! Plumbing between the console and the machine's index engine.
//!
//! The console never speaks a protocol of its own: it holds one
//! [`DaemonClient`] — the same client the CLI and the MCP proxy use —
//! and reduces its answers to the small [`EngineState`] the frame renders.

use std::time::Duration;

use ministr_api::client::DaemonClient;

/// How long to wait for a freshly spawned engine to answer.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the console knows about the engine right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    /// No probe has answered yet.
    Starting,
    /// The engine answered the last probe.
    Running {
        /// Engine version, shown in the title row.
        version: String,
        /// Number of projects the engine holds.
        projects: usize,
    },
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

/// One status probe, reduced to the state the frame renders.
pub async fn probe(client: &DaemonClient) -> EngineState {
    match client.status().await {
        Ok(status) => EngineState::Running {
            version: status.version,
            projects: status.corpora.len(),
        },
        Err(_) => EngineState::Unreachable,
    }
}
