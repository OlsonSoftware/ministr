//! Tauri IPC commands — bridge between the React frontend and Rust backend.

use serde::Serialize;

use ministr_api::activity::ActivityEvent;
use ministr_api::coherence::CoherenceEvent;
use ministr_api::corpus::{CorpusInfo, RegisterCorpusResponse};
use ministr_api::status::DaemonStatus;
use ministr_core::session::UsageLevel;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use ministr_daemon::state::AppState;

use crate::error::{CommandError, ErrorKind};

/// List all registered corpora.
///
/// gd2: routed over UDS to the headless daemon — the canonical registry —
/// rather than the GUI's in-process `AppState`.
#[tauri::command]
pub async fn list_corpora() -> Result<Vec<CorpusInfo>, CommandError> {
    ministr_api::client::DaemonClient::new()
        .list_corpora()
        .await
        .map_err(Into::into)
}

/// Register a new corpus by paths.
///
/// gd3: routed over UDS to the headless daemon — the single writer — so the
/// new corpus is immediately visible to `list_corpora` (gd2, sidecar) and to
/// MCP clients, instead of writing the GUI's in-process registry.
#[tauri::command]
pub async fn register_corpus(paths: Vec<String>) -> Result<RegisterCorpusResponse, CommandError> {
    ministr_api::client::DaemonClient::new()
        .register_corpus(&paths)
        .await
        .map_err(Into::into)
}

/// Unregister a corpus.
///
/// gd3: routed over UDS to the daemon registry (the single writer).
#[tauri::command]
pub async fn unregister_corpus(corpus_id: String) -> Result<(), CommandError> {
    ministr_api::client::DaemonClient::new()
        .unregister_corpus(&corpus_id)
        .await
        .map_err(Into::into)
}

/// Get daemon status (memory, uptime, corpora, autostart).
///
/// `autostart_enabled` is populated by querying the autolaunch plugin
/// directly so the React UI doesn't need a separate `is_autostart_enabled`
/// round-trip on every Settings mount.
#[tauri::command]
pub async fn daemon_status(app: AppHandle) -> Result<DaemonStatus, CommandError> {
    use tauri_plugin_autostart::ManagerExt;

    // gd2: status now reflects the headless daemon process (version, uptime,
    // RSS, model, corpora, sessions, log path) over UDS — not the GUI's
    // in-process AppState. `autostart_enabled` stays a GUI-local concern: the
    // autolaunch plugin lives in the tray app, and the headless daemon always
    // returns `None` for it, so the GUI overrides it here.
    let mut status = ministr_api::client::DaemonClient::new().status().await?;
    status.autostart_enabled = app.autolaunch().is_enabled().ok();
    Ok(status)
}

/// Open a directory picker dialog and register the selected directory as a corpus.
#[tauri::command]
pub async fn add_project_dialog(
    app: AppHandle,
) -> Result<Option<RegisterCorpusResponse>, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app.dialog().file().blocking_pick_folder();

    let Some(folder) = picked else {
        return Ok(None);
    };

    let path = folder.to_string();
    // gd3: register over UDS (the daemon is the single writer).
    let resp = ministr_api::client::DaemonClient::new()
        .register_corpus(&[path])
        .await?;
    Ok(Some(resp))
}

/// Remove a project and clean up its index data.
///
/// gd3: routed over UDS as a single daemon operation — `?purge=true` so the
/// daemon (which owns the corpus data directory) unregisters AND deletes the
/// on-disk index. The GUI no longer reaches into `~/.ministr/corpora`.
#[tauri::command]
pub async fn remove_project(corpus_id: String) -> Result<(), CommandError> {
    tracing::info!(corpus_id = %corpus_id, "remove_project called from frontend");
    ministr_api::client::DaemonClient::new()
        .unregister_corpus_purge(&corpus_id)
        .await
        .map_err(Into::into)
}

/// One linked project as shown in the Linked Projects panel.
#[derive(Serialize)]
pub struct LinkedProjectOut {
    /// The exact `path` string stored in `.ministr.toml` (used as the
    /// stable key when removing the entry).
    pub path: String,
    /// Explicit label from config, if any.
    pub label: Option<String>,
    /// Effective label the agent targets (explicit, else the dir name).
    pub resolved_label: String,
    /// Whether the linked root currently exists on disk.
    pub exists: bool,
}

/// List the `[[linked]]` projects declared in `project_root`'s
/// `.ministr.toml`. Returns an empty list when there's no config file.
#[tauri::command]
pub async fn linked_projects_list(
    project_root: String,
) -> Result<Vec<LinkedProjectOut>, CommandError> {
    let root = std::path::Path::new(&project_root);
    let Some((_, cfg)) = ministr_core::config::RepoConfig::discover(root)
        .map_err(|e| CommandError::internal(e.to_string()))?
    else {
        return Ok(Vec::new());
    };

    Ok(cfg
        .linked
        .iter()
        .map(|l| {
            let expanded = expand_tilde(&l.path);
            let resolved_label = match l.label.as_deref() {
                Some(s) if !s.trim().is_empty() => s.trim().to_string(),
                _ => std::path::Path::new(&expanded)
                    .file_name()
                    .map_or_else(|| l.path.clone(), |n| n.to_string_lossy().to_string()),
            };
            LinkedProjectOut {
                path: l.path.clone(),
                label: l.label.clone(),
                resolved_label,
                exists: std::path::Path::new(&expanded).is_dir(),
            }
        })
        .collect())
}

/// Add (or update) a linked project in `project_root`'s `.ministr.toml`.
///
/// Format-preserving and idempotent on `path` — see
/// [`RepoConfig::add_linked_project`](ministr_core::config::RepoConfig::add_linked_project).
#[tauri::command]
pub async fn linked_project_add(
    project_root: String,
    path: String,
    label: Option<String>,
) -> Result<(), CommandError> {
    let root = std::path::Path::new(&project_root);
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(CommandError::invalid_input("linked project path is empty"));
    }
    let label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    ministr_core::config::RepoConfig::add_linked_project(root, trimmed, label.as_deref())
        .map_err(|e| CommandError::internal(e.to_string()))?;
    tracing::info!(project_root = %project_root, linked = %trimmed, "added linked project");
    Ok(())
}

/// Open a folder picker and link the chosen directory into
/// `project_root`. Returns the new entry, or `None` if the user cancelled.
#[tauri::command]
pub async fn linked_project_add_dialog(
    app: AppHandle,
    project_root: String,
) -> Result<Option<LinkedProjectOut>, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    // `blocking_pick_folder` parks the calling thread while the native
    // dialog is open (potentially many seconds). Off-load it to the
    // blocking pool so the async runtime's worker threads stay free.
    let app_for_dialog = app.clone();
    let picked =
        tokio::task::spawn_blocking(move || app_for_dialog.dialog().file().blocking_pick_folder())
            .await
            .map_err(|e| CommandError::internal(format!("folder picker task failed: {e}")))?;
    let Some(folder) = picked else {
        return Ok(None);
    };
    let picked = folder.to_string();
    let resolved_label = std::path::Path::new(&picked)
        .file_name()
        .map_or_else(|| picked.clone(), |n| n.to_string_lossy().to_string());

    let root = std::path::Path::new(&project_root);
    ministr_core::config::RepoConfig::add_linked_project(root, &picked, None)
        .map_err(|e| CommandError::internal(e.to_string()))?;
    tracing::info!(project_root = %project_root, linked = %picked, "linked project via dialog");

    Ok(Some(LinkedProjectOut {
        path: picked.clone(),
        label: None,
        resolved_label,
        exists: std::path::Path::new(&picked).is_dir(),
    }))
}

/// Remove a linked project by its stored `path`. Returns `true` if an
/// entry was removed.
#[tauri::command]
pub async fn linked_project_remove(
    project_root: String,
    path: String,
) -> Result<bool, CommandError> {
    let root = std::path::Path::new(&project_root);
    let removed = ministr_core::config::RepoConfig::remove_linked_project(root, &path)
        .map_err(|e| CommandError::internal(e.to_string()))?;
    tracing::info!(project_root = %project_root, linked = %path, removed, "removed linked project");
    Ok(removed)
}

/// Trigger a full re-index of a corpus.
///
/// gd3: routed over UDS to the daemon's reindex endpoint (gd3a), which
/// purges the on-disk index and re-registers daemon-side so the config is
/// re-resolved and the corpus re-embedded from scratch.
#[tauri::command]
pub async fn trigger_reindex(corpus_id: String) -> Result<(), CommandError> {
    tracing::info!(corpus_id = %corpus_id, "trigger_reindex called from frontend");
    ministr_api::client::DaemonClient::new()
        .reindex_corpus(&corpus_id)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

/// Persist a corpus's per-corpus config (`model` / `dimension` /
/// `rerank_depth`) to its `.ministr.toml` `[corpus]` table — the SAME file the
/// CLI and the daemon's per-corpus config seam read — then re-index so the
/// change takes effect.
///
/// parity-gui-corpus-config-write-seam: lets a GUI-only user opt a corpus into
/// e.g. a code-specialised model (`jina-embeddings-v2-base-code`) or Matryoshka
/// truncation, and have the daemon actually honor it — model via the registry
/// embedder pool, `dimension` + `rerank_depth` via the registry's Matryoshka wiring
/// — with no write-then-ignore footgun. A `None` argument leaves that field
/// untouched. The write is format-preserving (see
/// [`RepoConfig::set_corpus_config`](ministr_core::config::RepoConfig::set_corpus_config)).
#[tauri::command]
pub async fn set_corpus_config(
    corpus_id: String,
    model: Option<String>,
    dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<(), CommandError> {
    let client = ministr_api::client::DaemonClient::new();

    // The `.ministr.toml` lives at the corpus's repo root. Derive it from the
    // first registered path: a directory path *is* the root; a file path's
    // parent is. gd3: the paths come from the daemon over UDS (its registry is
    // the single source of truth) rather than an in-process handle.
    let paths = client.corpus_status(&corpus_id).await?.paths;
    let first = paths.first().ok_or_else(|| {
        CommandError::invalid_input(format!("corpus '{corpus_id}' has no paths to configure"))
    })?;
    let p = std::path::Path::new(first);
    let repo_root = if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent().map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        )
    };

    ministr_core::config::RepoConfig::set_corpus_config(
        &repo_root,
        model.as_deref(),
        dimension,
        rerank_depth,
    )
    .map_err(|e| CommandError::internal(e.to_string()))?;
    tracing::info!(
        corpus_id = %corpus_id,
        repo_root = %repo_root.display(),
        ?model,
        ?dimension,
        ?rerank_depth,
        "persisted per-corpus config to .ministr.toml — re-indexing"
    );

    // Re-index over UDS so the daemon re-resolves the new config and re-embeds.
    client
        .reindex_corpus(&corpus_id)
        .await
        .map(|_| ())
        .map_err(Into::into)
}

/// One supported embedding model, surfaced to the GUI's per-corpus model
/// picker (parity-gui-corpus-config-ui). Sourced from the single core
/// [`supported_models`](ministr_core::embedding::supported_models) list so the
/// dropdown can't drift from what the daemon can actually load.
#[derive(Serialize)]
pub struct SupportedModel {
    /// CLI/config name (the value written to `.ministr.toml` `[corpus] model`).
    pub name: String,
    /// Native output vector dimensionality.
    pub dimension: usize,
    /// Short human-readable description.
    pub description: String,
    /// Whether this model is optimised for source code.
    pub code_optimized: bool,
}

/// List the embedding models a corpus can be configured to use.
///
/// The GUI's per-corpus model picker reads this so its options stay in lockstep
/// with [`ministr_core::embedding::supported_models`] — the same list the CLI
/// and daemon validate against — rather than a hand-maintained TS list.
#[tauri::command]
pub fn list_supported_models() -> Vec<SupportedModel> {
    ministr_core::embedding::supported_models()
        .iter()
        .map(|m| SupportedModel {
            name: m.name.to_string(),
            dimension: m.dimension,
            description: m.description.to_string(),
            code_optimized: m.code_optimized,
        })
        .collect()
}

/// Result of an agent-config repair pass.
#[derive(Serialize)]
pub struct RepairReport {
    /// The project roots that were scaffolded/healed.
    pub roots: Vec<String>,
    /// Newly created files (were missing).
    pub created: usize,
    /// Stale machine-generated hook files overwritten with the current template.
    pub healed: usize,
    /// Custom rules injected from `.ministr.toml [agent] rules`.
    pub custom_rules: usize,
}

/// Idempotently repair every AI-assistant config file for all registered
/// corpora.
///
/// For each unique local corpus root this (re)writes the full agent
/// configuration set via `ministr_core::scaffold::scaffold_agent_config`:
/// `.claude/` rules + `settings.json` + the `steer-to-ministr.sh` hook
/// script, Cursor / Windsurf / Continue / Copilot hooks and rules, and
/// `AGENTS.md`. It is **idempotent and non-destructive**: advisory `.md`
/// files are created only if missing (never overwritten), machine hook
/// files are healed only when their content drifts from the current
/// template, and `.claude/settings.json` is *merged* — unrelated user
/// keys (e.g. `permissions`) are preserved; only the `hooks` key is
/// replaced. Nested sub-paths of an already-included root are skipped so
/// config is written once per project, not scattered into subdirectories.
#[tauri::command]
pub async fn repair_agent_config(state: State<'_, AppState>) -> Result<RepairReport, CommandError> {
    use ministr_core::config::{CorpusSource, classify_corpus_path};

    let corpora = state.registry.list().await;
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for c in &corpora {
        for p in &c.paths {
            if let CorpusSource::Local(pb) = classify_corpus_path(p)
                && pb.is_dir()
            {
                // Prefer the filesystem-canonical form so equivalent
                // spellings of the same project collapse to one root.
                roots.push(std::fs::canonicalize(&pb).unwrap_or(pb));
            }
        }
    }
    roots.sort();
    roots.dedup();
    // Drop any root nested under another — scaffold once per project.
    let mut top: Vec<std::path::PathBuf> = Vec::new();
    for r in roots {
        if !top.iter().any(|a| r.starts_with(a)) {
            top.push(r);
        }
    }
    if top.is_empty() {
        return Err(CommandError::invalid_input(
            "no local corpus roots registered to repair",
        ));
    }

    let report = tokio::task::spawn_blocking(move || {
        let mut created = 0;
        let mut healed = 0;
        let mut custom_rules = 0;
        let mut done = Vec::with_capacity(top.len());
        for root in &top {
            let res = ministr_core::scaffold::scaffold_agent_config(root);
            created += res.created;
            healed += res.healed;
            custom_rules += res.custom_rules;
            done.push(root.display().to_string());
        }
        RepairReport {
            roots: done,
            created,
            healed,
            custom_rules,
        }
    })
    .await
    .map_err(|e| CommandError::internal(format!("repair task failed to join: {e}")))?;

    tracing::info!(
        roots = report.roots.len(),
        created = report.created,
        healed = report.healed,
        custom_rules = report.custom_rules,
        "repair_agent_config completed"
    );
    Ok(report)
}

/// Add a project from the tray menu (called from Rust, not from JS).
pub async fn add_project_from_tray(handle: &AppHandle) {
    use tauri_plugin_dialog::DialogExt;

    let picked = handle.dialog().file().blocking_pick_folder();

    let Some(folder) = picked else {
        return;
    };

    let path = folder.to_string();
    // gd3: register over UDS so the tray-added corpus lands in the single
    // daemon registry (visible to list_corpora + MCP), not the GUI's
    // in-process one.
    match ministr_api::client::DaemonClient::new()
        .register_corpus(std::slice::from_ref(&path))
        .await
    {
        Ok(resp) => {
            tracing::info!(corpus_id = resp.corpus_id, path, "project added from tray");
        }
        Err(e) => {
            tracing::warn!(error = %e, path, "failed to add project from tray");
        }
    }
}

/// Enable or disable auto-start at login.
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager
            .enable()
            .map_err(|e| CommandError::internal(e.to_string()))
    } else {
        manager
            .disable()
            .map_err(|e| CommandError::internal(e.to_string()))
    }
}

/// Read the last N lines from the daemon log file.
#[tauri::command]
pub async fn read_logs(lines: Option<usize>) -> Result<Vec<String>, CommandError> {
    let max_lines = lines.unwrap_or(200);
    let log_path = ministr_api::daemon_data_dir().join("ministr.log");

    if !log_path.exists() {
        return Ok(vec!["No log file found.".to_string()]);
    }

    let content = std::fs::read_to_string(&log_path).map_err(CommandError::from)?;
    let all_lines: Vec<String> = content.lines().map(String::from).collect();
    let start = all_lines.len().saturating_sub(max_lines);
    Ok(all_lines[start..].to_vec())
}

/// Check if first-run onboarding should be shown.
#[tauri::command]
pub async fn should_show_onboarding() -> Result<bool, CommandError> {
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    Ok(!sentinel.exists())
}

/// First-run setup state, surfaced to the branded setup wizard so it can
/// show real status (and a "Fix PATH" affordance) instead of guessing.
/// Rendered identically on macOS / Windows / Linux.
#[derive(Serialize)]
pub struct SetupStatus {
    /// A working `ministr` binary is resolvable (canonical bin dir,
    /// `/usr/local/bin`, or on `PATH`).
    pub cli_on_path: bool,
    /// Absolute path to the resolved CLI, when found.
    pub cli_path: Option<String>,
    /// `~/.ministr` — where corpora, the daemon socket, and markers live.
    pub data_dir: String,
    /// The app/CLI version this build expects.
    pub version: String,
}

/// True only if `p` is a regular file we could actually execute — not a
/// directory, and on Unix it must carry an exec bit. Guards against
/// reporting `cli_on_path = true` for a path that `fix_path` would then
/// fail to spawn.
fn is_usable_cli(p: &std::path::Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Resolve a usable `ministr` CLI the same way the rest of the app does:
/// canonical `~/.ministr/bin`, the `.pkg`/Linux-package symlink at
/// `/usr/local/bin`, then `PATH`. Every candidate must pass
/// [`is_usable_cli`]; the `PATH` lookup reuses [`which_on_path`], which
/// is `PATHEXT`-aware (so a Windows `.cmd`/`.bat` shim resolves with its
/// real extension and `fix_path` can route it through `cmd /c`).
fn resolve_cli_path() -> Option<std::path::PathBuf> {
    let bin_dir = ministr_api::daemon_data_dir().join("bin");

    // On Windows the first-run `install_cli_binary` (setup.rs) stages the
    // sidecar as `ministr` (no extension), while a native installer drops
    // `ministr.exe` — probe both, mirroring setup.rs::ensure_path, so we
    // don't false-negative "CLI not found".
    let mut candidates = if cfg!(windows) {
        vec![bin_dir.join("ministr.exe"), bin_dir.join("ministr")]
    } else {
        vec![bin_dir.join("ministr")]
    };
    if !cfg!(windows) {
        candidates.push(std::path::PathBuf::from("/usr/local/bin/ministr"));
    }
    for c in candidates {
        if is_usable_cli(&c) {
            return Some(c);
        }
    }

    // Last resort: PATH. which_on_path is PATHEXT-aware on Windows, but
    // only guarantees is_file() — re-run is_usable_cli so a
    // non-executable Unix file on PATH can't make cli_on_path=true and
    // then EACCES in fix_path.
    which_on_path("ministr")
        .map(std::path::PathBuf::from)
        .filter(|p| is_usable_cli(p))
}

/// Report first-run setup state for the setup wizard.
#[tauri::command]
pub async fn setup_status() -> Result<SetupStatus, CommandError> {
    let cli = resolve_cli_path();
    Ok(SetupStatus {
        cli_on_path: cli.is_some(),
        cli_path: cli.map(|p| p.display().to_string()),
        data_dir: ministr_api::daemon_data_dir().display().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Wire `ministr` onto the user's PATH by invoking the CLI's own `setup`
/// subcommand (the `onpath` crate — same surface the installers use, so
/// re-running is idempotent). Backs the wizard's "Fix PATH" action.
#[tauri::command]
pub async fn fix_path() -> Result<String, CommandError> {
    let cli = resolve_cli_path().ok_or_else(|| CommandError {
        kind: ErrorKind::NotFound,
        message: "ministr CLI not found — reinstall the app and try again".to_string(),
    })?;

    // A `.cmd`/`.bat` shim (possible when resolved off PATH on Windows)
    // can't be spawned directly — Rust ≥1.77 hard-errors; it must go via
    // `cmd /c`. Mirrors `test_via_cli`. Suppress the console flash from
    // this GUI process.
    let mut command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let is_script = cli
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        if is_script {
            command = std::process::Command::new("cmd");
            command.arg("/c").arg(&cli).arg("setup");
        } else {
            command = std::process::Command::new(&cli);
            command.arg("setup");
        }
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        command = std::process::Command::new(&cli);
        command.arg("setup");
    }

    let out = command.output().map_err(CommandError::from)?;
    if !out.status.success() {
        return Err(CommandError {
            kind: ErrorKind::Internal,
            message: format!(
                "`ministr setup` failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Dismiss the onboarding screen.
#[tauri::command]
pub async fn dismiss_onboarding() -> Result<(), CommandError> {
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    std::fs::write(&sentinel, "").map_err(CommandError::from)
}

/// Reset onboarding so it shows again on next visit.
#[tauri::command]
pub async fn reset_onboarding() -> Result<(), CommandError> {
    let sentinel = ministr_api::daemon_data_dir().join("onboarding_done");
    if sentinel.exists() {
        std::fs::remove_file(&sentinel).map_err(CommandError::from)?;
    }
    Ok(())
}

/// Detected project for onboarding.
#[derive(Serialize)]
pub struct DetectedProject {
    pub path: String,
    pub name: String,
}

/// The directories scanned for `.ministr.toml` projects, cross-platform
/// (`HOME` on Unix, `USERPROFILE` on Windows via [`home_pathbuf`]).
pub(crate) fn project_scan_dirs() -> Vec<std::path::PathBuf> {
    let Some(home) = home_pathbuf() else {
        return Vec::new();
    };
    vec![
        home.join("Code"),
        home.join("Projects"),
        home.join("Developer"),
        home.join("src"),
    ]
}

/// Synchronous filesystem scan for projects containing a `.ministr.toml`.
///
/// Blocking by nature (`read_dir`/`exists`); callers in async contexts
/// must run this on a blocking thread (`spawn_blocking`). When
/// `include_home_root` is set, the user's home directory is also scanned
/// one level deep (used by the interactive picker, not first-launch
/// auto-detect, which would be too broad unattended).
pub(crate) fn scan_ministr_projects(include_home_root: bool) -> Vec<DetectedProject> {
    let mut scan_dirs = project_scan_dirs();
    if include_home_root && let Some(home) = home_pathbuf() {
        scan_dirs.insert(0, home);
    }
    let home = home_pathbuf();

    let mut found = Vec::new();
    for dir_path in &scan_dirs {
        if !dir_path.is_dir() {
            continue;
        }
        // Check the directory itself for .ministr.toml (but never treat
        // the bare home dir as a project).
        if home.as_deref() != Some(dir_path.as_path()) && dir_path.join(".ministr.toml").exists() {
            let name = dir_path.file_name().map_or_else(
                || dir_path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            found.push(DetectedProject {
                path: dir_path.display().to_string(),
                name,
            });
            continue;
        }
        // Scan one level deep.
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

    // Deduplicate by path.
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found.dedup_by(|a, b| a.path == b.path);
    found
}

/// Scan common directories for projects with `.ministr.toml` files.
#[tauri::command]
pub async fn detect_projects() -> Result<Vec<DetectedProject>, CommandError> {
    // The scan does blocking `read_dir`/`exists` syscalls — keep them
    // off the async runtime threads.
    tokio::task::spawn_blocking(|| scan_ministr_projects(true))
        .await
        .map_err(|e| CommandError::internal(format!("project scan task failed: {e}")))
}

/// Register multiple projects at once (for onboarding batch import).
///
/// `register` is idempotent on canonical identity and never touches an
/// unrelated corpus's state, so registering a sibling project will never
/// destroy a neighbour's sessions. Per-path errors are warned and skipped.
#[tauri::command]
pub async fn register_projects_batch(paths: Vec<String>) -> Result<Vec<String>, CommandError> {
    // gd3: each registration round-trips through the daemon over UDS (the
    // single writer); the local `.ministr.toml` discovery/resolution is
    // unchanged.
    let client = ministr_api::client::DaemonClient::new();
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
        match client.register_corpus(&resolved).await {
            Ok(resp) => registered.push(resp.corpus_id),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to register project in batch");
            }
        }
    }
    Ok(registered)
}

/// Remove a project by ID (called from tray menu).
#[allow(dead_code)]
pub async fn remove_project_by_id(handle: &AppHandle, corpus_id: &str) -> Result<(), CommandError> {
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
        ministr_core::fs_util::remove_dir_all_robust(&dir)
            .await
            .map_err(|e| {
                tracing::error!(path = %dir.display(), error = %e, "failed to delete corpus data from tray remove");
                format!("failed to delete corpus data at {}: {e}", dir.display())
            })?;
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
    #[serde(rename = "pressure_level")]
    pub level: String,
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
    /// Tokens freed by eviction vs compression — the token-level split
    /// behind `total_tokens_saved` (UI economics bar).
    pub cumulative_tokens_evicted: u64,
    pub cumulative_tokens_compressed: u64,
    /// Deliveries that changed since last seen (delta updates).
    pub delta_updates: u64,
    pub dedup_hits: u64,
    pub compression_ratio: f64,
    // Budget configuration — lets the UI derive pressure / projections
    // from the *real* (env-driven) window + thresholds instead of
    // hardcoding 0.80 / 0.95.
    pub context_window_tokens: usize,
    pub pressure_threshold: f64,
    pub critical_threshold: f64,
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
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionDetail>, CommandError> {
    let guard = state.registry.corpora().read().await;
    let mut sessions = Vec::new();

    for (corpus_id, handle) in guard.iter() {
        let reg = handle.sessions.lock().await;
        for sid in reg.session_ids() {
            if let Some(entry) = reg.get_session(&sid) {
                let status = entry.budget.usage_status();
                let metrics = entry.session.metrics();
                let cfg = entry.budget.config();
                #[allow(clippy::cast_precision_loss)]
                let compression_ratio = if metrics.cumulative_tokens_delivered > 0 {
                    metrics.total_tokens_saved() as f64 / metrics.cumulative_tokens_delivered as f64
                } else {
                    0.0
                };
                sessions.push(SessionDetail {
                    session_id: sid.clone(),
                    corpus_id: corpus_id.clone(),
                    level: match entry.budget.level() {
                        UsageLevel::Normal => "normal",
                        UsageLevel::Elevated => "elevated",
                        UsageLevel::Critical => "critical",
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
                    cumulative_tokens_evicted: metrics.cumulative_tokens_evicted,
                    cumulative_tokens_compressed: metrics.cumulative_tokens_compressed,
                    delta_updates: metrics.delta_updates,
                    dedup_hits: metrics.dedup_hits,
                    compression_ratio,
                    context_window_tokens: cfg.max_context_tokens,
                    pressure_threshold: cfg.pressure_threshold,
                    critical_threshold: cfg.critical_threshold,
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
///
/// gd2c: routed over UDS to the daemon's files endpoint (the daemon owns the
/// storage); maps the API `FileInfo` onto the frontend DTO field-for-field.
#[tauri::command]
pub async fn list_corpus_files(corpus_id: String) -> Result<Vec<FileInfo>, CommandError> {
    let files = ministr_api::client::DaemonClient::new()
        .list_corpus_files(&corpus_id)
        .await?;

    Ok(files
        .into_iter()
        .map(|f| FileInfo {
            path: f.path,
            content_hash: f.content_hash,
            mtime_ns: f.mtime_ns,
            section_count: f.section_count,
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

/// Search a corpus by query.
///
/// gd2c: routed over UDS to the daemon's survey endpoint (the single source
/// of truth) rather than the GUI's in-process `QueryService`.
#[tauri::command]
pub async fn search_corpus(
    corpus_id: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<SearchResult>, CommandError> {
    let resp = ministr_api::client::DaemonClient::new()
        .survey(&corpus_id, &query, top_k)
        .await?;

    Ok(resp
        .results
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
///
/// gd2c: routed over UDS to the daemon's symbols endpoint. The daemon encodes
/// `module_path` into the API `SymbolDefinition.heading_path` (split on `::`),
/// so it is recovered here by re-joining. `file_path` is honored server-side
/// (gd2c-2 added the filter to `SymbolsRequest`).
#[tauri::command]
pub async fn search_symbols(
    corpus_id: String,
    query: String,
    kind: Option<String>,
    file_path: Option<String>,
) -> Result<Vec<SymbolInfo>, CommandError> {
    let req = ministr_api::query::SymbolsRequest {
        query,
        kind,
        module: None,
        visibility: None,
        file_path,
        limit: None,
        session_id: None,
    };
    let resp = ministr_api::client::DaemonClient::new()
        .symbols(&corpus_id, &req)
        .await?;

    Ok(resp
        .symbols
        .into_iter()
        .map(|s| SymbolInfo {
            id: s.id,
            name: s.name,
            kind: s.kind,
            file_path: s.file_path,
            visibility: s.visibility,
            signature: s.signature,
            doc_comment: s.doc_comment,
            module_path: s.heading_path.join("::"),
        })
        .collect())
}

/// A symbol's clickable span within a file (1-based, inclusive line range).
///
/// `signature` + `doc_comment` ride along so the code browser can render a
/// hovercard without a second round-trip to `symbol_definition`.
#[derive(Serialize)]
pub struct SymbolSpan {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
}

/// A source file's full contents plus the symbol spans the index knows for it.
///
/// Returned by [`read_file`]; the code browser renders `content` with Shiki
/// (using `lang`) and overlays `symbol_spans` as clickable, navigable hot-zones.
#[derive(Serialize)]
pub struct FileContent {
    pub path: String,
    pub lang: String,
    pub content: String,
    pub symbol_spans: Vec<SymbolSpan>,
}

/// Map a file path's extension to a Shiki language id for highlighting.
///
/// Single source of truth for language inference (the frontend just forwards
/// the result to Shiki). Unknown extensions fall back to `"text"`.
fn lang_from_path(path: &str) -> String {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "xml" => "xml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "sql" => "sql",
        "md" | "markdown" => "markdown",
        "lua" => "lua",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "hs" => "haskell",
        _ => "text",
    };
    lang.to_string()
}

/// Read an indexed source file: full contents, a Shiki language id inferred
/// from the extension, and the definition spans the symbol index knows for the
/// file (so the UI can render clickable, navigable symbols).
/// gd2c: routed over UDS — the daemon returns the file content + its symbol
/// spans (it owns the storage + path resolution); `lang_from_path` stays
/// app-local (the Shiki language id is a pure function of the extension).
#[tauri::command]
pub async fn read_file(corpus_id: String, path: String) -> Result<FileContent, CommandError> {
    let resp = ministr_api::client::DaemonClient::new()
        .read_file_content(&corpus_id, path.clone())
        .await?;

    let symbol_spans = resp
        .symbols
        .into_iter()
        .map(|s| SymbolSpan {
            id: s.id,
            name: s.name,
            kind: s.kind,
            signature: s.signature,
            doc_comment: s.doc_comment,
            line_start: s.line_start,
            line_end: s.line_end,
        })
        .collect();

    Ok(FileContent {
        lang: lang_from_path(&path),
        path,
        content: resp.content,
        symbol_spans,
    })
}

/// One resolved identifier occurrence (F-CodeExplorer v2), for click-any-token.
#[derive(Serialize)]
pub struct Occurrence {
    pub symbol_id: String,
    pub name: String,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line: u32,
    pub col: u32,
}

/// File-level occurrence index: every resolved identifier site in the file.
///
/// Empty unless the corpus was indexed with occurrence indexing enabled
/// (`MINISTR_INDEX_OCCURRENCES`). When present it lets the Code surface
/// resolve a click on *any* token, not just known definitions.
/// gd2c: routed over UDS to the daemon's occurrences endpoint.
#[tauri::command]
pub async fn file_occurrences(
    corpus_id: String,
    path: String,
) -> Result<Vec<Occurrence>, CommandError> {
    let records = ministr_api::client::DaemonClient::new()
        .file_occurrences(&corpus_id, path)
        .await?;

    Ok(records
        .into_iter()
        .map(|r| Occurrence {
            symbol_id: r.symbol_id,
            name: r.name,
            byte_start: r.byte_start,
            byte_end: r.byte_end,
            line: r.line,
            col: r.col,
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
///
/// gd2c: routed over UDS to the daemon's references endpoint.
#[tauri::command]
pub async fn symbol_references(
    corpus_id: String,
    symbol_id: String,
) -> Result<Vec<SymbolRef>, CommandError> {
    let resp = ministr_api::client::DaemonClient::new()
        .references(&corpus_id, &symbol_id, None)
        .await?;

    Ok(resp
        .references
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
    limit: Option<usize>,
    since_ms: Option<u64>,
) -> Result<Vec<CoherenceEvent>, CommandError> {
    // gd2: read the daemon's coherence (file-change) ring over UDS rather
    // than the GUI's in-process buffer. The daemon applies limit/since;
    // default 50 to match the prior in-process behavior.
    Ok(ministr_api::client::DaemonClient::new()
        .recent_coherence_events(Some(limit.unwrap_or(50)), since_ms)
        .await?
        .events)
}

/// Snapshot recent tool-call activity events from the in-process ring buffer.
///
/// Mirrors the daemon's `/activity` HTTP endpoint for the Tauri frontend —
/// when the Tauri app runs in-process it consults [`AppState::activity`]
/// directly rather than hopping over UDS.
#[tauri::command]
pub async fn recent_activity(
    limit: Option<usize>,
    since_ms: Option<u64>,
    session_id: Option<String>,
) -> Result<Vec<ActivityEvent>, CommandError> {
    let limit = limit.unwrap_or(50);
    // gd2: read the daemon's activity ring over UDS. After gd1 the GUI's
    // in-process ring is empty (MCP traffic hits the sidecar), so this is
    // where the real timeline lives. The daemon applies the limit/since
    // filtering server-side and returns newest-first.
    let events = ministr_api::client::DaemonClient::new()
        .recent_activity(Some(limit), since_ms)
        .await?
        .events;
    // Per-session filter. Every event the MCP proxy generates is now
    // stamped with its originating session via the X-Ministr-Session-Id
    // header (read in the daemon's activity middleware), so a strict
    // session_id match returns the complete timeline for this session.
    let events = match session_id {
        Some(sid) => events
            .into_iter()
            .filter(|e| e.session_id.as_deref() == Some(sid.as_str()))
            .collect(),
        None => events,
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
///
/// gd2c: routed over UDS to the daemon's bridge endpoint. The daemon applies
/// the limit (default 500, matching the prior client-side cap). The API's
/// `BridgeLink` carries binding keys as `source`/`target` and languages as
/// `source_language`/`target_language`, mapped here onto the frontend's
/// `export_*`/`import_*` DTO fields.
#[tauri::command]
pub async fn bridge_query(
    corpus_id: String,
    query: Option<String>,
    kind: Option<String>,
    source_language: Option<String>,
    file_path: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<BridgeLinkOut>, CommandError> {
    let req = ministr_api::query::BridgeRequest {
        query,
        kind,
        source_language,
        file_path,
        limit: Some(limit.unwrap_or(500)),
        session_id: None,
    };
    let resp = ministr_api::client::DaemonClient::new()
        .bridge(&corpus_id, &req)
        .await?;

    Ok(resp
        .links
        .into_iter()
        .map(|l| BridgeLinkOut {
            kind: l.kind,
            confidence: l.confidence,
            export_file: l.export_file,
            export_binding_key: l.source,
            export_symbol: l.export_symbol,
            export_language: l.source_language,
            export_line: l.export_line,
            import_file: l.import_file,
            import_binding_key: l.target,
            import_symbol: l.import_symbol,
            import_language: l.target_language,
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
pub async fn open_path(path: String) -> Result<(), CommandError> {
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
        // Normalize the remainder's separators to the platform's so a
        // mixed `~/a\b` / `~\a/b` input doesn't yield a path the OS
        // can't resolve.
        let rest = if cfg!(windows) {
            rest.replace('/', "\\")
        } else {
            rest.replace('\\', "/")
        };
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
    corpus_id: String,
    file_path: String,
    line_start: u32,
    line_end: u32,
) -> Result<String, CommandError> {
    // gd2c: the corpus's root paths (which bound the path-scope security
    // check below) come from the daemon over UDS, not the in-process
    // registry. The canonicalize + scope check + line read stay app-local.
    let roots: Vec<String> = ministr_api::client::DaemonClient::new()
        .corpus_status(&corpus_id)
        .await?
        .paths;

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
        return Err(CommandError::invalid_input("path outside corpus"));
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
///
/// gd2c: routed over UDS to the daemon's definition endpoint.
#[tauri::command]
pub async fn symbol_definition(
    corpus_id: String,
    symbol_id: String,
) -> Result<SymbolDefinitionOut, CommandError> {
    let def = ministr_api::client::DaemonClient::new()
        .definition(&corpus_id, &symbol_id, None)
        .await?;

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
///
/// gd2b: routed over UDS to the daemon's all-corpora progress snapshot
/// (`GET /api/v1/progress`). The daemon owns the indexer now, so its in-memory
/// `IngestionProgress` is the source of truth; the app just maps the API type
/// onto the frontend DTO field-for-field.
#[tauri::command]
pub async fn ingestion_progress() -> Result<Vec<IngestionProgressInfo>, CommandError> {
    let snapshot = ministr_api::client::DaemonClient::new()
        .ingestion_progress()
        .await?;
    Ok(snapshot
        .into_iter()
        .map(|p| IngestionProgressInfo {
            corpus_id: p.corpus_id,
            status: p.status,
            phase: p.phase,
            files_total: p.files_total,
            files_done: p.files_done,
            sections_done: p.sections_done,
            embeddings_total: p.embeddings_total,
            embeddings_done: p.embeddings_done,
            current_file: p.current_file,
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
// One linear polling loop: read atomics, detect change, compute ETA, send.
// Splitting it would scatter the change-detection state across helpers for no
// readability gain.
#[allow(clippy::too_many_lines)]
pub async fn indexing_progress_events(
    on_event: Channel<IndexingProgressEvent>,
) -> Result<(), CommandError> {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    tauri::async_runtime::spawn(async move {
        // Per-corpus tracking for change-detection + ETA. We only emit when
        // something the UI cares about changed, and ETA is computed in the
        // command (ministr-core's IngestionProgress doesn't track timing).
        struct Track {
            last_status: u8,
            last_files_done: usize,
            last_embeddings_done: usize,
            last_phase: String,
            last_current_file: String,
            run_started: Option<Instant>,
        }
        let mut tracks: HashMap<String, Track> = HashMap::new();

        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;

            let now_ms = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis()),
            )
            .unwrap_or(u64::MAX);

            // gd2b: source the snapshot from the daemon over UDS instead of an
            // in-process registry. The daemon owns the indexer now; the 250ms
            // poll cadence + change-detection + ETA below are unchanged.
            // Daemon momentarily unreachable (e.g. restart window) — skip this
            // tick and retry next; the frontend keeps its last frame.
            let Ok(snapshot) = ministr_api::client::DaemonClient::new()
                .ingestion_progress()
                .await
            else {
                continue;
            };
            for info in &snapshot {
                let corpus_id = &info.corpus_id;
                let status = info.status;
                let files_total = info.files_total;
                let files_done = info.files_done;
                let embeddings_total = info.embeddings_total;
                let embeddings_done = info.embeddings_done;
                let phase = info.phase.clone();
                let current_file = info.current_file.clone();

                let track = tracks.entry(corpus_id.clone()).or_insert(Track {
                    last_status: u8::MAX,
                    last_files_done: 0,
                    last_embeddings_done: 0,
                    last_phase: String::new(),
                    last_current_file: String::new(),
                    run_started: None,
                });

                let started_running = status == 1 && track.last_status != 1;
                let stopped_running = status != 1 && track.last_status == 1;
                // Emit on ANY UI-visible change. Crucially this now includes
                // `embeddings_done` and `phase`: during the GPU-bound Embedding
                // phase the parser is backpressured by the bounded parse→embed
                // channel, so `files_done` / `current_file` stall for seconds at
                // a time while embeddings keep landing every batch. Watching
                // only files froze the live card (e.g. "114/237 FILES") until a
                // file finally ticked — now the bar moves with the GPU.
                let progressed = files_done != track.last_files_done
                    || embeddings_done != track.last_embeddings_done
                    || phase != track.last_phase
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

                // Rate-based ETA. While files are still being parsed, estimate
                // from the file rate; once everything is parsed (files_done ==
                // files_total) but the GPU embed queue is still draining,
                // estimate from the embeddings rate so the tail keeps an ETA
                // instead of going blank (or showing a stale files-rate value
                // that's meaningless once parsing is backpressured).
                let estimated_remaining_secs = track.run_started.and_then(|t| {
                    if status != 1 {
                        return None;
                    }
                    let elapsed = t.elapsed().as_secs_f64();
                    if elapsed < 1.0 {
                        return None;
                    }
                    // Precision loss is fine — counts top out in the millions
                    // and ETA renders to the nearest second.
                    #[allow(
                        clippy::cast_precision_loss,
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss
                    )]
                    let eta_from = |done: usize, total: usize| -> Option<u64> {
                        if done == 0 || total <= done {
                            return None;
                        }
                        let rate = done as f64 / elapsed;
                        if rate <= 0.0 {
                            return None;
                        }
                        Some((((total - done) as f64) / rate).round() as u64)
                    };
                    if files_total > files_done {
                        eta_from(files_done, files_total)
                    } else {
                        eta_from(embeddings_done, embeddings_total)
                    }
                });

                let ev = IndexingProgressEvent {
                    corpus_id: corpus_id.clone(),
                    status,
                    phase: phase.clone(),
                    files_total,
                    files_done,
                    sections_done: info.sections_done,
                    embeddings_total,
                    embeddings_done,
                    current_file: current_file.clone(),
                    estimated_remaining_secs,
                    timestamp_ms: now_ms,
                };

                track.last_status = status;
                track.last_files_done = files_done;
                track.last_embeddings_done = embeddings_done;
                track.last_phase = phase;
                track.last_current_file = current_file;

                if on_event.send(ev).is_err() {
                    // Frontend dropped the receiver — exit cleanly.
                    return;
                }
            }
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
) -> Result<(), CommandError> {
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
            Err(CommandError::internal(message))
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
pub async fn inference_health(
    _state: State<'_, AppState>,
) -> Result<InferenceHealth, CommandError> {
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
///
/// On Windows the bare name rarely exists on disk — CLIs installed via
/// npm (`claude`, `codex`) are `name.cmd`/`name.ps1` shims and native
/// installers drop `name.exe`. So when `name` has no extension we also
/// try every `PATHEXT` suffix. Returning the *resolved absolute path*
/// (with its real extension) is what lets the caller spawn it correctly
/// — `Command::new("claude")` from a GUI process finds nothing.
///
/// On macOS / Linux the inherited `PATH` is the *other* failure mode:
/// a GUI process launched from Finder / Dock / a `.desktop` file gets
/// launchd's narrow `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin` plus
/// `/etc/paths.d`), which **excludes** Homebrew (`/opt/homebrew/bin`),
/// npm-global, Volta, `~/.local/bin`, and friends. The shell PATH the
/// user sees in a terminal never reaches the app, so `claude` —
/// usually installed via `npm i -g` against Homebrew Node — is
/// invisible. We probe a curated set of common install locations
/// after exhausting `PATH` so "works in my shell" matches "works in
/// the app" without forcing the user to relaunch from a terminal.
fn which_on_path(name: &str) -> Option<String> {
    let has_ext = std::path::Path::new(name).extension().is_some();
    #[cfg(windows)]
    let exts: Vec<String> = if has_ext {
        Vec::new()
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    };

    let probe = |dir: &std::path::Path| -> Option<String> {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct.display().to_string());
        }
        #[cfg(windows)]
        if !has_ext {
            for ext in &exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand.display().to_string());
                }
            }
        }
        None
    };

    // Track which directories we've already probed so the unix-fallback
    // sweep doesn't re-stat dirs that were already on PATH.
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !seen.insert(dir.clone()) {
                continue;
            }
            if let Some(hit) = probe(&dir) {
                return Some(hit);
            }
        }
    }

    #[cfg(not(windows))]
    for dir in unix_extra_bin_dirs() {
        if !seen.insert(dir.clone()) {
            continue;
        }
        if let Some(hit) = probe(&dir) {
            return Some(hit);
        }
    }

    let _ = has_ext;
    let _ = seen;
    None
}

/// Common bin directories that GUI-launched processes on macOS / Linux
/// don't inherit from launchd / the desktop session, but where users
/// expect tools like `claude`, `codex`, `node`, `npm`, `bun`, etc. to
/// live. Used as a fallback by [`which_on_path`] after the inherited
/// `PATH` comes up empty.
///
/// Order matters — Homebrew first (most common on macOS), then per-user
/// language-toolchain dirs, then system-wide non-Homebrew locations.
/// Each path is returned as an absolute [`PathBuf`]; non-existent
/// entries are kept in the list (probe will simply miss them) rather
/// than `stat`-ed twice.
#[cfg(not(windows))]
fn unix_extra_bin_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
        PathBuf::from("/opt/local/bin"),
        PathBuf::from("/opt/local/sbin"),
    ];

    if let Some(home) = home_pathbuf() {
        for sub in [
            ".local/bin",
            "bin",
            ".npm-global/bin",
            ".npm/bin",
            ".yarn/bin",
            ".config/yarn/global/node_modules/.bin",
            ".bun/bin",
            ".volta/bin",
            ".cargo/bin",
            ".deno/bin",
            ".asdf/shims",
            ".local/share/mise/shims",
            ".fnm/aliases/default/bin",
            ".nodenv/shims",
            ".rbenv/shims",
            ".pyenv/shims",
        ] {
            dirs.push(home.join(sub));
        }

        // nvm: walk `$NVM_DIR/versions/node/*/bin` (or `~/.nvm/...`)
        // for any installed node version. Users routinely have a
        // single version installed via nvm, so probing the directory
        // surfaces `claude` without needing the shell-only `nvm use`
        // symlink dance.
        let nvm_root = std::env::var_os("NVM_DIR").map_or_else(|| home.join(".nvm"), PathBuf::from);
        let versions_dir = nvm_root.join("versions").join("node");
        if let Ok(entries) = std::fs::read_dir(&versions_dir) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin");
                if bin.is_dir() {
                    dirs.push(bin);
                }
            }
        }
    }

    dirs
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
///
/// gd2c: routed over UDS to the daemon's read-section endpoint.
#[tauri::command]
pub async fn read_section(
    corpus_id: String,
    section_id: String,
) -> Result<SectionDetailOut, CommandError> {
    let detail = ministr_api::client::DaemonClient::new()
        .read_section(&corpus_id, &section_id)
        .await?;

    Ok(SectionDetailOut {
        section_id: detail.section_id,
        heading_path: detail.heading_path,
        text: detail.text,
        summary: detail.summary,
        claims_available: detail.claims_available,
    })
}

// ---------------------------------------------------------------------------
// MCP wizard — detect / write / test the per-client config files. Powers
// the Settings → AI Assistants panel + the onboarding "Connect your AI
// tool" step.
// ---------------------------------------------------------------------------

/// Status of one detected MCP client on the user's machine.
#[derive(Serialize)]
pub struct McpClientInfo {
    /// Stable id (`claude_code` / `cursor` / `vscode` / `codex`).
    pub id: String,
    /// Human-readable label.
    pub display_name: String,
    /// Whether the client appears to be installed (CLI on PATH or a
    /// known config dir is present).
    pub installed: bool,
    /// Where ministr would write the config for this client. Always
    /// populated, even if not yet `configured`.
    pub config_path: String,
    /// Whether the config file already exists *and* contains a ministr
    /// entry. The wizard uses this to label connected vs. not-yet rows.
    pub configured: bool,
}

/// Result of a connection test against one MCP client.
#[derive(Serialize)]
pub struct McpTestResult {
    /// Whether the test passed.
    pub ok: bool,
    /// Short user-facing message (e.g. "ministr listed in `claude mcp list`"
    /// or "Config file missing").
    pub message: String,
    /// Truncated raw output of the spawned CLI, when applicable. Empty
    /// for editor-client tests.
    pub raw_output_truncated: Option<String>,
    /// True for editor clients (Cursor, VS Code) where we can only
    /// validate the config file, not the live runtime. The wizard uses
    /// this to add a "Restart your editor and re-test" hint.
    pub manual_verify_needed: bool,
}

/// Detect the supported MCP clients on this machine and report whether
/// each is already wired up to ministr.
///
/// `project_root` is the absolute path to the active project — used as
/// the destination for per-project clients (Claude Code, Cursor,
/// VS Code). Codex is user-global; `project_root` is ignored for it.
#[tauri::command]
pub async fn mcp_detect_clients(project_root: String) -> Result<Vec<McpClientInfo>, CommandError> {
    use ministr_core::init::McpClientId;
    let root = std::path::PathBuf::from(&project_root);

    Ok(vec![
        client_info(McpClientId::ClaudeCode, &root),
        client_info(McpClientId::Cursor, &root),
        client_info(McpClientId::VsCode, &root),
        client_info(McpClientId::Codex, &root),
    ])
}

/// Write the MCP config for a single client. Returns the absolute path
/// of the file that was written so the wizard can show it to the user.
#[tauri::command]
pub async fn mcp_write_config(
    project_root: String,
    client_id: String,
) -> Result<String, CommandError> {
    use ministr_core::init::{McpClientId, write_mcp_config};
    let client = McpClientId::parse(&client_id)
        .ok_or_else(|| CommandError::not_found(format!("unknown MCP client id: {client_id}")))?;
    let root = std::path::PathBuf::from(&project_root);
    let path = write_mcp_config(client, &root).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Test the live connection from a CLI client to the ministr server.
///
/// For CLI clients (Claude Code, Codex) we shell out to their `mcp list`
/// equivalent, parse the output, and look for "ministr". For editor
/// clients (Cursor, VS Code Copilot) we can only validate the config
/// file — the wizard surfaces this with `manual_verify_needed: true`.
#[tauri::command]
pub async fn mcp_test_connection(
    project_root: String,
    client_id: String,
) -> Result<McpTestResult, CommandError> {
    use ministr_core::init::McpClientId;
    let client = McpClientId::parse(&client_id)
        .ok_or_else(|| CommandError::not_found(format!("unknown MCP client id: {client_id}")))?;
    let root = std::path::PathBuf::from(&project_root);

    Ok(match client {
        McpClientId::ClaudeCode => test_via_cli("claude", &["mcp", "list"], &root),
        McpClientId::Codex => test_via_cli("codex", &["mcp", "list"], &root),
        McpClientId::Cursor => test_via_config(client, &root, "Cursor"),
        McpClientId::VsCode => test_via_config(client, &root, "VS Code"),
    })
}

fn client_info(client: ministr_core::init::McpClientId, root: &std::path::Path) -> McpClientInfo {
    use ministr_core::init::McpClientId;

    let installed = match client {
        McpClientId::ClaudeCode => probe_cli("claude") || home_subdir_exists(".claude"),
        McpClientId::Cursor => probe_cli("cursor") || home_subdir_exists(".cursor"),
        McpClientId::VsCode => probe_cli("code") || root.join(".vscode").exists(),
        McpClientId::Codex => probe_cli("codex") || home_subdir_exists(".codex"),
    };

    let config_path = match client {
        McpClientId::ClaudeCode => root.join(".mcp.json"),
        McpClientId::Cursor => root.join(".cursor").join("mcp.json"),
        McpClientId::VsCode => root.join(".vscode").join("mcp.json"),
        McpClientId::Codex => home_pathbuf().map_or_else(
            || std::path::PathBuf::from("~/.codex/config.toml"),
            |h| h.join(".codex").join("config.toml"),
        ),
    };

    let configured = match client {
        McpClientId::ClaudeCode | McpClientId::Cursor | McpClientId::VsCode => {
            json_has_ministr(&config_path)
        }
        McpClientId::Codex => toml_has_ministr(&config_path),
    };

    McpClientInfo {
        id: client.as_str().to_string(),
        display_name: client.display_name().to_string(),
        installed,
        config_path: config_path.display().to_string(),
        configured,
    }
}

fn test_via_cli(binary: &str, args: &[&str], cwd: &std::path::Path) -> McpTestResult {
    let Some(exe) = which_on_path(binary) else {
        return McpTestResult {
            ok: false,
            message: format!(
                "`{binary}` not found on PATH. If it is installed, the ministr \
                 desktop app may have a narrower PATH than your shell — relaunch \
                 it after installing {binary}, or ensure {binary} is on the user PATH."
            ),
            raw_output_truncated: None,
            manual_verify_needed: false,
        };
    };

    // Build the command from the *resolved absolute path* — `Command::new(
    // "claude")` from a GUI process can't find an npm `.cmd` shim, and a
    // `.cmd`/`.bat` cannot be spawned directly on Windows (it must go
    // through `cmd /c`, and Rust ≥1.77 hard-errors otherwise).
    let mut command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // GUI binary (windows_subsystem = "windows") → suppress the
        // console window the child would otherwise flash.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let is_script = std::path::Path::new(&exe)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
        if is_script {
            command = std::process::Command::new("cmd");
            command.arg("/c").arg(&exe).args(args);
        } else {
            command = std::process::Command::new(&exe);
            command.args(args);
        }
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        command = std::process::Command::new(&exe);
        command.args(args);
    }

    // Run *inside the project root*. Claude Code's `.mcp.json` is
    // project-scoped — `claude mcp list` only enumerates it when invoked
    // from that directory. Without `current_dir` the command ran in the
    // Tauri app's cwd and never saw the project server, producing a
    // false "ran but didn't list ministr".
    let result = command
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            let listed = combined.to_lowercase().contains("ministr");
            let truncated: String = combined.chars().take(800).collect();
            McpTestResult {
                ok: listed,
                message: if listed {
                    format!("ministr listed in `{binary} {}`.", args.join(" "))
                } else {
                    format!(
                        "`{binary} {}` ran but didn't list ministr. The config is \
                         project-scoped (.mcp.json) — open this project in {binary} \
                         once and approve the ministr server when prompted, then re-test.",
                        args.join(" ")
                    )
                },
                raw_output_truncated: Some(truncated),
                manual_verify_needed: false,
            }
        }
        Err(e) => McpTestResult {
            ok: false,
            message: format!("Failed to run `{binary}`: {e}"),
            raw_output_truncated: None,
            manual_verify_needed: false,
        },
    }
}

fn test_via_config(
    client: ministr_core::init::McpClientId,
    root: &std::path::Path,
    label: &str,
) -> McpTestResult {
    use ministr_core::init::McpClientId;
    let path = match client {
        McpClientId::Cursor => root.join(".cursor").join("mcp.json"),
        McpClientId::VsCode => root.join(".vscode").join("mcp.json"),
        _ => unreachable!("test_via_config is only called for editor clients"),
    };

    if !path.exists() {
        return McpTestResult {
            ok: false,
            message: format!("Config file not found at {}", path.display()),
            raw_output_truncated: None,
            manual_verify_needed: true,
        };
    }

    if json_has_ministr(&path) {
        McpTestResult {
            ok: true,
            message: format!(
                "ministr is configured in {}. Restart {label} and re-test if you haven't yet.",
                path.display()
            ),
            raw_output_truncated: None,
            manual_verify_needed: true,
        }
    } else {
        McpTestResult {
            ok: false,
            message: format!(
                "{} exists but has no ministr entry — run Connect to write one.",
                path.display()
            ),
            raw_output_truncated: None,
            manual_verify_needed: true,
        }
    }
}

fn json_has_ministr(path: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value
        .get("mcpServers")
        .and_then(|v| v.get("ministr"))
        .is_some()
}

fn toml_has_ministr(path: &std::path::Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    // We look for the `[mcp_servers.ministr]` header rather than parsing
    // the TOML — the same shortcut used by `write_codex_mcp` to avoid
    // round-tripping a hand-edited file.
    content.contains("[mcp_servers.ministr]")
}

fn probe_cli(binary: &str) -> bool {
    if cfg!(windows) {
        which_on_path(&format!("{binary}.exe")).is_some() || which_on_path(binary).is_some()
    } else {
        which_on_path(binary).is_some()
    }
}

fn home_subdir_exists(name: &str) -> bool {
    home_pathbuf().is_some_and(|h| h.join(name).exists())
}

/// Cross-platform home-dir lookup as a [`PathBuf`]. The crate already has
/// a `home_dir() -> Option<String>` helper used by the open-path expansion
/// flow; this returns a typed path so the MCP wizard can compose it with
/// `.join()` calls cleanly.
fn home_pathbuf() -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Some(std::path::PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.is_empty()
    {
        return Some(std::path::PathBuf::from(profile));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::lang_from_path;

    #[test]
    fn lang_from_path_maps_known_extensions() {
        assert_eq!(lang_from_path("src/main.rs"), "rust");
        assert_eq!(lang_from_path("a/b/App.tsx"), "tsx");
        assert_eq!(lang_from_path("lib/types.ts"), "typescript");
        assert_eq!(lang_from_path("script.py"), "python");
        assert_eq!(lang_from_path("Cargo.toml"), "toml");
        assert_eq!(lang_from_path("README.md"), "markdown");
    }

    #[test]
    fn lang_from_path_is_case_insensitive() {
        assert_eq!(lang_from_path("FOO.RS"), "rust");
        assert_eq!(lang_from_path("Page.JSX"), "jsx");
    }

    #[test]
    fn lang_from_path_falls_back_to_text() {
        assert_eq!(lang_from_path("data.unknownext"), "text");
        assert_eq!(lang_from_path("LICENSE"), "text");
        assert_eq!(lang_from_path(""), "text");
    }
}
