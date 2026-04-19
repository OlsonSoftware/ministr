---
title: Download iris
description: One installer for the iris desktop app and the iris CLI. Signed, notarized, auto-PATH.
hide:
  - navigation
  - toc
---

<div class="iris-hero" markdown>

<span class="iris-hero__eyebrow">
  <svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
  Download · macOS · Linux · Windows
</span>

# One install. Both tools. { .iris-hero__title }

<p class="iris-hero__tagline">
  The iris installer drops the desktop observatory into <code>/Applications</code>,
  places the <code>iris</code> CLI on your <code>PATH</code>, and registers a
  background agent so the daemon is always ready when your MCP client connects.
</p>

<div
  class="iris-download"
  data-iris-download
  data-version="0.1.0"
  data-release-base="https://github.com/AlrikOlson/iris-rs/releases/latest/download"
>

<div class="iris-download__primary" data-target="macos-arm64">
  <div class="iris-download__primary-meta">
    <span class="iris-download__badge">Recommended for your Mac</span>
    <h2 class="iris-download__primary-title">
      <svg class="icon icon-md"><use href="assets/icons.svg#cube-focus"/></svg>
      iris for macOS
      <span class="iris-download__arch">Apple Silicon</span>
    </h2>
    <p class="iris-download__primary-sub">
      Signed + notarized <strong>.pkg</strong> · <span data-iris-size>~48 MB</span> · macOS 13 Ventura or newer
    </p>
  </div>

  <a
    class="iris-hero__cta iris-hero__cta--primary iris-download__cta"
    data-primary-link
    href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-0.1.0.pkg"
  >
    <svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
    <span data-primary-label>Download for macOS</span>
  </a>

  <details class="iris-download__alt">
    <summary>Other macOS options</summary>
    <ul class="iris-download__alt-list">
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris_0.1.0_x64.dmg">
          <strong>Intel Mac DMG</strong> — drag-to-install, CLI set up on first launch
        </a>
      </li>
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris_0.1.0_aarch64.dmg">
          <strong>Apple Silicon DMG</strong> — drag-to-install alternative to the PKG
        </a>
      </li>
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-aarch64-apple-darwin.tar.gz">
          <strong>CLI only (Apple Silicon)</strong> — headless agent use
        </a>
      </li>
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-x86_64-apple-darwin.tar.gz">
          <strong>CLI only (Intel)</strong> — headless agent use
        </a>
      </li>
    </ul>
  </details>
</div>

</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#check-circle-fill"/></svg>
    What the installer does
  </span>
  <h2>Four steps, zero prompts</h2>
  <p>The PKG runs a post-install script that wires everything up. No terminal needed.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#squares-four"/></svg> 1. iris.app → Applications
The Tauri observatory lands in <code>/Applications/iris.app</code>. Launch it to see live corpora, replay sessions, and tune budget configuration.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#terminal-window"/></svg> 2. `iris` CLI → /usr/local/bin
The same binary your agents will call, ready at <code>/usr/local/bin/iris</code>. No shell restart required — a <code>/etc/paths.d/iris</code> entry keeps it on every new terminal.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#lightning"/></svg> 3. Background daemon
A <code>launchd</code> agent (<code>com.iris.desktop</code>) keeps the UDS daemon running on login, so `claude mcp add iris -- iris` connects instantly — no cold start.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#shield-check-fill"/></svg> 4. Signed + notarized
Built with an Apple Developer ID, stapled via `notarytool`. Gatekeeper clears it on first launch without the right-click-Open workaround.
</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#sparkle-fill"/></svg>
    After install
  </span>
  <h2>Wire it into your agent</h2>
  <p>One line per MCP client. iris auto-discovers <code>.iris.toml</code> from the working directory.</p>
</div>

=== "Claude Code"

    ```sh
    cd your-project
    iris init                          # creates .iris.toml + .mcp.json
    claude mcp add iris -- iris        # register the MCP server
    ```

=== "Cursor"

    Add to `~/.cursor/mcp.json`:

    ```json
    {
      "mcpServers": {
        "iris": {
          "command": "iris"
        }
      }
    }
    ```

=== "Custom agent"

    ```sh
    iris --mcp                         # stdio JSON-RPC for any MCP-speaking client
    ```

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#circle"/></svg>
    Other platforms
  </span>
  <h2>Linux & Windows</h2>
  <p>The Tauri app builds for every platform. The PKG auto-PATH magic is macOS-only — elsewhere, installer bundles set up PATH natively.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#package"/></svg> Linux
- [`iris_0.1.0_amd64.deb`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris_0.1.0_amd64.deb) — Debian / Ubuntu
- [`iris_0.1.0_amd64.AppImage`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris_0.1.0_amd64.AppImage) — portable, no install
- [`iris-x86_64-unknown-linux-gnu.tar.gz`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-x86_64-unknown-linux-gnu.tar.gz) — CLI only
- [`iris-aarch64-unknown-linux-gnu.tar.gz`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-aarch64-unknown-linux-gnu.tar.gz) — CLI, ARM64
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#package"/></svg> Windows
- [`iris_0.1.0_x64-setup.exe`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris_0.1.0_x64-setup.exe) — NSIS installer, desktop app + CLI
- [`iris-x86_64-pc-windows-msvc.zip`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-x86_64-pc-windows-msvc.zip) — CLI only
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#terminal-window"/></svg> Shell install (any Unix)
```sh
curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
```
CLI-only; detects platform, verifies SHA-256, installs to `/usr/local/bin` or `~/.local/bin`.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#code"/></svg> From source
```sh
cargo install --git https://github.com/AlrikOlson/iris-rs iris-cli
```
Requires Rust 1.85+. Homebrew tap and `crates.io` publish land with 1.0.
</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#shield-check-fill"/></svg>
    Verify
  </span>
  <h2>SHA-256 checksums</h2>
  <p>Every artifact ships with a matching <code>.sha256</code> file. Verify the PKG:</p>
</div>

```sh
shasum -a 256 -c iris-0.1.0.pkg.sha256
```

All release artifacts are listed at [github.com/AlrikOlson/iris-rs/releases](https://github.com/AlrikOlson/iris-rs/releases).

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#x"/></svg>
    Uninstall
  </span>
  <h2>Clean removal</h2>
</div>

=== "macOS (PKG or DMG)"

    ```sh
    # Stop the daemon
    launchctl unload ~/Library/LaunchAgents/com.iris.desktop.plist 2>/dev/null
    # Remove binaries and app
    sudo rm -f /usr/local/bin/iris /etc/paths.d/iris
    sudo rm -rf /Applications/iris.app
    # Remove per-user state (optional; includes indexed corpora + sessions)
    rm -rf ~/.iris ~/Library/Application\ Support/com.iris.desktop
    ```

=== "Linux"

    ```sh
    # Debian/Ubuntu
    sudo apt remove iris
    # Or portable binary
    rm -f /usr/local/bin/iris
    rm -rf ~/.iris ~/.config/iris
    ```

=== "Windows"

    Control Panel → Programs → Uninstall **iris**. Remove `%APPDATA%\iris` for a full wipe.
