/**
 * Single source of truth for ministr's install funnel.
 *
 * Every install/download command, asset name, and host URL that the docs
 * site renders is defined here. The canonical install page (`/install`),
 * the landing-page Hero, and the landing-page InstallTabs all import
 * from this module — so changing a command in one place updates every
 * surface at once.
 *
 * Asset names match what `.github/workflows/release.yml` produces. If
 * you add a new build target there, add it here too.
 */

/** Canonical front-door URL for the install page. */
export const INSTALL_HOST = 'https://ministr.ai';

/** Cloudflare Worker proxy that fronts GitHub Releases. */
export const DOWNLOAD_HOST = 'https://dl.ministr.app';

/** Returns the URL the Worker exposes for `latest` release metadata. */
export function latestReleaseUrl(): string {
  return `${DOWNLOAD_HOST}/latest`;
}

/** Returns the proxy URL for a specific release asset. */
export function downloadUrl(tag: string, filename: string): string {
  return `${DOWNLOAD_HOST}/${tag}/${filename}`;
}

/** Returns the proxy URL pointing at the latest tag for an asset name. */
export function latestDownloadUrl(filename: string): string {
  return `${DOWNLOAD_HOST}/latest/${filename}`;
}

/** Detected OS family. `'unknown'` falls back to the macOS tab. */
export type OsFamily = 'macos' | 'linux' | 'windows' | 'unknown';

/**
 * Best-effort OS detection from a User-Agent string. Used by the client
 * `/install` page to default-select the right CLI tab on first paint.
 *
 * Intentionally simple — server-side uses the request's UA, client-side
 * uses `navigator.userAgent`. For the (rare) ambiguous case we return
 * `'unknown'` and the page falls back to the macOS tab.
 */
export function detectOsFamily(userAgent: string | undefined | null): OsFamily {
  if (!userAgent) return 'unknown';
  const ua = userAgent.toLowerCase();
  if (ua.includes('mac os') || ua.includes('macos') || ua.includes('darwin')) {
    return 'macos';
  }
  if (ua.includes('windows')) return 'windows';
  if (ua.includes('linux') || ua.includes('x11') || ua.includes('cros')) {
    return 'linux';
  }
  return 'unknown';
}

// ─── CLI install commands ───────────────────────────────────────────────

export type CliCommandId = 'macos' | 'linux' | 'windows';

export interface CliCommand {
  id: CliCommandId;
  /** Tab label. */
  label: string;
  /** Fenced shell language for syntax highlight. */
  shell: 'bash' | 'powershell' | 'sh';
  /** The command shown to the user. */
  command: string;
  /** What the copy button puts on the clipboard (often === command). */
  copyText: string;
  /** Optional one-line note rendered under the command. */
  note?: string;
}

export const INSTALL_COMMANDS: readonly CliCommand[] = [
  {
    id: 'macos',
    label: 'macOS',
    shell: 'bash',
    command: 'curl -fsSL https://ministr.app/install.sh | bash',
    copyText: 'curl -fsSL https://ministr.app/install.sh | bash',
    note: 'Apple Silicon only. Intel Mac users: build from source via the Cargo tab.',
  },
  {
    id: 'linux',
    label: 'Linux',
    shell: 'bash',
    command: 'curl -fsSL https://ministr.app/install.sh | bash',
    copyText: 'curl -fsSL https://ministr.app/install.sh | bash',
    note: 'x86_64 and aarch64 both supported. Auto-detected.',
  },
  {
    id: 'windows',
    label: 'Windows',
    shell: 'powershell',
    command: 'iwr -useb https://ministr.app/install.ps1 | iex',
    copyText: 'iwr -useb https://ministr.app/install.ps1 | iex',
    note: 'Adds %USERPROFILE%\\.ministr\\bin to your User PATH. Open a new terminal after install.',
  },
] as const;

// ─── Desktop installers ─────────────────────────────────────────────────

export type DesktopPlatformId =
  | 'macos-aarch64'
  | 'windows-x64'
  | 'linux-deb'
  | 'linux-appimage';

export interface DesktopInstaller {
  id: DesktopPlatformId;
  /** Human label for the card heading. */
  label: string;
  /** Asset filename as published on the GitHub Release. */
  filename: string;
  /** Friendly file extension shown in the UI. */
  ext: 'dmg' | 'exe' | 'deb' | 'AppImage';
  /** One-line install/run hint. */
  hint: string;
}

export const DESKTOP_INSTALLERS: readonly DesktopInstaller[] = [
  {
    id: 'macos-aarch64',
    label: 'macOS (Apple Silicon)',
    filename: 'ministr-desktop-aarch64-apple-darwin.dmg',
    ext: 'dmg',
    hint: 'Drag ministr.app into /Applications.',
  },
  {
    id: 'windows-x64',
    label: 'Windows (x86_64)',
    filename: 'ministr-desktop-x86_64-pc-windows-msvc-setup.exe',
    ext: 'exe',
    hint: 'NSIS installer. Adds the CLI to PATH automatically.',
  },
  {
    id: 'linux-deb',
    label: 'Linux Debian / Ubuntu',
    filename: 'ministr-desktop-x86_64-unknown-linux-gnu.deb',
    ext: 'deb',
    hint: 'sudo dpkg -i ministr-desktop-*.deb',
  },
  {
    id: 'linux-appimage',
    label: 'Linux (universal)',
    filename: 'ministr-desktop-x86_64-unknown-linux-gnu.AppImage',
    ext: 'AppImage',
    hint: 'chmod +x and run.',
  },
] as const;

// ─── CLI archive name helpers (match release.yml) ───────────────────────

/** Rust target triples we publish CLI archives for. */
export type CliTarget =
  | 'x86_64-unknown-linux-gnu'
  | 'aarch64-unknown-linux-gnu'
  | 'aarch64-apple-darwin'
  | 'x86_64-pc-windows-msvc';

/** Returns the CLI archive name for a target triple (matches release.yml). */
export function cliArchiveName(target: CliTarget): string {
  return target === 'x86_64-pc-windows-msvc'
    ? `ministr-${target}.zip`
    : `ministr-${target}.tar.gz`;
}

/** Unified checksums file uploaded to every release. */
export const SHA256SUMS_FILENAME = 'SHA256SUMS';
