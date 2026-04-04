//! Tauri IPC commands — bridge between the Svelte frontend and Rust backend.

use iris_api::corpus::{CorpusInfo, RegisterCorpusResponse};
use iris_api::status::DaemonStatus;
use tauri::{AppHandle, Manager, State};

use iris_daemon::state::AppState;

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

/// Open a directory picker dialog and register the selected directory as a corpus.
#[tauri::command]
pub async fn add_project_dialog(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<RegisterCorpusResponse>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app.dialog().file().blocking_pick_folder();

    let Some(folder) = picked else {
        return Ok(None);
    };

    let path = folder.to_string();
    let (corpus_id, indexing_started) = state
        .registry
        .register(&[path])
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(RegisterCorpusResponse {
        corpus_id,
        indexing_started,
    }))
}

/// Remove a project and clean up its index data.
#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, corpus_id: String) -> Result<(), String> {
    // Get data_dir before unregistering.
    let data_dir = {
        let guard = state.registry.corpora().read().await;
        guard.get(&corpus_id).map(|h| h.data_dir.clone())
    };

    state
        .registry
        .unregister(&corpus_id)
        .await
        .map_err(|e| e.to_string())?;

    // Clean up index data.
    if let Some(dir) = data_dir {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
            tracing::info!(path = %dir.display(), "cleaned up corpus data");
        }
    }

    Ok(())
}

/// Add a project from the tray menu (called from Rust, not from JS).
pub async fn add_project_from_tray(handle: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;

    let picked = handle.dialog().file().blocking_pick_folder();

    let Some(folder) = picked else {
        return;
    };

    let path = folder.to_string();
    let state = handle.state::<AppState>();
    match state.registry.register(std::slice::from_ref(&path)).await {
        Ok((corpus_id, _)) => {
            tracing::info!(corpus_id, path, "project added from tray");
        }
        Err(e) => {
            tracing::warn!(error = %e, path, "failed to add project from tray");
        }
    }
}

/// Remove a project by ID (called from tray menu).
pub async fn remove_project_by_id(handle: &AppHandle, corpus_id: &str) -> Result<(), String> {
    let state = handle.state::<AppState>();

    // Get data_dir before unregistering.
    let data_dir = {
        let guard = state.registry.corpora().read().await;
        guard.get(corpus_id).map(|h| h.data_dir.clone())
    };

    state
        .registry
        .unregister(corpus_id)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(dir) = data_dir {
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
            tracing::info!(path = %dir.display(), "cleaned up corpus data from tray remove");
        }
    }

    Ok(())
}
