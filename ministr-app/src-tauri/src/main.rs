//! ministr desktop app — Tauri v2 entry point.
//!
//! Starts the ministr daemon (axum on UDS) alongside the Tauri webview.
//! The app runs as a system tray icon by default, with the main window
//! hidden until the user clicks the tray icon.

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod setup;
mod tray;

use ministr_core::config::MinistrConfig;
use ministr_core::embedding;
use ministr_daemon::daemon;
use ministr_daemon::registry::CorpusRegistry;
use ministr_daemon::state::AppState;
use tauri::{AppHandle, Manager};
use tracing::info;

fn main() {
    // Initialize tracing to stderr + log file for the LogViewer tab.
    let log_path = ministr_api::daemon_data_dir().join("ministr.log");
    ministr_core::tracing::init_tracing_with_file(&log_path);

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
            // --- First-launch setup (install CLI, PATH, launchd) ---
            if let Err(e) = setup::run_first_launch_setup(app) {
                tracing::warn!(error = %e, "first-launch setup had errors");
            }

            // --- Initialize the ministr daemon ---
            let config = MinistrConfig::load(&MinistrConfig::default_path())
                .unwrap_or_else(|_| MinistrConfig::default());

            // Load embedding model (once for all corpora).
            // Uses Candle Metal on macOS when supported, falls back to ONNX.
            let (raw_embedder, backend_info) =
                embedding::create_embedder(&config.default_model, &config.data_dir)
                    .expect("failed to initialize embedding model");

            info!(
                model = %config.default_model,
                backend = ?backend_info.format,
                device = %backend_info.device,
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

            // --- System tray (initial placeholder menu; rebuilt live) ---
            tray::build_tray(app)?;

            // --- Auto-detect .ministr.toml on first launch ---
            let detect_state = state.clone();
            let detect_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auto_detect_projects(&detect_state, &detect_handle).await;
            });

            // --- Periodic tray refresh (tooltip + Recent/Indexing submenus) ---
            tray::spawn_refresh_loop(app.handle().clone(), state.clone());

            info!("ministr app started");
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
            commands::symbol_definition,
            commands::bridge_query,
            commands::read_source_excerpt,
            commands::open_path,
            commands::ingestion_progress,
            commands::indexing_progress_events,
            commands::recent_activity,
            commands::recent_coherence_events,
            commands::detect_projects,
            commands::register_projects_batch,
            commands::ask_corpus,
            commands::inference_health,
            commands::read_section,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ministr app");
}

/// Scan common project directories for `.ministr.toml` files on first launch.
async fn auto_detect_projects(state: &AppState, _handle: &AppHandle) {
    let sentinel = ministr_api::daemon_data_dir().join("first_launch_done");

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
                let toml_path = entry.path().join(".ministr.toml");
                if toml_path.exists() {
                    found_paths.push(entry.path().display().to_string());
                }
            }
        }
    }

    for path in &found_paths {
        info!(path, "auto-detected project with .ministr.toml");
        // Read .ministr.toml and resolve paths so the corpus uses the configured
        // paths (e.g. ["src", "docs"]) rather than the bare project directory.
        let project_dir = std::path::Path::new(path);
        let resolved = ministr_core::config::RepoConfig::discover(project_dir)
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
