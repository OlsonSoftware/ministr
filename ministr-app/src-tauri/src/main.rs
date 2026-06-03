//! ministr desktop app — Tauri v2 entry point.
//!
//! Ensures the headless `ministr __daemon` sidecar (which owns the UDS
//! socket) is running, then launches the Tauri webview. The daemon is a
//! separate process so it survives GUI close/restart; the GUI no longer
//! binds the socket in-process (see [`daemon_sidecar`]). The app runs as a
//! system tray icon by default, with the main window hidden until the user
//! clicks the tray icon.

// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod commands_cloud;
mod daemon_sidecar;
mod error;
mod setup;
mod tray;

use tauri::Manager;
use tracing::info;

#[allow(clippy::too_many_lines)] // entrypoint composes plugins + 50+ commands
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
        .plugin(tauri_plugin_opener::init())
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

            // --- Ensure the headless daemon sidecar owns the UDS socket ---
            // gd4: the GUI no longer builds an in-process AppState / embedder /
            // CorpusRegistry — it is a pure DaemonClient. The single daemon
            // lifecycle is: spawn the detached `ministr __daemon` sidecar if none
            // is alive (it survives GUI close and restores its own corpora), else
            // attach to the running one. Every Tauri command, the tray, and the
            // first-launch auto-detect talk to that daemon over UDS.
            if let Some(daemon_bin) = daemon_sidecar::resolve_daemon_binary(app) {
                tauri::async_runtime::spawn(daemon_sidecar::ensure_daemon_running(daemon_bin));
            } else {
                tracing::warn!(
                    "could not locate the ministr CLI binary to spawn the daemon sidecar — \
                     the UI will report the daemon as unreachable until one is on PATH"
                );
            }

            // --- System tray (initial placeholder menu; rebuilt live) ---
            tray::build_tray(app)?;

            // --- Auto-detect .ministr.toml on first launch ---
            // gd4: registers detected projects with the daemon over UDS.
            tauri::async_runtime::spawn(auto_detect_projects());

            // --- Periodic tray refresh (tooltip) ---
            tray::spawn_refresh_loop(app.handle().clone());

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
            commands::set_corpus_config,
            commands::list_supported_models,
            commands::repair_agent_config,
            commands::set_autostart,
            commands::read_logs,
            commands::should_show_onboarding,
            commands::dismiss_onboarding,
            commands::reset_onboarding,
            commands::setup_status,
            commands::fix_path,
            commands::list_sessions,
            commands::list_corpus_files,
            commands::search_corpus,
            commands::search_symbols,
            commands::symbol_references,
            commands::symbol_definition,
            commands::read_file,
            commands::file_occurrences,
            commands::bridge_query,
            commands::dead_code,
            commands::solid_findings,
            commands::diagnostics,
            commands::diff_impact,
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
            commands::mcp_detect_clients,
            commands::mcp_write_config,
            commands::mcp_test_connection,
            commands::linked_projects_list,
            commands::linked_project_add,
            commands::linked_project_add_dialog,
            commands::linked_project_remove,
            commands_cloud::cloud_status,
            commands_cloud::cloud_set_endpoint,
            commands_cloud::cloud_set_bearer_token,
            commands_cloud::cloud_authenticate,
            commands_cloud::cloud_authenticate_github,
            commands_cloud::cloud_disconnect,
            commands_cloud::cloud_health_check,
            commands_cloud::cloud_trigger_reindex,
            commands_cloud::cloud_billing_usage,
            commands_cloud::cloud_billing_checkout,
            commands_cloud::cloud_billing_portal,
            commands_cloud::cloud_list_corpora,
            commands_cloud::cloud_register_corpus,
            commands_cloud::cloud_clone_repo,
            commands_cloud::cloud_unregister_corpus,
            commands_cloud::cloud_corpus_progress,
            commands_cloud::cloud_list_orgs,
            commands_cloud::cloud_share_corpus,
            commands_cloud::cloud_list_corpus_shares,
            commands_cloud::cloud_revoke_corpus_share,
            commands_cloud::cloud_transfer_corpus_to_org,
            commands_cloud::cloud_transfer_personal_sub,
            commands_cloud::cloud_fetch_session_bundle,
            commands_cloud::cloud_list_sessions,
            commands_cloud::cloud_list_api_keys,
            commands_cloud::cloud_create_api_key,
            commands_cloud::cloud_revoke_api_key,
            commands_cloud::cloud_list_webhook_subs,
            commands_cloud::cloud_create_webhook_sub,
            commands_cloud::cloud_delete_webhook_sub,
            commands_cloud::cloud_test_webhook_sub,
            commands_cloud::cloud_get_org_usage,
            commands_cloud::cloud_export_org_usage_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ministr app");
}

/// Scan common project directories for `.ministr.toml` files on first launch.
async fn auto_detect_projects() {
    /// Mark first-launch auto-detect as done. A failed write isn't
    /// fatal (the scan just re-runs next launch) but must not be silent
    /// — a persistently unwritable data dir is worth surfacing.
    fn mark_done(sentinel: &std::path::Path) {
        if let Err(e) = std::fs::write(sentinel, "") {
            tracing::warn!(
                error = %e,
                path = %sentinel.display(),
                "failed to write first-launch sentinel; auto-detect will re-run next launch"
            );
        }
    }

    let sentinel = ministr_api::daemon_data_dir().join("first_launch_done");

    if sentinel.exists() {
        return;
    }

    // gd4: the corpus registry belongs to the daemon now — query + register
    // over UDS instead of an in-process registry.
    let client = ministr_api::client::DaemonClient::new();

    // Only scan if no corpora are currently registered. If the daemon isn't
    // reachable yet (startup race), skip WITHOUT marking done so the scan
    // retries on the next launch rather than silently never running.
    match client.list_corpora().await {
        Ok(corpora) if !corpora.is_empty() => {
            mark_done(&sentinel);
            return;
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "auto-detect: daemon unreachable, retrying next launch");
            return;
        }
    }

    // Reuse the shared, cross-platform scanner on a blocking thread so
    // the `read_dir`/`exists` syscalls don't stall the async runtime.
    // First-launch auto-detect excludes the bare home root (too broad
    // to scan unattended) — that's `include_home_root = false`.
    let found_paths: Vec<String> =
        match tokio::task::spawn_blocking(|| commands::scan_ministr_projects(false)).await {
            Ok(projects) => projects.into_iter().map(|p| p.path).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "project auto-detect scan task failed");
                mark_done(&sentinel);
                return;
            }
        };

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

        if let Err(e) = client.register_corpus(&resolved).await {
            tracing::warn!(error = %e, path, "failed to auto-register project");
        }
    }

    mark_done(&sentinel);
}
