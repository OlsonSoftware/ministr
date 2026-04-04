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
    let total_sessions: usize = corpora.iter().map(|c| c.active_sessions).sum();

    let log_path = iris_api::daemon_socket_path()
        .parent()
        .map(|p| p.join("iris.log"))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());

    Ok(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: rss,
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
        log_path,
        total_sessions,
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

/// Trigger a full re-index of a corpus.
#[tauri::command]
pub async fn trigger_reindex(state: State<'_, AppState>, corpus_id: String) -> Result<(), String> {
    // Get the paths for this corpus, then re-register (which triggers re-indexing).
    let paths = {
        let guard = state.registry.corpora().read().await;
        guard
            .get(&corpus_id)
            .map(|h| h.info.blocking_read().paths.clone())
    };
    let Some(paths) = paths else {
        return Err(format!("corpus '{corpus_id}' not found"));
    };

    // Re-registering with the same paths triggers re-indexing.
    state
        .registry
        .register(&paths)
        .await
        .map_err(|e| e.to_string())?;
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

/// Check if auto-start at login is enabled.
#[tauri::command]
pub async fn is_autostart_enabled(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Enable or disable auto-start at login.
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

/// Read the last N lines from the daemon log file.
#[tauri::command]
pub async fn read_logs(lines: Option<usize>) -> Result<Vec<String>, String> {
    let max_lines = lines.unwrap_or(200);
    let log_dir = iris_api::daemon_socket_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .to_path_buf();
    let log_path = log_dir.join("iris.log");

    if !log_path.exists() {
        return Ok(vec!["No log file found.".to_string()]);
    }

    let content = std::fs::read_to_string(&log_path).map_err(|e| e.to_string())?;
    let all_lines: Vec<String> = content.lines().map(String::from).collect();
    let start = all_lines.len().saturating_sub(max_lines);
    Ok(all_lines[start..].to_vec())
}

/// Check if first-run onboarding should be shown.
#[tauri::command]
pub async fn should_show_onboarding() -> Result<bool, String> {
    let sentinel = iris_api::daemon_socket_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("onboarding_done");
    Ok(!sentinel.exists())
}

/// Dismiss the onboarding screen.
#[tauri::command]
pub async fn dismiss_onboarding() -> Result<(), String> {
    let sentinel = iris_api::daemon_socket_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("onboarding_done");
    std::fs::write(&sentinel, "").map_err(|e| e.to_string())
}

/// Remove a project by ID (called from tray menu).
#[allow(dead_code)]
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
