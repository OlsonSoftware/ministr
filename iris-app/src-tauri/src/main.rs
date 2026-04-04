//! iris desktop app — Tauri v2 entry point.
//!
//! Starts the iris daemon (axum on UDS) alongside the Tauri webview.
//! The app runs as a system tray icon by default, with the main window
//! hidden until the user clicks the tray icon.

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::Arc;

use iris_core::config::IrisConfig;
use iris_core::embedding::FastEmbedder;
use iris_daemon::daemon;
use iris_daemon::registry::CorpusRegistry;
use iris_daemon::state::AppState;
use tauri::{
    AppHandle, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIcon, TrayIconBuilder},
};
use tracing::info;

/// Tray icon ID used for lookups.
const TRAY_ID: &str = "iris-tray";

fn main() {
    // Initialize tracing to stderr (stdout is reserved for Tauri IPC).
    iris_core::tracing::init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, show the main window.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // --- Initialize the iris daemon ---
            let config = IrisConfig::load(&IrisConfig::default_path())
                .unwrap_or_else(|_| IrisConfig::default());

            // Load embedding model (once for all corpora).
            let raw_embedder: Arc<dyn iris_core::embedding::Embedder> = Arc::new(
                FastEmbedder::with_data_dir(&config.default_model, &config.data_dir)
                    .expect("failed to initialize embedding model"),
            );

            info!(
                model = %config.default_model,
                dim = raw_embedder.dimension(),
                "embedding model loaded"
            );

            let registry = CorpusRegistry::new(raw_embedder, config);
            let state = AppState::new(registry);

            // Share state with Tauri commands.
            app.manage(state.clone());

            // --- Start the UDS daemon in the background ---
            let daemon_state = state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = daemon::start(daemon_state).await {
                    tracing::error!(error = %e, "daemon failed");
                }
            });

            // --- System tray (initial static menu) ---
            build_initial_tray(app)?;

            // --- Auto-detect .iris.toml on first launch ---
            let detect_state = state.clone();
            let detect_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auto_detect_projects(&detect_state, &detect_handle).await;
            });

            // --- Dynamic tray menu refresh ---
            let refresh_state = state.clone();
            let refresh_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tray_refresh_loop(&refresh_state, &refresh_handle).await;
            });

            info!("iris app started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_corpora,
            commands::register_corpus,
            commands::unregister_corpus,
            commands::daemon_status,
            commands::add_project_dialog,
            commands::remove_project,
            commands::trigger_reindex,
            commands::is_autostart_enabled,
            commands::set_autostart,
            commands::read_logs,
            commands::should_show_onboarding,
            commands::dismiss_onboarding,
        ])
        .run(tauri::generate_context!())
        .expect("error while running iris app");
}

/// Build the initial tray icon with a static menu.
fn build_initial_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Show Dashboard").build(app)?;
    let add = MenuItemBuilder::with_id("add_project", "Add Project...").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit iris").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &add, &quit]).build()?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .tooltip("iris — index daemon")
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;

    Ok(())
}

/// Handle tray menu events.
fn handle_menu_event(app: &AppHandle, event_id: &str) {
    match event_id {
        "show" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        "add_project" => {
            // Trigger the file picker via Tauri command from Rust side.
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                commands::add_project_from_tray(&handle).await;
            });
        }
        "quit" => {
            let _ = std::fs::remove_file(iris_api::daemon_socket_path());
            let _ = std::fs::remove_file(iris_api::daemon_pid_path());
            app.exit(0);
        }
        id if id.starts_with("remove:") => {
            let corpus_id = id.strip_prefix("remove:").unwrap_or("").to_string();
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = commands::remove_project_by_id(&handle, &corpus_id).await {
                    tracing::warn!(error = %e, corpus_id, "failed to remove project");
                }
            });
        }
        _ => {}
    }
}

/// Refresh the tray menu and tooltip every 5 seconds with current corpus list.
async fn tray_refresh_loop(state: &AppState, handle: &AppHandle) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let corpora = state.registry.list().await;
        let count = corpora.len();

        // Update tooltip.
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let tooltip = if count == 0 {
                "iris — no projects".to_string()
            } else {
                format!(
                    "iris — {count} project{}",
                    if count == 1 { "" } else { "s" }
                )
            };
            let _: Result<(), _> = tray.set_tooltip(Some(&tooltip));

            // Rebuild menu with corpus entries.
            if let Err(e) = rebuild_tray_menu(handle, &tray, &corpora) {
                tracing::debug!(error = %e, "failed to rebuild tray menu");
            }
        }
    }
}

/// Rebuild the tray menu with current corpus list.
fn rebuild_tray_menu(
    handle: &AppHandle,
    tray: &TrayIcon,
    corpora: &[iris_api::corpus::CorpusInfo],
) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Show Dashboard").build(handle)?;
    let add = MenuItemBuilder::with_id("add_project", "Add Project...").build(handle)?;

    let mut builder = MenuBuilder::new(handle);
    builder = builder.items(&[&show, &add]);

    // Add separator and corpus entries if any.
    if !corpora.is_empty() {
        builder = builder.separator();
        for corpus in corpora {
            let status_icon = match corpus.status {
                iris_api::corpus::IndexingStatus::Idle => "●",
                iris_api::corpus::IndexingStatus::Indexing { .. } => "◌",
                iris_api::corpus::IndexingStatus::Error { .. } => "✕",
            };
            // Use last path component as display name.
            let display_name = corpus
                .paths
                .first()
                .and_then(|p| std::path::Path::new(p).file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(&corpus.id);
            let label = format!("{status_icon} {display_name}");
            let remove_id = format!("remove:{}", corpus.id);
            let item = MenuItemBuilder::with_id(&remove_id, &label).build(handle)?;
            builder = builder.item(&item);
        }
    }

    builder = builder.separator();
    let quit = MenuItemBuilder::with_id("quit", "Quit iris").build(handle)?;
    builder = builder.item(&quit);

    let menu = builder.build()?;
    tray.set_menu(Some(menu))?;
    tray.on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()));

    Ok(())
}

/// Scan common project directories for `.iris.toml` files on first launch.
async fn auto_detect_projects(state: &AppState, _handle: &AppHandle) {
    let sentinel = iris_api::daemon_socket_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("first_launch_done");

    if sentinel.exists() {
        return;
    }

    // Only scan if no corpora are currently registered.
    if !state.registry.list().await.is_empty() {
        let _ = std::fs::write(&sentinel, "");
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let scan_dirs = [
        format!("{home}/Code"),
        format!("{home}/Projects"),
        format!("{home}/Developer"),
        format!("{home}/src"),
    ];

    let mut found_paths = Vec::new();
    for dir in &scan_dirs {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        // Only scan one level deep.
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let toml_path = entry.path().join(".iris.toml");
                if toml_path.exists() {
                    found_paths.push(entry.path().display().to_string());
                }
            }
        }
    }

    for path in &found_paths {
        info!(path, "auto-detected project with .iris.toml");
        if let Err(e) = state.registry.register(std::slice::from_ref(path)).await {
            tracing::warn!(error = %e, path, "failed to auto-register project");
        }
    }

    let _ = std::fs::write(&sentinel, "");
}
