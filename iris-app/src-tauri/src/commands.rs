//! Tauri IPC commands — bridge between the Svelte frontend and Rust backend.

use iris_api::corpus::{CorpusInfo, RegisterCorpusResponse};
use iris_api::status::DaemonStatus;
use tauri::State;

use crate::state::AppState;

/// List all registered corpora.
#[tauri::command]
pub async fn list_corpora(state: State<'_, AppState>) -> Result<Vec<CorpusInfo>, String> {
    Ok(state.registry.list().await)
}

/// Register a new corpus by paths.
#[tauri::command]
pub async fn register_corpus(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<RegisterCorpusResponse, String> {
    let (corpus_id, indexing_started) = state
        .registry
        .register(&paths)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RegisterCorpusResponse {
        corpus_id,
        indexing_started,
    })
}

/// Unregister a corpus.
#[tauri::command]
pub async fn unregister_corpus(
    state: State<'_, AppState>,
    corpus_id: String,
) -> Result<(), String> {
    state
        .registry
        .unregister(&corpus_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get daemon status (memory, uptime, corpora).
#[tauri::command]
pub async fn daemon_status(state: State<'_, AppState>) -> Result<DaemonStatus, String> {
    let corpora = state.registry.list().await;
    let rss = iris_core::mem_profile::rss_mb().unwrap_or(0.0);

    Ok(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: rss,
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
    })
}
