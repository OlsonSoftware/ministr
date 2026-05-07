//! Tauri IPC commands — bridge between the React frontend and Rust backend.

use serde::Serialize;

use ministr_api::activity::ActivityEvent;
use ministr_api::coherence::CoherenceEvent;
use ministr_api::corpus::{CorpusInfo, RegisterCorpusResponse};
use ministr_api::status::DaemonStatus;
use ministr_core::session::PressureLevel;
use ministr_core::storage::traits::Storage;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use ministr_daemon::state::AppState;

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

/// Get daemon status (memory, uptime, corpora, autostart).
///
/// `autostart_enabled` is populated by querying the autolaunch plugin
/// directly so the React UI doesn't need a separate `is_autostart_enabled`
/// round-trip on every Settings mount.
#[tauri::command]
pub async fn daemon_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DaemonStatus, String> {
    use tauri_plugin_autostart::ManagerExt;

    let corpora = state.registry.list().await;
    tracing::debug!(corpora_count = corpora.len(), "daemon_status polled");
    let rss = ministr_core::mem_profile::rss_mb().unwrap_or(0.0);
    let total_sessions: usize = corpora.iter().map(|c| c.active_sessions).sum();

    let log_path = Some(ministr_api::daemon_data_dir().join("ministr.log"))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());

    let autostart_enabled = app.autolaunch().is_enabled().ok();

    Ok(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.uptime_secs(),
        memory_mb: rss,
        model: state.registry.config().default_model.clone(),
        model_dimension: state.registry.embedder().dimension(),
        corpora,
        log_path,
        total_sessions,
        autostart_enabled,
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
    tracing::info!(corpus_id = %corpus_id, "remove_project called from frontend");

    // Get data_dir before unregistering.
    let data_dir = {
        let guard = state.registry.corpora().read().await;
        guard.get(&corpus_id).map(|h| h.data_dir.clone())
    };

    state.registry.unregister(&corpus_id).await.map_err(|e| {
        tracing::error!(corpus_id = %corpus_id, error = %e, "unregister failed");
        e.to_string()
    })?;

    // Clean up index data.
    if let Some(dir) = data_dir
        && dir.exists()
    {
        let _ = std::fs::remove_dir_all(&dir);
        tracing::info!(path = %dir.display(), "cleaned up corpus data");
    }

    Ok(())
}

/// Trigger a full re-index of a corpus.
#[tauri::command]
pub async fn trigger_reindex(state: State<'_, AppState>, corpus_id: String) -> Result<(), String> {
    tracing::info!(corpus_id = %corpus_id, "trigger_reindex called from frontend");

    // Get the paths for this corpus.
    let paths = {
        let guard = state.registry.corpora().read().await;
        let Some(h) = guard.get(&corpus_id) else {
            tracing::warn!(corpus_id = %corpus_id, "trigger_reindex: corpus not found");
            return Err(format!("corpus '{corpus_id}' not found"));
        };
        h.info.read().await.paths.clone()
    };

    tracing::info!(corpus_id = %corpus_id, paths = ?paths, "trigger_reindex: unregister + re-register");
    // Unregister first, then re-register to force a fresh indexing run.
    let _ = state.registry.unregister(&corpus_id).await;
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
    let log_path = ministr_api::daemon_data_dir().join("ministr.log");

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
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    Ok(!sentinel.exists())
}

/// Dismiss the onboarding screen.
#[tauri::command]
pub async fn dismiss_onboarding() -> Result<(), String> {
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    std::fs::write(&sentinel, "").map_err(|e| e.to_string())
}

/// Reset onboarding so it shows again on next visit.
#[tauri::command]
pub async fn reset_onboarding() -> Result<(), String> {
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    if sentinel.exists() {
        std::fs::remove_file(&sentinel).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Detected project for onboarding.
#[derive(Serialize)]
pub struct DetectedProject {
    pub path: String,
    pub name: String,
}

/// Scan common directories for projects with `.ministr.toml` files.
#[tauri::command]
pub async fn detect_projects() -> Result<Vec<DetectedProject>, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let scan_dirs = [
        home.clone(),
        format!("{home}/Code"),
        format!("{home}/Projects"),
        format!("{home}/Developer"),
        format!("{home}/src"),
    ];

    let mut found = Vec::new();
    for dir in &scan_dirs {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        // Check the directory itself for .ministr.toml
        if dir != &home && dir_path.join(".ministr.toml").exists() {
            let name = dir_path
                .file_name()
                .map_or_else(|| dir.clone(), |n| n.to_string_lossy().into_owned());
            found.push(DetectedProject {
                path: dir.clone(),
                name,
            });
            continue;
        }
        // Scan one level deep
        let Ok(entries) = std::fs::read_dir(dir_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() && entry_path.join(".ministr.toml").exists() {
                let name = entry_path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                found.push(DetectedProject {
                    path: entry_path.display().to_string(),
                    name,
                });
            }
        }
    }

    // Deduplicate by path
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found.dedup_by(|a, b| a.path == b.path);

    Ok(found)
}

/// Register multiple projects at once (for onboarding batch import).
///
/// `register` is idempotent on canonical identity and never touches an
/// unrelated corpus's state, so registering a sibling project will never
/// destroy a neighbour's sessions. Per-path errors are warned and skipped.
#[tauri::command]
pub async fn register_projects_batch(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut registered = Vec::new();
    for path in &paths {
        let project_dir = std::path::Path::new(path);
        let resolved = ministr_core::config::RepoConfig::discover(project_dir)
            .ok()
            .flatten()
            .map_or_else(
                || vec![path.clone()],
                |(base, rc)| rc.resolve_local_paths(&base),
            );
        match state.registry.register(&resolved).await {
            Ok((corpus_id, _)) => registered.push(corpus_id),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to register project in batch");
            }
        }
    }
    Ok(registered)
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

    if let Some(dir) = data_dir
        && dir.exists()
    {
        let _ = std::fs::remove_dir_all(&dir);
        tracing::info!(path = %dir.display(), "cleaned up corpus data from tray remove");
    }

    Ok(())
}

// ── New GUI feature commands ─────────────────────────────────────────────────

/// Session info returned to the frontend.
#[derive(Serialize)]
pub struct SessionDetail {
    pub session_id: String,
    pub corpus_id: String,
    pub pressure_level: String,
    pub tokens_used: usize,
    pub tokens_remaining: usize,
    pub utilization: f64,
    pub delivered_count: usize,
    pub current_turn: u32,
    // Token economics metrics
    pub total_deliveries: u64,
    pub cumulative_tokens_delivered: u64,
    pub total_tokens_saved: u64,
    pub total_evictions: u64,
    pub total_compressions: u64,
    pub dedup_hits: u64,
    pub compression_ratio: f64,
    /// Parent session id when this session was created on behalf of a
    /// subagent (e.g. Claude Code's Task tool spawning a sub-claude).
    /// `None` for top-level sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// MCP `clientInfo.name` captured at initialize (e.g. "claude-code",
    /// "mcp-inspector"). `None` until the handshake completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
}

/// List all active sessions across all corpora.
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionDetail>, String> {
    let guard = state.registry.corpora().read().await;
    let mut sessions = Vec::new();

    for (corpus_id, handle) in guard.iter() {
        let reg = handle.sessions.lock().await;
        for sid in reg.session_ids() {
            if let Some(entry) = reg.get_session(&sid) {
                let status = entry.budget.budget_status();
                let metrics = entry.session.metrics();
                #[allow(clippy::cast_precision_loss)]
                let compression_ratio = if metrics.cumulative_tokens_delivered > 0 {
                    metrics.total_tokens_saved() as f64 / metrics.cumulative_tokens_delivered as f64
                } else {
                    0.0
                };
                sessions.push(SessionDetail {
                    session_id: sid.clone(),
                    corpus_id: corpus_id.clone(),
                    pressure_level: match entry.budget.pressure_level() {
                        PressureLevel::Normal => "normal",
                        PressureLevel::Elevated => "elevated",
                        PressureLevel::Critical => "critical",
                    }
                    .to_string(),
                    tokens_used: status.tokens_used,
                    tokens_remaining: status.tokens_remaining,
                    utilization: status.utilization,
                    delivered_count: entry.session.delivered_ids().len(),
                    current_turn: entry.session.current_turn(),
                    total_deliveries: metrics.total_deliveries,
                    cumulative_tokens_delivered: metrics.cumulative_tokens_delivered,
                    total_tokens_saved: metrics.total_tokens_saved(),
                    total_evictions: metrics.total_evictions,
                    total_compressions: metrics.total_compressions,
                    dedup_hits: metrics.dedup_hits,
                    compression_ratio,
                    parent_session_id: entry
                        .parent_session_id
                        .as_ref()
                        .map(std::string::ToString::to_string),
                    client_name: entry.client_name.clone(),
                });
            }
        }
    }

    Ok(sessions)
}

/// File info for the corpus treemap.
#[derive(Serialize)]
pub struct FileInfo {
    pub path: String,
    pub content_hash: String,
    pub mtime_ns: Option<i64>,
    pub section_count: usize,
}

/// List all indexed files for a corpus with section counts.
#[tauri::command]
pub async fn list_corpus_files(
    state: State<'_, AppState>,
    corpus_id: String,
) -> Result<Vec<FileInfo>, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;
    let storage = &handle.storage;

    let hashes = storage
        .list_file_hashes()
        .await
        .map_err(|e| e.to_string())?;

    // Count sections per source_path by querying documents then sections.
    let docs = storage.list_documents().await.map_err(|e| e.to_string())?;

    let mut section_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for doc in &docs {
        let sections = storage.list_sections(&doc.id).await.unwrap_or_default();
        *section_counts.entry(doc.source_path.clone()).or_default() += sections.len();
    }

    Ok(hashes
        .into_iter()
        .map(|h| FileInfo {
            section_count: section_counts.get(&h.path).copied().unwrap_or(0),
            path: h.path,
            content_hash: h.content_hash,
            mtime_ns: h.mtime_ns,
        })
        .collect())
}

/// Search result returned to the frontend.
#[derive(Serialize)]
pub struct SearchResult {
    pub content_id: String,
    pub resolution: String,
    pub score: f32,
    pub text: String,
    pub heading_path: Vec<String>,
}

/// Search a corpus by query (wraps `QueryService::survey`).
#[tauri::command]
pub async fn search_corpus(
    state: State<'_, AppState>,
    corpus_id: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<SearchResult>, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let results = handle
        .service
        .survey(&query, top_k.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;

    Ok(results
        .into_iter()
        .map(|r| SearchResult {
            content_id: r.content_id,
            resolution: r.resolution,
            score: r.score,
            text: r.text,
            heading_path: r.heading_path.unwrap_or_default(),
        })
        .collect())
}

/// Symbol info returned to the frontend.
#[derive(Serialize)]
pub struct SymbolInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub visibility: String,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub module_path: String,
}

/// Search symbols in a corpus.
#[tauri::command]
pub async fn search_symbols(
    state: State<'_, AppState>,
    corpus_id: String,
    query: String,
    kind: Option<String>,
    file_path: Option<String>,
) -> Result<Vec<SymbolInfo>, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let filter = ministr_core::storage::traits::SymbolFilter {
        name: Some(query),
        name_exact: None,
        kind,
        visibility: None,
        module: None,
        file_path,
    };

    let records = handle
        .storage
        .list_symbols(&filter)
        .await
        .map_err(|e| e.to_string())?;

    Ok(records
        .into_iter()
        .map(|r| SymbolInfo {
            id: r.id.0,
            name: r.name,
            kind: r.kind,
            file_path: r.file_path,
            visibility: r.visibility,
            signature: r.signature,
            doc_comment: r.doc_comment,
            module_path: r.module_path,
        })
        .collect())
}

/// Reference link for the symbol graph.
#[derive(Serialize)]
pub struct SymbolRef {
    pub from_name: String,
    pub from_file: String,
    pub to_name: String,
    pub to_file: String,
    pub ref_kind: String,
}

/// Get references (callers, importers, implementors) for a symbol.
#[tauri::command]
pub async fn symbol_references(
    state: State<'_, AppState>,
    corpus_id: String,
    symbol_id: String,
) -> Result<Vec<SymbolRef>, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let refs = handle
        .service
        .get_symbol_references(&symbol_id, None)
        .await
        .map_err(|e| e.to_string())?;

    Ok(refs
        .into_iter()
        .map(|r| SymbolRef {
            from_name: r.from_name,
            from_file: r.from_file,
            to_name: r.to_name,
            to_file: r.to_file,
            ref_kind: r.ref_kind,
        })
        .collect())
}

/// Ingestion progress snapshot for a corpus.
#[derive(Serialize)]
pub struct IngestionProgressInfo {
    pub corpus_id: String,
    pub status: u8,
    pub phase: String,
    pub files_total: usize,
    pub files_done: usize,
    pub sections_done: usize,
    pub embeddings_total: usize,
    pub embeddings_done: usize,
    pub current_file: String,
}

/// Snapshot recent coherence (file-change) events from the in-process
/// ring buffer. Mirrors the daemon's `/coherence-events` route.
#[tauri::command]
pub async fn recent_coherence_events(
    state: State<'_, AppState>,
    limit: Option<usize>,
    since_ms: Option<u64>,
) -> Result<Vec<CoherenceEvent>, String> {
    let limit = limit.unwrap_or(50);
    let events = if let Some(since) = since_ms {
        state.coherence_since(since, limit).await
    } else {
        state.recent_coherence(limit).await
    };
    Ok(events)
}

/// Snapshot recent tool-call activity events from the in-process ring buffer.
///
/// Mirrors the daemon's `/activity` HTTP endpoint for the Tauri frontend —
/// when the Tauri app runs in-process it consults [`AppState::activity`]
/// directly rather than hopping over UDS.
#[tauri::command]
pub async fn recent_activity(
    state: State<'_, AppState>,
    limit: Option<usize>,
    since_ms: Option<u64>,
) -> Result<Vec<ActivityEvent>, String> {
    let limit = limit.unwrap_or(50);
    let events = if let Some(since) = since_ms {
        state.activity_since(since, limit).await
    } else {
        state.recent_activity(limit).await
    };
    Ok(events)
}

/// Cross-language bridge link returned to the frontend.
#[derive(Serialize)]
pub struct BridgeLinkOut {
    pub kind: String,
    pub confidence: f32,
    pub export_file: String,
    pub export_binding_key: String,
    pub export_symbol: String,
    pub export_language: String,
    pub export_line: u32,
    pub import_file: String,
    pub import_binding_key: String,
    pub import_symbol: String,
    pub import_language: String,
    pub import_line: u32,
}

/// Query cross-language bridge links (Tauri commands, `PyO3`, NAPI, FFI, HTTP routes).
#[tauri::command]
pub async fn bridge_query(
    state: State<'_, AppState>,
    corpus_id: String,
    query: Option<String>,
    kind: Option<String>,
    source_language: Option<String>,
    file_path: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<BridgeLinkOut>, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let links = handle
        .service
        .query_bridges(
            query.as_deref(),
            kind.as_deref(),
            source_language.as_deref(),
            file_path.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;

    let cap = limit.unwrap_or(500);
    Ok(links
        .into_iter()
        .take(cap)
        .map(|l| BridgeLinkOut {
            kind: l.kind,
            confidence: l.confidence,
            export_file: l.export_file,
            export_binding_key: l.export_binding_key,
            export_symbol: l.export_symbol,
            export_language: l.export_language,
            export_line: l.export_line,
            import_file: l.import_file,
            import_binding_key: l.import_binding_key,
            import_symbol: l.import_symbol,
            import_language: l.import_language,
            import_line: l.import_line,
        })
        .collect())
}

/// Full symbol definition with source context.
#[derive(Serialize)]
pub struct SymbolDefinitionOut {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub visibility: String,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub heading_path: Vec<String>,
    pub source_context: String,
}

/// Open a file or folder with the OS default handler.
///
/// Used by the Settings page (Open data folder / Open log file) and any
/// caller that wants the OS file manager / text editor to surface a path.
///
/// Expands a leading `~/` (or bare `~`) to the user's home directory before
/// invoking the OS opener. Tilde expansion is a shell convention; the
/// raw `open` / `explorer.exe` / `xdg-open` syscalls do *not* expand it,
/// so call sites that pass `~/.ministr/` would otherwise fail silently.
#[tauri::command]
pub async fn open_path(path: String) -> Result<(), String> {
    let resolved = expand_tilde(&path);

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&resolved)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(&resolved)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&resolved)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Expand a leading `~/` or bare `~` to the user's home directory.
///
/// Reads `HOME` on Unix and `USERPROFILE` on Windows; falls back to the
/// original input if neither is set. Only the leading segment is
/// expanded — `~` mid-path is preserved verbatim because that's a
/// filename, not a shell expansion.
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return home_dir().unwrap_or_else(|| path.to_string());
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = home_dir()
    {
        let sep = if cfg!(windows) { '\\' } else { '/' };
        return format!("{home}{sep}{rest}");
    }
    path.to_string()
}

fn home_dir() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").ok()
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME").ok()
    }
}

/// Read a snippet of a source file with a small context window.
///
/// Used by the Bridge tab to render side-by-side endpoint code panes.
/// Verifies the corpus exists AND that `file_path` resolves inside one
/// of that corpus's root paths before reading from disk — without the
/// scope check, a renderer-side caller could exfiltrate arbitrary text
/// files from the host filesystem.
#[tauri::command]
pub async fn read_source_excerpt(
    state: State<'_, AppState>,
    corpus_id: String,
    file_path: String,
    line_start: u32,
    line_end: u32,
) -> Result<String, String> {
    // Snapshot the corpus's root paths under the registry lock, then
    // drop the guard so the canonicalize awaits don't hold it.
    let roots: Vec<String> = {
        let guard = state.registry.corpora().read().await;
        let handle = guard.get(&corpus_id).ok_or("corpus not found")?;
        handle.info.read().await.paths.clone()
    };

    // Canonicalize both sides so symlinks / `..` segments / relative paths
    // can't be used to step outside a corpus root. canonicalize() implicitly
    // verifies the file exists; we treat the I/O error as "outside corpus"
    // rather than leaking a missing-file error message.
    let target = tokio::fs::canonicalize(&file_path)
        .await
        .map_err(|_| "path outside corpus".to_string())?;
    let mut allowed = false;
    for root in &roots {
        if let Ok(canonical_root) = tokio::fs::canonicalize(root).await
            && target.starts_with(&canonical_root)
        {
            allowed = true;
            break;
        }
    }
    if !allowed {
        return Err("path outside corpus".to_string());
    }

    let content = tokio::fs::read_to_string(&target)
        .await
        .map_err(|e| e.to_string())?;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if total == 0 {
        return Ok(String::new());
    }

    // 1-based line numbers from the daemon. Take a 3-line context window.
    let s = (line_start.saturating_sub(4) as usize).min(total);
    let e = ((line_end as usize).saturating_add(3)).min(total);
    Ok(lines[s..e.max(s)].join("\n"))
}

/// Get the full definition of a symbol with surrounding source context.
#[tauri::command]
pub async fn symbol_definition(
    state: State<'_, AppState>,
    corpus_id: String,
    symbol_id: String,
) -> Result<SymbolDefinitionOut, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let def = handle
        .service
        .get_symbol_definition(&symbol_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(SymbolDefinitionOut {
        id: def.id,
        name: def.name,
        kind: def.kind,
        visibility: def.visibility,
        signature: def.signature,
        doc_comment: def.doc_comment,
        file_path: def.file_path,
        line_start: def.line_start,
        line_end: def.line_end,
        heading_path: def.heading_path,
        source_context: def.source_context,
    })
}

/// Get ingestion progress for all corpora.
#[tauri::command]
pub async fn ingestion_progress(
    state: State<'_, AppState>,
) -> Result<Vec<IngestionProgressInfo>, String> {
    let guard = state.registry.corpora().read().await;
    Ok(guard
        .iter()
        .map(|(corpus_id, handle)| IngestionProgressInfo {
            corpus_id: corpus_id.clone(),
            status: handle.progress.status(),
            phase: handle.progress.phase().as_str().to_string(),
            files_total: handle.progress.files_total(),
            files_done: handle.progress.files_done(),
            sections_done: handle.progress.sections_done(),
            embeddings_total: handle.progress.embeddings_total(),
            embeddings_done: handle.progress.embeddings_done(),
            current_file: handle.progress.current_file(),
        })
        .collect())
}

/// Push-based indexing-progress event streamed to the frontend over a
/// [`Channel`]. The frontend opens this once per surface that needs live
/// progress (Projects, Onboarding) and consumes events as they arrive,
/// avoiding the previous 1Hz polling of `ingestion_progress`.
///
/// `status`: 0 = pending, 1 = running, 2 = complete (mirrors `IngestionProgress`).
/// `estimated_remaining_secs` is `None` until at least one second of
/// running samples has been observed (rate is too noisy below that).
#[derive(Clone, Serialize)]
pub struct IndexingProgressEvent {
    pub corpus_id: String,
    pub status: u8,
    pub phase: String,
    pub files_total: usize,
    pub files_done: usize,
    pub sections_done: usize,
    pub embeddings_total: usize,
    pub embeddings_done: usize,
    pub current_file: String,
    pub estimated_remaining_secs: Option<u64>,
    pub timestamp_ms: u64,
}

/// Stream indexing-progress events to the frontend.
///
/// Returns immediately after spawning a background task that polls the
/// atomic `IngestionProgress` for every corpus on a 250ms tick and sends
/// an [`IndexingProgressEvent`] whenever something changed (status flip,
/// file count tick, current-file change). The task exits when
/// `on_event.send(...)` fails, which is how the Tauri channel signals
/// that the frontend has dropped its receiver.
///
/// We poll the atomics rather than wiring a notify into ministr-core
/// because the atomics are essentially free to read and the change
/// signal we need (UI repaint) is naturally rate-limited.
#[tauri::command]
pub async fn indexing_progress_events(
    state: State<'_, AppState>,
    on_event: Channel<IndexingProgressEvent>,
) -> Result<(), String> {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    let registry = state.registry.clone();

    tauri::async_runtime::spawn(async move {
        // Per-corpus tracking for change-detection + ETA. We only emit when
        // something the UI cares about changed, and ETA is computed in the
        // command (ministr-core's IngestionProgress doesn't track timing).
        struct Track {
            last_status: u8,
            last_files_done: usize,
            last_current_file: String,
            run_started: Option<Instant>,
        }
        let mut tracks: HashMap<String, Track> = HashMap::new();

        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;

            let now_ms = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
            )
            .unwrap_or(u64::MAX);

            let guard = registry.corpora().read().await;
            for (corpus_id, handle) in guard.iter() {
                let p = &handle.progress;
                let status = p.status();
                let files_total = p.files_total();
                let files_done = p.files_done();
                let current_file = p.current_file();

                let track = tracks.entry(corpus_id.clone()).or_insert(Track {
                    last_status: u8::MAX,
                    last_files_done: 0,
                    last_current_file: String::new(),
                    run_started: None,
                });

                let started_running = status == 1 && track.last_status != 1;
                let stopped_running = status != 1 && track.last_status == 1;
                let progressed = files_done != track.last_files_done
                    || current_file != track.last_current_file;
                let status_changed = status != track.last_status;

                if started_running {
                    track.run_started = Some(Instant::now());
                }
                if stopped_running {
                    track.run_started = None;
                }

                if !status_changed && !progressed {
                    continue;
                }

                let estimated_remaining_secs = if status == 1 && files_total > files_done {
                    track.run_started.and_then(|t| {
                        let elapsed = t.elapsed().as_secs_f64();
                        if elapsed < 1.0 || files_done == 0 {
                            return None;
                        }
                        // Precision loss is fine — these counts top out in
                        // the millions for huge corpora and we only render
                        // ETA to the nearest second.
                        #[allow(clippy::cast_precision_loss)]
                        let rate = files_done as f64 / elapsed;
                        if rate <= 0.0 {
                            return None;
                        }
                        #[allow(
                            clippy::cast_precision_loss,
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss
                        )]
                        let remaining =
                            (((files_total - files_done) as f64) / rate).round() as u64;
                        Some(remaining)
                    })
                } else {
                    None
                };

                let ev = IndexingProgressEvent {
                    corpus_id: corpus_id.clone(),
                    status,
                    phase: p.phase().as_str().to_string(),
                    files_total,
                    files_done,
                    sections_done: p.sections_done(),
                    embeddings_total: p.embeddings_total(),
                    embeddings_done: p.embeddings_done(),
                    current_file: current_file.clone(),
                    estimated_remaining_secs,
                    timestamp_ms: now_ms,
                };

                track.last_status = status;
                track.last_files_done = files_done;
                track.last_current_file = current_file;

                if on_event.send(ev).is_err() {
                    // Frontend dropped the receiver — exit cleanly.
                    return;
                }
            }
            drop(guard);
        }
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Ask (sub-inference) — phased, citation-aware Q&A for the desktop app.
// ---------------------------------------------------------------------------

/// Phase events streamed from `ask_corpus` to the frontend so the UI can
/// render retrieving → synthesizing → done without faking progress.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AskPhase {
    /// Verified cache hit — answer is about to arrive in `Done`.
    CacheHit { source_ids: Vec<String> },
    /// Query analysis finished. Sub-question decomposition + `HyDE` preview
    /// + symbol hints + bridge relevance flag arrive together.
    Analyzed {
        sub_questions: Vec<String>,
        hyde_preview: String,
        symbol_hints: Vec<String>,
        bridge_relevant: bool,
    },
    /// Multi-strategy retrieval finished. Reports per-strategy counts +
    /// the merged candidate ids that survived RRF fusion.
    RetrievedCandidates {
        by_strategy: std::collections::HashMap<String, usize>,
        merged_ids: Vec<String>,
    },
    /// LLM rerank pass finished — these are the surviving sources in
    /// score order.
    Reranked { source_ids: Vec<String> },
    /// All retrieval is done; inference is about to start.
    Retrieved { source_ids: Vec<String> },
    /// Verification stage ran. `unsupported_claims` is empty when the
    /// answer is fully grounded; non-empty entries already appear in
    /// the final `Done` answer as a confidence note.
    Verified { unsupported_claims: Vec<String> },
    /// Final answer with citations.
    Done {
        answer: String,
        source_ids: Vec<String>,
        cached: bool,
        model: String,
        elapsed_ms: u64,
    },
    /// Pipeline failed. The command will also return Err(message).
    Error { message: String },
}

/// Synthesize an answer for a natural-language question against a corpus.
///
/// Streams phase events on `progress` so the UI can render skeletons that
/// resolve into real content. The full answer is also returned via the
/// final `Done` event; the command's `Result` is just a success signal.
#[tauri::command]
pub async fn ask_corpus(
    state: State<'_, AppState>,
    corpus_id: String,
    query: String,
    progress: Channel<AskPhase>,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let _permit = state
        .query_semaphore
        .acquire()
        .await
        .map_err(|e| e.to_string())?;

    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let progress_for_callback = progress.clone();
    let result = ministr_daemon::ask::ask_with_progress(
        &query,
        &handle.service,
        &handle.storage,
        state.inference.as_ref(),
        move |event| {
            let phase = match event {
                ministr_daemon::ask::AskEvent::CacheHit { source_ids } => {
                    AskPhase::CacheHit { source_ids }
                }
                ministr_daemon::ask::AskEvent::Analyzed {
                    sub_questions,
                    hyde_preview,
                    symbol_hints,
                    bridge_relevant,
                } => AskPhase::Analyzed {
                    sub_questions,
                    hyde_preview,
                    symbol_hints,
                    bridge_relevant,
                },
                ministr_daemon::ask::AskEvent::RetrievedCandidates {
                    by_strategy,
                    merged_ids,
                } => AskPhase::RetrievedCandidates {
                    by_strategy,
                    merged_ids,
                },
                ministr_daemon::ask::AskEvent::Reranked { source_ids } => {
                    AskPhase::Reranked { source_ids }
                }
                ministr_daemon::ask::AskEvent::Retrieved { source_ids } => {
                    AskPhase::Retrieved { source_ids }
                }
                ministr_daemon::ask::AskEvent::Verified { unsupported_claims } => {
                    AskPhase::Verified { unsupported_claims }
                }
            };
            // Channel send only fails if the frontend dropped the receiver,
            // in which case there's nothing useful to do here.
            let _ = progress_for_callback.send(phase);
        },
    )
    .await;
    drop(guard);

    match result {
        Ok(r) => {
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let _ = progress.send(AskPhase::Done {
                answer: r.answer,
                source_ids: r.source_ids,
                cached: r.cached,
                model: r.model,
                elapsed_ms,
            });
            Ok(())
        }
        Err(e) => {
            let message = e.to_string();
            let _ = progress.send(AskPhase::Error {
                message: message.clone(),
            });
            Err(message)
        }
    }
}

/// Health summary for the sub-inference backend used by `ask_corpus`.
#[derive(Serialize)]
pub struct InferenceHealth {
    /// True if a usable inference backend is wired up. Currently this means
    /// the `claude` CLI is present on PATH for the production
    /// `ClaudeCliInference`. False means `ask` will fail at submit time.
    pub available: bool,
    /// Short human-readable reason when `available` is false (e.g.
    /// "claude CLI not found on PATH"). Empty when available.
    pub reason: String,
    /// Best-effort path to the resolved binary, when available.
    pub binary_path: Option<String>,
}

/// Probe whether the inference backend is ready, without invoking it.
///
/// The Ask tab shows a one-time install hint when this returns
/// `available: false` so users find out about missing dependencies before
/// typing a question rather than after.
#[tauri::command]
pub async fn inference_health(_state: State<'_, AppState>) -> Result<InferenceHealth, String> {
    // The default backend is ClaudeCliInference, which spawns `claude -p`.
    // A PATH probe is the cheapest reliable readiness signal.
    let binary = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    if let Some(path) = which_on_path(binary) {
        Ok(InferenceHealth {
            available: true,
            reason: String::new(),
            binary_path: Some(path),
        })
    } else {
        Ok(InferenceHealth {
            available: false,
            reason: format!("`{binary}` not found on PATH — install Claude Code to enable Ask."),
            binary_path: None,
        })
    }
}

/// Look up a binary on `PATH`, returning the first absolute match.
fn which_on_path(name: &str) -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    None
}

/// A section's full text, used by `AskView` to resolve a citation
/// `content_id` into something it can hand to the entity panel as a
/// `SearchResult`.
#[derive(Serialize)]
pub struct SectionDetailOut {
    pub section_id: String,
    pub heading_path: Vec<String>,
    pub text: String,
    pub summary: Option<String>,
    pub claims_available: usize,
}

/// Read the full text of a section by its hierarchical content ID.
#[tauri::command]
pub async fn read_section(
    state: State<'_, AppState>,
    corpus_id: String,
    section_id: String,
) -> Result<SectionDetailOut, String> {
    let guard = state.registry.corpora().read().await;
    let handle = guard.get(&corpus_id).ok_or("corpus not found")?;

    let detail = handle
        .service
        .read_section(&section_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(SectionDetailOut {
        section_id: detail.section_id,
        heading_path: detail.heading_path,
        text: detail.text,
        summary: detail.summary,
        claims_available: detail.claims_available,
    })
}
