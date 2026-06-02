//! Ensure the headless `ministr __daemon` sidecar is running (gd1 of the
//! daemon/GUI decouple).
//!
//! The desktop GUI no longer binds the daemon's UDS socket in-process — it
//! no longer calls [`ministr_daemon::daemon::start`]. Instead it ensures a
//! *separate* headless daemon owns the socket, spawning the `ministr` CLI's
//! hidden `__daemon` subcommand when none is alive. The daemon then survives
//! GUI close/restart (corpora stay warm, indexing continues) and is the
//! single host the MCP proxy and CLI also attach to.
//!
//! The in-process `AppState` still backs the Tauri commands for now; gd2/gd3
//! migrate those reads/writes onto the daemon over UDS and gd4 removes the
//! in-process state entirely.

use std::path::PathBuf;
use std::time::Duration;

use tauri::Manager as _;
use tracing::{info, warn};

/// How long to wait for a freshly-spawned daemon to answer before giving up.
/// `bootstrap::run` binds the listener *before* restoring corpora, so a
/// healthy `/api/v1/status` typically arrives in well under a second — this
/// is generous headroom, not an expected wait.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolve the `ministr` CLI binary that hosts the `__daemon` subcommand.
///
/// Preference order, most-canonical first:
/// 1. PKG install at `/usr/local/bin/ministr` (the system sidecar MCP
///    clients on `PATH` already use).
/// 2. Staged `~/.ministr/bin/ministr` — the shared sidecar first-launch
///    setup installs and the version guard protects.
/// 3. The `ministr-cli` sidecar bundled next to the app binary (the
///    fallback before staging completes on a fresh install).
///
/// Returns `None` when no CLI binary can be found — the caller logs and the
/// UI surfaces the daemon as unreachable via the existing `daemon_status`
/// polling.
#[must_use]
pub fn resolve_daemon_binary(app: &tauri::App) -> Option<PathBuf> {
    // 1. System PKG install.
    let pkg = PathBuf::from("/usr/local/bin/ministr");
    if pkg.exists() {
        return Some(pkg);
    }

    // 2. Staged shared sidecar. On Windows the staged name may carry `.exe`
    //    (NSIS) or not (the app's own copy) — probe both, mirroring
    //    `setup::ensure_path`.
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let bin = PathBuf::from(home).join(".ministr").join("bin");
        for name in staged_names() {
            let candidate = bin.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 3. Bundled sidecar (`ministr-cli`) — resource dir, then next to the
    //    current executable (Contents/MacOS/ministr-cli on macOS).
    let sidecar = sidecar_name();
    if let Ok(res) = app.path().resource_dir() {
        let candidate = res.join(sidecar);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let candidate = dir.join(sidecar);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// Candidate names for the staged `~/.ministr/bin` CLI.
fn staged_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["ministr.exe", "ministr"]
    } else {
        &["ministr"]
    }
}

/// Bundled-sidecar file name next to the app binary.
fn sidecar_name() -> &'static str {
    if cfg!(windows) {
        "ministr-cli.exe"
    } else {
        "ministr-cli"
    }
}

/// Ensure a healthy daemon owns the socket, spawning the detached sidecar if
/// not. Logs the outcome; never panics — daemon availability is surfaced to
/// the UI by the existing `daemon_status` polling / `DaemonErrorBanner`.
pub async fn ensure_daemon_running(daemon_bin: PathBuf) {
    let client = ministr_api::client::DaemonClient::new();
    match client
        .ensure_daemon_spawned(&daemon_bin, SPAWN_READY_TIMEOUT)
        .await
    {
        Ok(false) => info!("attached to an already-running ministr daemon sidecar"),
        Ok(true) => {
            info!(bin = %daemon_bin.display(), "spawned headless ministr daemon sidecar");
        }
        Err(e) => warn!(
            error = %e,
            bin = %daemon_bin.display(),
            "failed to ensure the ministr daemon sidecar — the UI will report \
             the daemon as unreachable"
        ),
    }
}
