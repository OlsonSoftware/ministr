//! First-launch setup: installs the iris CLI, configures PATH, and sets up
//! the launchd agent so everything "just works" after opening the .app.

use std::fs;
use std::io::Write;
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
    let data_dir = iris_data_dir();
    let bin_dir = data_dir.join("bin");
    let current_version = env!("CARGO_PKG_VERSION");

    // Skip if this exact version was already set up.
    let version_path = data_dir.join(SETUP_VERSION_FILE);
    if let Ok(installed) = fs::read_to_string(&version_path) {
        if installed.trim() == current_version {
            info!(version = current_version, "setup already completed for this version");
            return Ok(());
        }
    }

    info!(version = current_version, "running first-launch setup");

    // 1. Create ~/.iris/bin/
    fs::create_dir_all(&bin_dir)?;

    // 2. Copy the sidecar CLI binary into ~/.iris/bin/iris
    if let Err(e) = install_cli_binary(app, &bin_dir) {
        warn!(error = %e, "could not install CLI binary — continuing without it");
    }

    // 3. Ensure ~/.iris/bin is on the user's PATH
    if let Err(e) = ensure_path(&bin_dir) {
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

/// Locate the `iris-cli` sidecar inside the .app bundle and copy it to
/// `~/.iris/bin/iris`.
fn install_cli_binary(app: &tauri::App, bin_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::Manager;

    // In a bundled macOS .app, the sidecar sits next to the main binary
    // at Contents/MacOS/iris-cli.
    let sidecar_name = "iris-cli";
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
            std::env::current_exe().ok().and_then(|exe| {
                exe.parent().map(|dir| dir.join(sidecar_name))
            })
        });

    let sidecar = match sidecar_path {
        Some(p) if p.exists() => p,
        _ => {
            info!("sidecar binary not found in bundle — skipping CLI install");
            return Ok(());
        }
    };

    let dest = bin_dir.join("iris");

    // Copy with atomic rename to avoid partial writes.
    let tmp = bin_dir.join(".iris.tmp");
    fs::copy(&sidecar, &tmp)?;

    // Ensure executable permission on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    }

    fs::rename(&tmp, &dest)?;
    info!(path = %dest.display(), "installed iris CLI binary");
    Ok(())
}

/// Append `~/.iris/bin` to PATH in the user's shell profile if not already present.
fn ensure_path(bin_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let bin_str = bin_dir.to_string_lossy();

    // Check if already on PATH.
    if let Ok(path) = std::env::var("PATH") {
        if path.split(':').any(|p| p == bin_str.as_ref()) {
            return Ok(());
        }
    }

    let home = home_dir()?;
    let export_line = format!("\n# Added by iris installer\nexport PATH=\"{}:$PATH\"\n", bin_str);

    // Patch whichever shell profiles exist.
    for profile in &[".zshrc", ".bashrc", ".bash_profile", ".profile"] {
        let path = home.join(profile);
        if !path.exists() {
            continue;
        }

        let content = fs::read_to_string(&path)?;
        if content.contains(&*bin_str) {
            continue;
        }

        let mut file = fs::OpenOptions::new().append(true).open(&path)?;
        file.write_all(export_line.as_bytes())?;
        info!(profile = profile, "added ~/.iris/bin to PATH");
    }

    Ok(())
}

/// Install the `com.iris.desktop` launchd plist for the current user.
#[cfg(target_os = "macos")]
fn install_launchd_plist() -> Result<(), Box<dyn std::error::Error>> {
    let home = home_dir()?;
    let agents_dir = home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents_dir)?;

    let plist_dest = agents_dir.join("com.iris.desktop.plist");

    // Only install if not already present (don't clobber user customizations).
    if plist_dest.exists() {
        info!("launchd plist already exists — skipping");
        return Ok(());
    }

    let plist_content = include_str!("../resources/com.iris.desktop.plist");
    fs::write(&plist_dest, plist_content)?;
    info!(path = %plist_dest.display(), "installed launchd plist");

    // Load the agent (non-fatal if this fails).
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_dest)
        .output();

    Ok(())
}

fn iris_data_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".iris")
}

fn home_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|e| e.into())
}
