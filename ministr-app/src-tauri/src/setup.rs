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

    // Decide whether to run setup. Crucially, NEVER let an older app build
    // re-run setup over a newer install: `install_cli_binary` below would
    // DOWNGRADE the shared ~/.ministr/bin sidecar out from under a running
    // daemon/MCP server. That is the app-install-singleton corruption — a
    // stale duplicate `ministr.app` bundle (same `ai.ministr.desktop` id) gets
    // launched and clobbers the live 148MB CLI binary the daemon is using.
    let version_path = data_dir.join(SETUP_VERSION_FILE);
    let installed_marker = fs::read_to_string(&version_path).ok();
    match decide_setup(installed_marker.as_deref(), current_version) {
        SetupDecision::AlreadyCurrent => {
            info!(
                version = current_version,
                "setup already completed for this version"
            );
            return Ok(());
        }
        SetupDecision::SkipOlderThanInstalled { installed } => {
            warn!(
                installed = %installed,
                this = current_version,
                "a newer ministr is already installed but an OLDER app build launched — \
                 skipping setup so the shared ~/.ministr/bin sidecar is not downgraded out \
                 from under a running daemon (likely a stale duplicate ministr.app; see \
                 app-install-singleton-enforce)"
            );
            return Ok(());
        }
        SetupDecision::Run => {}
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

    // 4b. Install a desktop entry (Linux only). The .deb / .rpm packages
    //     already ship a system .desktop via Tauri's bundler, but a
    //     double-clicked AppImage has no app-menu entry at all. Give the
    //     AppImage the same "shows up in your launcher" parity the
    //     macOS .pkg / Windows NSIS installs get for free.
    #[cfg(target_os = "linux")]
    if let Err(e) = install_linux_desktop_entry() {
        warn!(error = %e, "could not install Linux desktop entry");
    }

    // 5. Write setup version marker
    if let Err(e) = fs::write(&version_path, current_version) {
        warn!(
            error = %e,
            path = %version_path.display(),
            "could not write setup version marker; first-launch setup may re-run"
        );
    }

    info!("first-launch setup complete");
    Ok(())
}

/// Whether `run_first_launch_setup` should proceed, given the recorded
/// `setup_version` marker and this build's version.
#[derive(Debug, PartialEq, Eq)]
enum SetupDecision {
    /// This exact version is already set up — nothing to do.
    AlreadyCurrent,
    /// Fresh install or genuine upgrade — run setup.
    Run,
    /// A strictly NEWER ministr is already installed; this is an older build
    /// (e.g. a stale duplicate bundle). Skip setup so we don't downgrade the
    /// shared sidecar.
    SkipOlderThanInstalled { installed: String },
}

/// Decide whether first-launch setup should run.
///
/// `installed` is the contents of the `setup_version` marker (the version that
/// last completed setup), or `None`/empty if never set up. `current` is this
/// build's `CARGO_PKG_VERSION`.
fn decide_setup(installed: Option<&str>, current: &str) -> SetupDecision {
    let Some(installed) = installed.map(str::trim).filter(|s| !s.is_empty()) else {
        return SetupDecision::Run; // never set up
    };
    if installed == current {
        return SetupDecision::AlreadyCurrent;
    }
    if is_strictly_newer(installed, current) {
        return SetupDecision::SkipOlderThanInstalled {
            installed: installed.to_string(),
        };
    }
    SetupDecision::Run
}

/// Parse the leading `major.minor.patch` of a version, ignoring any
/// `-prerelease` / `+build` suffix. Missing minor/patch default to `0`.
fn version_triple(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// True when version `a` is strictly newer than `b`. Numeric (not
/// lexicographic) so `0.10.0 > 0.9.0`. Unparseable versions compare as "not
/// newer", so we conservatively fall back to running setup (the historical
/// behavior) rather than wrongly skipping it.
fn is_strictly_newer(a: &str, b: &str) -> bool {
    match (version_triple(a), version_triple(b)) {
        (Some(va), Some(vb)) => va > vb,
        _ => false,
    }
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
    // On Windows, probe both names: the NSIS installer hook stages as
    // `ministr.exe`, but install_cli_binary above stages as `ministr`
    // (no extension). Either is fine — Windows can spawn extensionless
    // PE files via CreateProcess. On Unix only the bare name is valid.
    let cli_path = if cfg!(windows) {
        let exe = bin_dir.join("ministr.exe");
        if exe.exists() {
            exe
        } else {
            bin_dir.join("ministr")
        }
    } else {
        bin_dir.join("ministr")
    };

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

    // Load the agent (non-fatal if this fails — the plist is installed
    // and will load on next login regardless).
    match std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_dest)
        .output()
    {
        Ok(out) if !out.status.success() => warn!(
            status = ?out.status.code(),
            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
            "launchctl load returned non-zero; agent will load on next login"
        ),
        Err(e) => warn!(error = %e, "failed to invoke launchctl; agent will load on next login"),
        Ok(_) => {}
    }

    Ok(())
}

/// Install a per-user XDG desktop entry + icon so the app appears in the
/// Linux application menu.
///
/// Only meaningful for the `AppImage` distribution: a double-clicked
/// `AppImage` is a single self-contained file with no installer and no
/// menu integration. The `.deb` / `.rpm` packages already register a
/// system-wide `.desktop` through Tauri's bundler, so we detect that
/// case (binary living under a system prefix) and skip — mirroring the
/// non-clobbering posture of `install_launchd_plist`.
#[cfg(target_os = "linux")]
fn install_linux_desktop_entry() -> Result<(), Box<dyn std::error::Error>> {
    // The AppImage runtime exports $APPIMAGE as the absolute path to the
    // .AppImage file itself — that's what the launcher must Exec. Without
    // it we're almost certainly running from a system package that
    // already has its own .desktop; nothing to do.
    let Ok(appimage) = std::env::var("APPIMAGE") else {
        info!("not running as an AppImage ($APPIMAGE unset) — skipping desktop entry");
        return Ok(());
    };

    let home = home_dir()?;
    let apps_dir = home.join(".local/share/applications");
    let icons_dir = home.join(".local/share/icons/hicolor/128x128/apps");
    fs::create_dir_all(&apps_dir)?;
    fs::create_dir_all(&icons_dir)?;

    // Stage the icon next to the entry so launchers resolve it by name.
    // Basename is `ai.ministr` (NOT the `ai.ministr.desktop` app id,
    // which already ends in `.desktop` and would yield a doubled
    // extension on the entry file).
    let icon_dest = icons_dir.join("ai.ministr.png");
    if !icon_dest.exists() {
        let icon_bytes: &[u8] = include_bytes!("../icons/128x128.png");
        fs::write(&icon_dest, icon_bytes)?;
    }

    // Single `.desktop` extension.
    let desktop_dest = apps_dir.join("ai.ministr.desktop");

    // Escape the AppImage path for the `Exec` key per the freedesktop
    // Desktop Entry spec: quote (so spaces are one argument) and
    // backslash-escape the reserved characters `"`, `$`, `` ` ``, `\`.
    let exec_value = {
        let escaped = appimage
            .replace('\\', "\\\\")
            .replace('`', "\\`")
            .replace('"', "\\\"")
            .replace('$', "\\$");
        format!("\"{escaped}\"")
    };

    // Don't clobber a user-customized entry, but always refresh if the
    // AppImage moved (common: Downloads -> ~/Applications).
    if desktop_dest.exists()
        && let Ok(existing) = fs::read_to_string(&desktop_dest)
        && existing.contains(&format!("Exec={exec_value}"))
    {
        info!("Linux desktop entry already current — skipping");
        return Ok(());
    }

    // Built line-by-line (no Rust-indentation leaking into the file):
    // .desktop keys must start at column 0.
    let entry = [
        "[Desktop Entry]",
        "Type=Application",
        "Name=ministr",
        "Comment=Code intelligence MCP server for AI coding agents",
        &format!("Exec={exec_value}"),
        "Icon=ai.ministr",
        "Terminal=false",
        "Categories=Development;Utility;",
        "StartupWMClass=ministr",
        "",
    ]
    .join("\n");
    fs::write(&desktop_dest, entry)?;
    info!(path = %desktop_dest.display(), "installed Linux desktop entry");

    // Best-effort menu refresh; non-fatal if the tool is absent.
    if let Err(e) = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .output()
    {
        info!(error = %e, "update-desktop-database unavailable — entry still registered");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_runs() {
        assert_eq!(decide_setup(None, "0.6.0"), SetupDecision::Run);
        assert_eq!(decide_setup(Some(""), "0.6.0"), SetupDecision::Run);
        assert_eq!(decide_setup(Some("   "), "0.6.0"), SetupDecision::Run);
    }

    #[test]
    fn same_version_is_already_current() {
        assert_eq!(
            decide_setup(Some("0.6.0"), "0.6.0"),
            SetupDecision::AlreadyCurrent
        );
        // The marker is read from a file, so tolerate trailing whitespace.
        assert_eq!(
            decide_setup(Some(" 0.6.0\n"), "0.6.0"),
            SetupDecision::AlreadyCurrent
        );
    }

    #[test]
    fn genuine_upgrade_runs() {
        assert_eq!(decide_setup(Some("0.6.0"), "0.7.0"), SetupDecision::Run);
        assert_eq!(decide_setup(Some("0.5.1"), "0.6.0"), SetupDecision::Run);
    }

    #[test]
    fn older_build_does_not_downgrade_a_newer_install() {
        // The app-install-singleton corruption: a stale 0.5.1 bundle launched
        // after 0.6.0 was installed must NOT re-run setup (which would
        // downgrade the shared ~/.ministr/bin sidecar under a live daemon).
        assert_eq!(
            decide_setup(Some("0.6.0"), "0.5.1"),
            SetupDecision::SkipOlderThanInstalled {
                installed: "0.6.0".into()
            }
        );
    }

    #[test]
    fn version_triple_parses_and_ignores_suffixes() {
        assert_eq!(version_triple("0.6.0"), Some((0, 6, 0)));
        assert_eq!(version_triple("1.2"), Some((1, 2, 0)));
        assert_eq!(version_triple("3"), Some((3, 0, 0)));
        assert_eq!(version_triple("0.6.0-rc.1"), Some((0, 6, 0)));
        assert_eq!(version_triple("0.6.0+build5"), Some((0, 6, 0)));
        assert_eq!(version_triple("not-a-version"), None);
    }

    #[test]
    fn ordering_is_numeric_not_lexicographic() {
        // 0.10.0 > 0.9.0 numerically; a string compare would get this wrong.
        assert!(is_strictly_newer("0.10.0", "0.9.0"));
        assert!(!is_strictly_newer("0.9.0", "0.10.0"));
        assert_eq!(
            decide_setup(Some("0.10.0"), "0.9.0"),
            SetupDecision::SkipOlderThanInstalled {
                installed: "0.10.0".into()
            }
        );
    }

    #[test]
    fn unparseable_marker_falls_back_to_run() {
        // Conservative: if we can't compare, behave like before (run setup),
        // never silently skip.
        assert_eq!(decide_setup(Some("garbage"), "0.6.0"), SetupDecision::Run);
    }
}
