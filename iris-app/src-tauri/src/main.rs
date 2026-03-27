//! iris desktop app — Tauri v2 entry point.
//!
//! Starts the iris daemon (axum on UDS) alongside the Tauri webview.
//! The app runs as a system tray icon by default, with the main window
//! hidden until the user clicks the tray icon.

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod daemon;
mod registry;
mod state;

use std::sync::Arc;

use iris_core::config::IrisConfig;
use iris_core::embedding::{CachedEmbedder, FastEmbedder};
use iris_core::embedding::cache::EmbeddingCache;
use tauri::{
    Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};
use tracing::info;

use registry::CorpusRegistry;
use state::AppState;

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

            // --- System tray ---
            let show = MenuItemBuilder::with_id("show", "Show Dashboard").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit iris").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&menu)
                .tooltip("iris — index daemon")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // Clean up the UDS socket before quitting.
                        let socket = iris_api::daemon_socket_path();
                        let _ = std::fs::remove_file(socket);
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            info!("iris app started");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_corpora,
            commands::register_corpus,
            commands::unregister_corpus,
            commands::daemon_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running iris app");
}
