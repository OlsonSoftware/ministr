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
    let registry = CorpusRegistry::new(embedder, config);
    Ok(AppState::new(registry))
}

/// Build state, restore previously-registered corpora, then serve the
/// daemon on the platform-native IPC endpoint until shutdown.
///
/// This is the headless entrypoint the CLI's hidden `__daemon`
/// subcommand runs, and the same path a future GUI could call.
///
/// # Errors
///
/// Propagates embedding-init failures and listener-bind failures —
/// including the deliberate "another ministr daemon is already running"
/// error, which callers treat as "a daemon already exists, just attach".
pub async fn run(config: MinistrConfig) -> Result<(), Box<dyn std::error::Error>> {
    let state = build_state(config)?;
    state.registry.restore().await;
    daemon::start(state).await
}
