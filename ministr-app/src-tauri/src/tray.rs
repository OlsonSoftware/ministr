//! Static system-tray menu — three entries plus a live tooltip.
//!
//! Pre-IA-collapse this menu was dynamic: recent-corpora submenu,
//! indexing submenu, Sessions and Logs entries. With the new three-
//! surface IA those affordances all live in the main window
//! (top-bar project picker, Projects surface, Settings → Developer),
//! so the tray serves a narrower role: restore the window, add a
//! project without opening the dashboard, quit. The tooltip still
//! refreshes every 10s with live counts so the user can see ministr
//! is alive at a glance.

use ministr_daemon::state::AppState;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};

/// Tray icon ID used for lookups.
pub const TRAY_ID: &str = "ministr-tray";

/// Build the tray icon with the static three-entry menu.
///
/// The menu is set once and never rebuilt — only the tooltip changes
/// over time (see [`spawn_refresh_loop`]).
pub fn build_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_menu(app.handle())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .tooltip("ministr — index daemon")
        .on_menu_event(|app, event| handle_event(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}

/// Route a tray menu event.
pub fn handle_event(app: &AppHandle, event_id: &str) {
    match event_id {
        "show" => {
            show_window(app);
            // Surface-aware navigation: the frontend interprets unknown
            // payloads gracefully, but `ask` is the canonical default.
            let _ = app.emit("navigate", "ask");
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
            if let Err(e) = std::fs::remove_file(ministr_api::daemon_socket_path())
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(error = %e, "failed to remove daemon socket on quit");
            }
            if let Err(e) = std::fs::remove_file(ministr_api::daemon_pid_path())
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(error = %e, "failed to remove daemon pid file on quit");
            }
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
) -> Result<tauri::menu::Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Open ministr").build(handle)?;
    let add = MenuItemBuilder::with_id("add_project", "Add project…").build(handle)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit ministr").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .items(&[&show, &add])
        .separator()
        .items(&[&quit])
        .build()?;
    Ok(menu)
}

/// Spawn the periodic tray-tooltip refresh loop.
///
/// Tooltip-only after the IA collapse — the menu itself is static so
/// we no longer rebuild it from the corpora list. The loop still runs
/// at 10s cadence so the user can hover the tray icon and see live
/// counts (corpora, sessions, RSS) without opening the window.
pub fn spawn_refresh_loop(handle: AppHandle, state: AppState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let corpora = state.registry.list().await;

            let (parent_count, subagent_count) = count_sessions_by_lineage(&state).await;
            let total_sessions = parent_count + subagent_count;
            let rss = ministr_core::mem_profile::rss_mb().unwrap_or(0.0);
            // When subagents are attached, surface the breakdown so the
            // user can spot subagent activity without opening the
            // dashboard. Otherwise keep the line compact.
            let session_part = if subagent_count > 0 {
                format!("{total_sessions} sessions ({parent_count} parent · {subagent_count} sub)")
            } else {
                format!("{total_sessions} sessions")
            };
            let tooltip = format!(
                "ministr — {} projects · {} · {:.0} MB",
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
/// The whole loop runs without crossing an `.await` once the corpora
/// map guard is taken: we use `try_lock` on each per-corpus session
/// registry and skip any that's contended this tick. That keeps the
/// registry-map read lifetime bounded by sync work (O(corpora · sessions))
/// instead of tokio scheduler interleavings, so concurrent
/// register/unregister writers don't block on us. The tooltip is
/// informational and refreshes every 10s — missing one tick on a
/// briefly-busy corpus self-heals on the next.
async fn count_sessions_by_lineage(state: &AppState) -> (usize, usize) {
    let mut parents = 0usize;
    let mut subagents = 0usize;
    let guard = state.registry.corpora().read().await;
    for handle in guard.values() {
        let Ok(reg) = handle.sessions.try_lock() else {
            continue;
        };
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
