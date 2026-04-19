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

<ul class="iris-trust-strip" aria-label="Trust signals">
  <li><svg class="icon icon-sm"><use href="assets/icons.svg#shield-check-fill"/></svg> Apple-signed + notarized</li>
  <li><svg class="icon icon-sm"><use href="assets/icons.svg#cpu"/></svg> 100% local — no telemetry</li>
  <li><svg class="icon icon-sm"><use href="assets/icons.svg#code"/></svg> Open source · MIT / Apache-2.0</li>
  <li><svg class="icon icon-sm"><use href="assets/icons.svg#lightning"/></svg> Universal binary · arm64 + x64</li>
</ul>

<aside
  class="iris-download-sticky"
  data-iris-download-sticky
  role="region"
  aria-label="Sticky download link"
  hidden
>
  <a class="iris-download-sticky__link" data-primary-mirror href="#"
    ><svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
    <span>
      <span class="iris-download-sticky__label" data-mirror-label>Download iris</span>
      <span class="iris-download-sticky__sub" data-mirror-sub>macOS · Apple Silicon</span>
    </span>
  </a>
  <button
    type="button"
    class="iris-download-sticky__dismiss"
    aria-label="Dismiss sticky download bar"
    data-sticky-dismiss
  >×</button>
</aside>

<div
  class="iris-download"
  data-iris-download
  data-version="0.1.0"
  data-repo="AlrikOlson/iris-rs"
  data-release-base="https://github.com/AlrikOlson/iris-rs/releases/latest/download"
>

<div class="iris-download__primary" data-target="macos-arm64" data-state="detecting">
  <div class="iris-download__primary-meta">
    <span class="iris-download__badge" data-primary-badge>Recommended for your Mac</span>
    <h2 class="iris-download__primary-title">
      <svg class="icon icon-md"><use href="assets/icons.svg#cube-focus"/></svg>
      iris for macOS
      <span class="iris-download__arch" data-iris-arch>Apple Silicon</span>
    </h2>
    <p class="iris-download__primary-sub">
      Signed + notarized <strong>.pkg</strong> · <span data-iris-size>~48 MB</span> · <span data-iris-minos>macOS 13 Ventura or newer</span>
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

  <p class="iris-download__caption" data-iris-caption>
    Detecting your platform… <a href="#other-platforms" class="iris-download__caption-link">See all downloads</a>
  </p>

  <p class="iris-download__proof" data-iris-proof hidden>
    <svg class="icon icon-sm"><use href="assets/icons.svg#check-circle-fill"/></svg>
    <span data-iris-proof-text>—</span>
  </p>

  <div class="iris-download__post-click" data-iris-postclick hidden>
    <svg class="icon icon-sm"><use href="assets/icons.svg#check-circle-fill"/></svg>
    <div>
      <strong>Downloading <span data-postclick-file>iris.pkg</span>…</strong>
      <span>Next up: <a href="#wire-it-into-your-agent">wire iris into your agent →</a></span>
    </div>
  </div>

  <div class="iris-download__release" data-iris-release hidden>
    <div class="iris-download__release-line">
      <span class="iris-download__release-item">
        <svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
        <span data-iris-version>v0.1.0</span>
      </span>
      <span class="iris-download__release-sep">·</span>
      <span class="iris-download__release-item" data-iris-reldate>released recently</span>
      <span class="iris-download__release-sep">·</span>
      <a class="iris-download__release-item" data-iris-notes href="https://github.com/AlrikOlson/iris-rs/releases/latest">
        <svg class="icon icon-sm"><use href="assets/icons.svg#sparkle-fill"/></svg>
        What's new
      </a>
    </div>
    <p class="iris-download__release-preview" data-iris-preview hidden></p>
  </div>

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
      <li class="iris-download__alt-item--muted">
        <span><strong>Homebrew</strong> — <code>brew install AlrikOlson/tap/iris</code> · <em>lands with 1.0</em></span>
      </li>
    </ul>
  </details>
</div>

</div>

</div>

<div class="iris-install-flow" aria-label="Install flow">
  <ol class="iris-install-flow__steps">
    <li class="iris-install-flow__step">
      <span class="iris-install-flow__num">1</span>
      <span class="iris-install-flow__label">Download the .pkg</span>
    </li>
    <li class="iris-install-flow__step">
      <span class="iris-install-flow__num">2</span>
      <span class="iris-install-flow__label">Double-click to run</span>
    </li>
    <li class="iris-install-flow__step">
      <span class="iris-install-flow__num">3</span>
      <span class="iris-install-flow__label">Launch iris from /Applications</span>
    </li>
    <li class="iris-install-flow__step iris-install-flow__step--final">
      <span class="iris-install-flow__num">✓</span>
      <span class="iris-install-flow__label">
        <code>iris</code> ready on your <code>PATH</code>
      </span>
    </li>
  </ol>
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

<div class="iris-section-header" id="wire-it-into-your-agent">
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

<div class="iris-section-header" id="other-platforms">
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

<div class="iris-section-header" id="changelog">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#sparkle-fill"/></svg>
    Recent releases
  </span>
  <h2>What's shipped lately</h2>
  <p>Pulled live from GitHub. Older versions and full changelogs are on the releases page.</p>
</div>

<div
  class="iris-changelog"
  data-iris-changelog
  data-repo="AlrikOlson/iris-rs"
  data-fallback
>
  <div class="iris-changelog__item iris-changelog__item--skeleton" aria-hidden="true">
    <span class="iris-changelog__tag">v0.1.0</span>
    <span class="iris-changelog__date">—</span>
    <span class="iris-changelog__body">Loading recent releases…</span>
  </div>
  <p class="iris-changelog__fallback">
    <a href="https://github.com/AlrikOlson/iris-rs/releases">See the full changelog on GitHub →</a>
  </p>
</div>

<div class="iris-section-header" id="faq">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#compass-tool"/></svg>
    FAQ
  </span>
  <h2>Common questions</h2>
  <p>What new users ask before hitting download. Click a question to expand.</p>
</div>

<div class="iris-faq">

<details class="iris-faq__item">
  <summary>Does iris phone home or send data anywhere?</summary>
  <div markdown>
No. The daemon listens on a local Unix domain socket, embeddings run locally via ONNX (optionally Metal-accelerated), and the vector index lives on disk under `~/.iris/`. The only network activity iris initiates is `iris_fetch` / `iris_clone` when you explicitly ask it to add web or Git sources to your corpus. No telemetry, no analytics, no update pings.
  </div>
</details>

<details class="iris-faq__item">
  <summary>Do I need the desktop app, or can I just use the CLI?</summary>
  <div markdown>
Either works. The MCP proxy ships as `iris` on your `PATH` and is the only piece your agent talks to. The desktop app is an optional observatory that attaches to the same daemon — it's useful for inspecting corpora, replaying sessions, and tuning configuration visually. If you're headless on a server, grab the CLI-only tarball instead.
  </div>
</details>

<details class="iris-faq__item">
  <summary>PKG or DMG — which should I pick?</summary>
  <div markdown>
The PKG is recommended because it wires the CLI into `/usr/local/bin/iris` and registers `/etc/paths.d/iris` during install, so `iris` works in any terminal immediately. The DMG drags `iris.app` into Applications, and the app installs the CLI to `~/.iris/bin/iris` on first launch — functionally equivalent, but the shell profile edit happens async instead of during install.
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does iris work with Cursor / Zed / Windsurf / Continue / Cline?</summary>
  <div markdown>
Yes. iris speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio JSON-RPC. Any MCP-compatible client can connect by registering the `iris` command. See the [client setup guide](client-setup.md) for tested configurations, or just run `iris --mcp` and point any MCP client at it.
  </div>
</details>

<details class="iris-faq__item">
  <summary>How much disk space does iris use?</summary>
  <div markdown>
The binary itself is ~14 MB. The desktop app bundle is ~35 MB. Per-corpus storage depends on the size of what you index — a 10k-file codebase typically lands around 150–300 MB (content DB + HNSW index + symbol index). Embeddings are quantized int8, which keeps the vector store small. See the [benchmarks page](benchmarks.md) for reference sizes.
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does the installer need admin / sudo?</summary>
  <div markdown>
The PKG does — writing to `/Applications` and `/usr/local/bin` requires root. macOS will prompt once with Touch ID or password. The DMG and the CLI tarball don't need admin: DMG lets you drag to a user-level Applications folder, and the tarball can extract anywhere on your `PATH`. If you're on a locked-down work laptop, `curl | bash` the install script — it falls back to `~/.local/bin` automatically when `/usr/local/bin` isn't writable.
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does iris cost anything?</summary>
  <div markdown>
No. iris is free and open source under MIT OR Apache-2.0. No paid tier, no license server, no vendor lock-in. You can fork it, embed it, ship it as part of your own product — both licenses permit that.
  </div>
</details>

<details class="iris-faq__item">
  <summary>How do updates work?</summary>
  <div markdown>
Pull the next PKG / DMG from the releases page and run it — the installer overwrites the app bundle and CLI in place. The session shadow, corpora, and indexed content in `~/.iris/` are preserved across upgrades. If you installed via Homebrew (post-1.0) or the shell install script, `brew upgrade iris` / re-running the install script does the same job.
  </div>
</details>

</div>

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

<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "SoftwareApplication",
      "@id": "https://AlrikOlson.github.io/iris-rs/#iris",
      "name": "iris",
      "operatingSystem": "macOS 13, Windows 10, Linux (glibc 2.31+)",
      "applicationCategory": "DeveloperApplication",
      "applicationSubCategory": "MCP server · Context cache for LLM agents",
      "description": "Context cache for LLM agents — MCP server with semantic search, code navigation, session tracking, and budget awareness. Includes a desktop observatory app and a CLI in a single installer.",
      "softwareVersion": "0.1.0",
      "downloadUrl": "https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-0.1.0.pkg",
      "installUrl": "https://AlrikOlson.github.io/iris-rs/download/",
      "releaseNotes": "https://github.com/AlrikOlson/iris-rs/releases/latest",
      "license": "https://github.com/AlrikOlson/iris-rs/blob/main/LICENSE-MIT",
      "offers": {
        "@type": "Offer",
        "price": "0",
        "priceCurrency": "USD"
      },
      "author": {
        "@type": "Person",
        "name": "Alrik Olson",
        "url": "https://github.com/AlrikOlson"
      }
    },
    {
      "@type": "FAQPage",
      "mainEntity": [
        {
          "@type": "Question",
          "name": "Does iris phone home or send data anywhere?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "No. The daemon listens on a local Unix domain socket, embeddings run locally via ONNX (optionally Metal-accelerated), and the vector index lives on disk under ~/.iris/. The only network activity iris initiates is iris_fetch / iris_clone when you explicitly ask it to add web or Git sources to your corpus. No telemetry, no analytics, no update pings."
          }
        },
        {
          "@type": "Question",
          "name": "Do I need the desktop app, or can I just use the CLI?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "Either works. The MCP proxy ships as iris on your PATH and is the only piece your agent talks to. The desktop app is an optional observatory that attaches to the same daemon — it's useful for inspecting corpora, replaying sessions, and tuning configuration visually."
          }
        },
        {
          "@type": "Question",
          "name": "PKG or DMG — which should I pick?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "The PKG is recommended because it wires the CLI into /usr/local/bin/iris and registers /etc/paths.d/iris during install, so iris works in any terminal immediately. The DMG drags iris.app into Applications, and the app installs the CLI on first launch."
          }
        },
        {
          "@type": "Question",
          "name": "Does iris work with Cursor, Zed, Windsurf, Continue, or Cline?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "Yes. iris speaks the Model Context Protocol over stdio JSON-RPC. Any MCP-compatible client can connect by registering the iris command."
          }
        },
        {
          "@type": "Question",
          "name": "How much disk space does iris use?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "The binary is ~14 MB, the desktop app bundle is ~35 MB, and per-corpus storage depends on what you index — a 10k-file codebase typically lands around 150–300 MB. Embeddings are quantized int8 to keep the vector store small."
          }
        },
        {
          "@type": "Question",
          "name": "Does the installer need admin or sudo?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "The PKG does — writing to /Applications and /usr/local/bin requires root. macOS prompts once with Touch ID or password. The DMG and CLI tarball don't need admin: drag to Applications, or extract anywhere on PATH. The shell install script falls back to ~/.local/bin when /usr/local/bin isn't writable."
          }
        },
        {
          "@type": "Question",
          "name": "Does iris cost anything?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "No. iris is free and open source under MIT OR Apache-2.0. No paid tier, no license server, no vendor lock-in."
          }
        },
        {
          "@type": "Question",
          "name": "How do updates work?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "Pull the next PKG or DMG from the releases page and run it — the installer overwrites the app bundle and CLI in place. Session shadow, corpora, and indexed content in ~/.iris/ are preserved across upgrades."
          }
        }
      ]
    }
  ]
}
</script>
