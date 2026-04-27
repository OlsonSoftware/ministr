//! Dynamic system-tray menu with live submenus for recent corpora and
//! in-flight indexing status.
//!
//! The menu is rebuilt periodically from the daemon's current state so
//! users can see at a glance which corpora are indexing and jump to
//! recently-registered projects directly from the menu bar without
//! opening the dashboard.

use ministr_api::corpus::{CorpusInfo, IndexingStatus};
use ministr_daemon::state::AppState;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    tray::TrayIconBuilder,
};

/// Tray icon ID used for lookups.
pub const TRAY_ID: &str = "ministr-tray";

/// Maximum number of corpora to show in the "Recent corpora" submenu.
const RECENT_LIMIT: usize = 5;

/// Maximum characters to display for a corpus in the tray (truncates long paths).
const MENU_LABEL_MAX: usize = 40;

/// Build the initial tray icon with a static placeholder menu.
///
/// The menu is a best-effort first render — [`rebuild_menu`] replaces
/// it with the full live menu shortly after startup and every 10s after
/// that.
pub fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_menu(app.handle(), &[])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .tooltip("ministr — index daemon")
        .on_menu_event(|app, event| handle_event(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

/// Rebuild the tray menu from the current list of corpora.
pub fn rebuild_menu(handle: &AppHandle, corpora: &[CorpusInfo]) {
    let Some(tray) = handle.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(handle, corpora) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                tracing::warn!(error = %e, "failed to update tray menu");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to build tray menu");
        }
    }
}

/// Route a tray menu event. The handler is synchronous so corpus-specific
/// actions (prefixed with `corpus:`) are parsed here and delegated to the
/// frontend via `select-corpus`/`navigate` events.
pub fn handle_event(app: &AppHandle, event_id: &str) {
    if let Some(corpus_id) = event_id.strip_prefix("corpus:") {
        show_window(app);
        let _ = app.emit("select-corpus", corpus_id.to_string());
        let _ = app.emit("navigate", "projects");
        return;
    }

    match event_id {
        "show" => {
            show_window(app);
            let _ = app.emit("navigate", "overview");
        }
        "show_sessions" => {
            show_window(app);
            let _ = app.emit("navigate", "sessions");
        }
        "show_logs" => {
            show_window(app);
            let _ = app.emit("navigate", "logs");
        }
        "add_project" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::commands::add_project_from_tray(&handle).await;
            });
        }
        "quit" => {
            // Best-effort cleanup — the socket file only exists on Unix;
            // Windows named pipes are torn down automatically when the
            // owning process exits.
            #[cfg(unix)]
            let _ = std::fs::remove_file(ministr_api::daemon_socket_path());
            let _ = std::fs::remove_file(ministr_api::daemon_pid_path());
            app.exit(0);
        }
        _ => {}
    }
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_menu(
    handle: &AppHandle,
    corpora: &[CorpusInfo],
) -> Result<tauri::menu::Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Open Overview").build(handle)?;
    let sessions = MenuItemBuilder::with_id("show_sessions", "Sessions").build(handle)?;
    let add = MenuItemBuilder::with_id("add_project", "Add Project…").build(handle)?;
    let logs = MenuItemBuilder::with_id("show_logs", "View Logs").build(handle)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit ministr").build(handle)?;

    // ---- Recent corpora submenu ----
    let recent_sub = {
        let mut b = SubmenuBuilder::new(handle, "Recent corpora");
        let recents: Vec<&CorpusInfo> = corpora.iter().take(RECENT_LIMIT).collect();
        if recents.is_empty() {
            let placeholder = MenuItemBuilder::with_id("recent_empty", "No corpora registered")
                .enabled(false)
                .build(handle)?;
            b = b.item(&placeholder);
        } else {
            for corpus in recents {
                let label = corpus_menu_label(corpus);
                let id = format!("corpus:{}", corpus.id);
                let item = MenuItemBuilder::with_id(&id, &label).build(handle)?;
                b = b.item(&item);
            }
        }
        b.build()?
    };

    // ---- Indexing submenu ----
    let indexing_sub = {
        let mut b = SubmenuBuilder::new(handle, "Indexing");
        let active: Vec<&CorpusInfo> = corpora
            .iter()
            .filter(|c| matches!(c.status, IndexingStatus::Indexing { .. }))
            .collect();
        if active.is_empty() {
            let placeholder = MenuItemBuilder::with_id("indexing_empty", "No indexing in progress")
                .enabled(false)
                .build(handle)?;
            b = b.item(&placeholder);
        } else {
            for corpus in active {
                let label = indexing_menu_label(corpus);
                let id = format!("corpus:{}", corpus.id);
                let item = MenuItemBuilder::with_id(&id, &label).build(handle)?;
                b = b.item(&item);
            }
        }
        b.build()?
    };

    let menu = MenuBuilder::new(handle)
        .items(&[&show, &sessions])
        .separator()
        .items(&[&recent_sub, &indexing_sub])
        .separator()
        .items(&[&add, &logs])
        .separator()
        .items(&[&quit])
        .build()?;
    Ok(menu)
}

fn corpus_menu_label(corpus: &CorpusInfo) -> String {
    // The daemon computes `display_name` from the LCA of the registered
    // paths (typically the directory containing `.ministr.toml`), so
    // it's already the right thing to show. Fall back to the basename
    // of the first path only if an older daemon left it empty.
    let name = if corpus.display_name.is_empty() {
        let primary = corpus.paths.first().cloned().unwrap_or_default();
        std::path::Path::new(&primary)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&primary)
            .to_string()
    } else {
        corpus.display_name.clone()
    };
    truncate(&name, MENU_LABEL_MAX)
}

fn indexing_menu_label(corpus: &CorpusInfo) -> String {
    let name = corpus_menu_label(corpus);
    match &corpus.status {
        IndexingStatus::Indexing {
            files_done,
            files_total,
        } => {
            // Precision loss is irrelevant at this scale — we only display a
            // 0-100 integer percent for tray feedback.
            #[allow(clippy::cast_precision_loss)]
            let pct = if *files_total == 0 {
                0.0
            } else {
                (*files_done as f64 / *files_total as f64) * 100.0
            };
            format!("{name} — {pct:.0}% ({files_done}/{files_total} files)")
        }
        _ => name,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Spawn the periodic tray menu refresh loop.
pub fn spawn_refresh_loop(handle: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let corpora = state.registry.list().await;
            rebuild_menu(&handle, &corpora);

            // Tooltip (kept here so the loop has a single source of truth).
            let (parent_count, subagent_count) = count_sessions_by_lineage(&state).await;
            let total_sessions = parent_count + subagent_count;
            let rss = ministr_core::mem_profile::rss_mb().unwrap_or(0.0);
            // When subagents are attached, surface the breakdown so the
            // user can spot subagent activity without opening the
            // dashboard. Otherwise keep the line compact.
            let session_part = if subagent_count > 0 {
                format!(
                    "{total_sessions} sessions ({parent_count} parent · {subagent_count} sub)"
                )
            } else {
                format!("{total_sessions} sessions")
            };
            let tooltip = format!(
                "ministr — {} corpora · {} · {:.0} MB",
                corpora.len(),
                session_part,
                rss,
            );
            if let Some(tray) = handle.tray_by_id(TRAY_ID) {
                let _ = tray.set_tooltip(Some(&tooltip));
            }
        }
    });
}

/// Count active sessions across all corpora, split by whether the
/// session has a parent (subagent) or not (top-level).
///
/// Walks the corpus registry and locks each corpus's session registry
/// briefly; the call is O(corpora · sessions) but session counts are
/// small in practice and the lock duration is tiny.
async fn count_sessions_by_lineage(state: &AppState) -> (usize, usize) {
    let mut parents = 0usize;
    let mut subagents = 0usize;
    let guard = state.registry.corpora().read().await;
    for handle in guard.values() {
        let reg = handle.sessions.lock().await;
        for sid in reg.session_ids() {
            if let Some(entry) = reg.get_session(&sid) {
                if entry.parent_session_id.is_some() {
                    subagents += 1;
                } else {
                    parents += 1;
                }
            }
        }
    }
    (parents, subagents)
}
