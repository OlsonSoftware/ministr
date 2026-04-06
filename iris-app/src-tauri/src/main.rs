//! iris desktop app — Tauri v2 entry point.
//!
//! Starts the iris daemon (axum on UDS) alongside the Tauri webview.
//! The app runs as a system tray icon by default, with the main window
//! hidden until the user clicks the tray icon.

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use iris_core::config::IrisConfig;
use iris_core::embedding;
use iris_daemon::daemon;
use iris_daemon::registry::CorpusRegistry;
use iris_daemon::state::AppState;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};
use tracing::info;

/// Tray icon ID used for lookups.
const TRAY_ID: &str = "iris-tray";

fn main() {
    // Initialize tracing to stderr + log file for the LogViewer tab.
    let log_path = iris_api::daemon_socket_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("iris.log");
    iris_core::tracing::init_tracing_with_file(&log_path);

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
            // Uses Candle Metal on macOS when supported, falls back to ONNX.
            let raw_embedder = embedding::create_embedder(&config.default_model, &config.data_dir)
                .expect("failed to initialize embedding model");

            info!(
                model = %config.default_model,
                dim = raw_embedder.dimension(),
                "embedding model loaded"
            );

            let registry = CorpusRegistry::new(raw_embedder, config);
            let state = AppState::new(registry);

            // Share state with Tauri commands.
            app.manage(state.clone());

            // --- Restore previously registered corpora ---
            let restore_state = state.clone();
            tauri::async_runtime::spawn(async move {
                restore_state.registry.restore().await;
            });

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

            // --- Periodically update tray tooltip with live stats ---
            let tooltip_state = state.clone();
            let tooltip_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    let corpora = tooltip_state.registry.list().await;
                    let total_sessions: usize = corpora.iter().map(|c| c.active_sessions).sum();
                    let rss = iris_core::mem_profile::rss_mb().unwrap_or(0.0);
                    let tooltip = format!(
                        "iris — {} corpora · {} sessions · {:.0} MB",
                        corpora.len(),
                        total_sessions,
                        rss,
                    );
                    if let Some(tray) = tooltip_handle.tray_by_id(TRAY_ID) {
                        let _ = tray.set_tooltip(Some(&tooltip));
                    }
                }
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
            commands::reset_onboarding,
            commands::list_sessions,
            commands::list_corpus_files,
            commands::search_corpus,
            commands::search_symbols,
            commands::symbol_references,
            commands::ingestion_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running iris app");
}

/// Build the initial tray icon with a static menu.
fn build_initial_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Show Dashboard").build(app)?;
    let add = MenuItemBuilder::with_id("add_project", "Add Project...").build(app)?;
    let logs = MenuItemBuilder::with_id("show_logs", "View Logs").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit iris").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&show, &add, &logs, &quit])
        .build()?;

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
        "show_logs" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            // Emit a navigation event the frontend listens for.
            let _ = app.emit("navigate", "logs");
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
        _ => {}
    }
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
        // Read .iris.toml and resolve paths so the corpus uses the configured
        // paths (e.g. ["src", "docs"]) rather than the bare project directory.
        let project_dir = std::path::Path::new(path);
        let resolved = iris_core::config::RepoConfig::discover(project_dir)
            .ok()
            .flatten()
            .map_or_else(
                || vec![path.clone()],
                |(base, rc)| rc.resolve_local_paths(&base),
            );

        if let Err(e) = state.registry.register(&resolved).await {
            tracing::warn!(error = %e, path, "failed to auto-register project");
        }
    }

    let _ = std::fs::write(&sentinel, "");
}
