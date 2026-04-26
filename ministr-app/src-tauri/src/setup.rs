//! First-launch setup: installs the ministr CLI, configures PATH, and sets up
//! the launchd agent so everything "just works" after opening the .app.

use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Marker file that records the version we last installed.
const SETUP_VERSION_FILE: &str = "setup_version";

/// Run the first-launch (or upgrade) setup sequence.
///
/// This is intentionally synchronous and fast — it only copies files and
/// patches shell configs.  Called from the Tauri `setup` callback before
/// the event loop starts.
pub fn run_first_launch_setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = ministr_data_dir();
    let bin_dir = data_dir.join("bin");
    let current_version = env!("CARGO_PKG_VERSION");

    // Skip if this exact version was already set up.
    let version_path = data_dir.join(SETUP_VERSION_FILE);
    if let Ok(installed) = fs::read_to_string(&version_path)
        && installed.trim() == current_version
    {
        info!(
            version = current_version,
            "setup already completed for this version"
        );
        return Ok(());
    }

    info!(version = current_version, "running first-launch setup");

    // 1. Create ~/.ministr/bin/
    fs::create_dir_all(&bin_dir)?;

    // 2. Copy the sidecar CLI binary — skip if PKG installer already placed
    //    the CLI at /usr/local/bin/ministr (detected by /etc/paths.d/ministr).
    if pkg_installed_cli() {
        info!("CLI already installed by PKG — skipping sidecar copy");
    } else if let Err(e) = install_cli_binary(app, &bin_dir) {
        warn!(error = %e, "could not install CLI binary — continuing without it");
    }

    // 3. Ensure PATH is set up — skip if PKG handled it via /etc/paths.d.
    if pkg_installed_cli() {
        info!("PATH already configured by PKG installer");
    } else if let Err(e) = ensure_path(&bin_dir) {
        warn!(error = %e, "could not update shell profile for PATH");
    }

    // 4. Install launchd agent plist (macOS only)
    #[cfg(target_os = "macos")]
    if let Err(e) = install_launchd_plist() {
        warn!(error = %e, "could not install launchd plist");
    }

    // 5. Write setup version marker
    let _ = fs::write(&version_path, current_version);

    info!("first-launch setup complete");
    Ok(())
}

/// Locate the `ministr-cli` sidecar inside the .app bundle and copy it to
/// `~/.ministr/bin/ministr`.
fn install_cli_binary(app: &tauri::App, bin_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::Manager;

    // In a bundled macOS .app, the sidecar sits next to the main binary
    // at Contents/MacOS/ministr-cli.
    let sidecar_name = "ministr-cli";
    let sidecar_path = app
        .path()
        .resource_dir()
        .ok()
        .and_then(|res| {
            // Tauri v2 puts sidecars in the resource dir on some platforms,
            // but on macOS they're next to the main binary.
            let candidate = res.join(sidecar_name);
            if candidate.exists() {
                return Some(candidate);
            }
            None
        })
        .or_else(|| {
            // Fallback: look next to the current executable.
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join(sidecar_name)))
        });

    let sidecar = match sidecar_path {
        Some(p) if p.exists() => p,
        _ => {
            info!("sidecar binary not found in bundle — skipping CLI install");
            return Ok(());
        }
    };

    let dest = bin_dir.join("ministr");

    // Copy with atomic rename to avoid partial writes.
    let tmp = bin_dir.join(".ministr.tmp");
    fs::copy(&sidecar, &tmp)?;

    // Ensure executable permission on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    }

    fs::rename(&tmp, &dest)?;
    info!(path = %dest.display(), "installed ministr CLI binary");
    Ok(())
}

/// Add `~/.ministr/bin` to the user's PATH by shelling out to the freshly
/// staged CLI's `setup` subcommand.
///
/// The CLI uses the `onpath` crate, which detects installed shells (bash,
/// zsh, fish, nushell, `PowerShell`, tcsh, xonsh) and writes the right rc
/// file edits. On Windows it writes `HKCU\Environment\PATH` directly. This
/// replaced an earlier hand-rolled patcher that only knew about
/// `.zshrc/.bashrc/.bash_profile/.profile` and missed fish + nushell.
fn ensure_path(bin_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let cli_name = if cfg!(windows) {
        "ministr.exe"
    } else {
        "ministr"
    };
    let cli_path = bin_dir.join(cli_name);

    if !cli_path.exists() {
        info!(
            path = %cli_path.display(),
            "CLI binary not staged yet — skipping PATH setup"
        );
        return Ok(());
    }

    let output = std::process::Command::new(&cli_path)
        .arg("setup")
        .arg("--bin-dir")
        .arg(bin_dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`ministr setup` exited non-zero: {}", stderr.trim()).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    info!(report = %stdout.trim(), "PATH setup complete via `ministr setup`");
    Ok(())
}

/// Install the `ai.ministr.desktop` launchd plist for the current user.
#[cfg(target_os = "macos")]
fn install_launchd_plist() -> Result<(), Box<dyn std::error::Error>> {
    let home = home_dir()?;
    let agents_dir = home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents_dir)?;

    let plist_dest = agents_dir.join("ai.ministr.desktop.plist");

    // Only install if not already present (don't clobber user customizations).
    if plist_dest.exists() {
        info!("launchd plist already exists — skipping");
        return Ok(());
    }

    let plist_content = include_str!("../resources/ai.ministr.desktop.plist");
    fs::write(&plist_dest, plist_content)?;
    info!(path = %plist_dest.display(), "installed launchd plist");

    // Load the agent (non-fatal if this fails).
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_dest)
        .output();

    Ok(())
}

/// Check if the PKG installer placed the CLI at /usr/local/bin/ministr.
/// The PKG also creates /etc/paths.d/ministr, which is a reliable marker.
fn pkg_installed_cli() -> bool {
    Path::new("/etc/paths.d/ministr").exists() && Path::new("/usr/local/bin/ministr").exists()
}

fn ministr_data_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".ministr")
}

fn home_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(std::convert::Into::into)
}
