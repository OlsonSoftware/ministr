//! Single source of truth for constructing the daemon's [`AppState`] and
//! running it headless.
//!
//! Both `ministr-app` (the GUI host) and the `ministr __daemon` CLI
//! subcommand build the daemon the same way — so the embedder → registry
//! → state wiring lives here exactly once, with no duplication between
//! the two callers.

use ministr_core::config::MinistrConfig;
use ministr_core::embedding;
use tracing::info;

use crate::daemon;
use crate::registry::CorpusRegistry;
use crate::state::AppState;

/// Build the daemon's [`AppState`]: load the embedding model once and
/// wrap a fresh [`CorpusRegistry`]. GUI-free; callers add restore /
/// tray / Tauri wiring on top as needed.
///
/// # Errors
///
/// Returns the embedding initialization error if the model can't be
/// loaded (missing weights, unsupported backend, etc.).
pub fn build_state(config: MinistrConfig) -> Result<AppState, Box<dyn std::error::Error>> {
    let (embedder, backend) = embedding::create_embedder(&config.default_model, &config.data_dir)?;
    info!(
        model = %config.default_model,
        backend = ?backend.format,
        device = %backend.device,
        dim = embedder.dimension(),
        "embedding model loaded"
    );
    let default_model_cache_key = format!("{}{}", config.default_model, backend.cache_key_suffix());
    let registry = CorpusRegistry::new(embedder, default_model_cache_key, config);
    Ok(AppState::new(registry))
}

/// Build state, bind the IPC listener immediately, and restore
/// previously-registered corpora in the background — then serve until
/// shutdown.
///
/// This is the headless entrypoint the CLI's hidden `__daemon`
/// subcommand runs, and the same path a future GUI could call.
///
/// Restore runs in a spawned task rather than blocking the listener bind:
/// the MCP stdio proxy auto-spawns this daemon and then connects to it, so
/// with restore on the critical path the proxy — and therefore the agent's
/// `initialize`/`tools/list` handshake — stalled until *every* persisted
/// corpus's HNSW index had finished loading from disk. Binding first makes
/// the daemon reachable in milliseconds; corpora then surface as the
/// background restore registers them (the desktop UI already polls for
/// this). It is safe to run concurrently with live requests: `register`
/// is idempotent (already-present corpora short-circuit under the map's
/// write lock) and the manifest is written atomically, so a client that
/// registers a corpus mid-restore converges with no torn manifest. The
/// `__daemon` process is only ever spawned when no live daemon exists (the
/// proxy probes first), so there is no competing manifest writer.
///
/// # Errors
///
/// Propagates embedding-init failures and listener-bind failures —
/// including the deliberate "another ministr daemon is already running"
/// error, which callers treat as "a daemon already exists, just attach".
pub async fn run(config: MinistrConfig) -> Result<(), Box<dyn std::error::Error>> {
    let state = build_state(config)?;

    let registry = std::sync::Arc::clone(&state.registry);
    tokio::spawn(async move {
        registry.restore().await;
        info!("background corpus restore complete");
    });

    daemon::start(state).await
}
