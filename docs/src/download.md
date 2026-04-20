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
  <li><svg class="icon icon-sm"><use href="assets/icons.svg#lightning"/></svg> Native Apple Silicon + Intel builds</li>
</ul>

<aside
  class="iris-download-sticky"
  data-iris-download-sticky
  aria-label="Sticky download link"
  hidden
>
  <a class="iris-download-sticky__link" data-primary-mirror href="https://github.com/AlrikOlson/iris-rs/releases/latest"
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
  data-version="__IRIS_VERSION__"
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
    href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-__IRIS_VERSION__.pkg"
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
        <span data-iris-version>v__IRIS_VERSION__</span>
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
    <summary><span class="iris-download__alt-marker" aria-hidden="true">›</span>Other macOS options</summary>
    <ul class="iris-download__alt-list">
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris___IRIS_VERSION___x64.dmg">
          <strong>Intel Mac DMG</strong> — drag-to-install, CLI set up on first launch
        </a>
      </li>
      <li>
        <a href="https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris___IRIS_VERSION___aarch64.dmg">
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

<div class="iris-app-preview iris-app-preview--download" role="img" aria-label="Preview of the iris desktop observatory — macOS window showing a sidebar of three corpora (iris-rs active with 4128 docs, docs, research-notes), two live sessions with budget percentages, a query playground displaying two ranked results for authentication middleware, and an indexing progress bar at 68 percent">
  <div class="iris-app-preview__chrome">
    <div class="iris-app-preview__dots">
      <span class="iris-app-preview__dot iris-app-preview__dot--r"></span>
      <span class="iris-app-preview__dot iris-app-preview__dot--y"></span>
      <span class="iris-app-preview__dot iris-app-preview__dot--g"></span>
    </div>
    <span class="iris-app-preview__title">iris — observatory</span>
    <span class="iris-app-preview__status">
      <span class="iris-app-preview__led"></span>
      daemon connected
    </span>
  </div>
  <div class="iris-app-preview__body">
    <aside class="iris-app-preview__sidebar">
      <div class="iris-app-preview__sidebar-label">Corpora · 3</div>
      <ul class="iris-app-preview__list">
        <li class="iris-app-preview__row iris-app-preview__row--active">
          <span class="iris-app-preview__row-name">iris-rs</span>
          <span class="iris-app-preview__row-meta">4128 docs</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">docs/</span>
          <span class="iris-app-preview__row-meta">312 docs</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">research-notes</span>
          <span class="iris-app-preview__row-meta">57 docs</span>
        </li>
      </ul>
      <div class="iris-app-preview__sidebar-label">Sessions · 2 live</div>
      <ul class="iris-app-preview__list">
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">claude-code · main</span>
          <span class="iris-app-preview__row-meta">42% budget</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">cursor · refactor</span>
          <span class="iris-app-preview__row-meta">18% budget</span>
        </li>
      </ul>
    </aside>
    <div class="iris-app-preview__main">
      <div class="iris-app-preview__panel">
        <div class="iris-app-preview__panel-header">
          <span class="iris-app-preview__panel-title">Query playground</span>
          <span class="iris-app-preview__panel-meta">iris_survey · 5 hits · 42 ms</span>
        </div>
        <div class="iris-app-preview__query">authentication middleware</div>
        <div class="iris-app-preview__results">
          <div class="iris-app-preview__result">
            <div class="iris-app-preview__result-head">
              <span class="iris-app-preview__result-path">src/auth.rs › login</span>
              <span class="iris-app-preview__score">0.91</span>
            </div>
            <p class="iris-app-preview__snippet">Validates JWT tokens using RS256 and calls <code>validate_token</code>…</p>
          </div>
          <div class="iris-app-preview__result">
            <div class="iris-app-preview__result-head">
              <span class="iris-app-preview__result-path">src/auth.rs › logout</span>
              <span class="iris-app-preview__score">0.87</span>
            </div>
            <p class="iris-app-preview__snippet">Revokes the session cookie and blacklists the refresh token until…</p>
          </div>
        </div>
      </div>
      <div class="iris-app-preview__panel">
        <div class="iris-app-preview__panel-header">
          <span class="iris-app-preview__panel-title">Indexing · iris-rs</span>
          <span class="iris-app-preview__panel-meta">2812 / 4128 sections</span>
        </div>
        <div class="iris-app-preview__progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="68" aria-label="Indexing progress">
          <span class="iris-app-preview__progress-fill" style="width: 68%"></span>
        </div>
      </div>
    </div>
  </div>
</div>

<p class="iris-app-preview__caption">Here's what lands in <code>/Applications</code> — the observatory attached to your local daemon.</p>

<div class="iris-install-flow" aria-label="Install flow" data-iris-flow>
  <ol class="iris-install-flow__steps">
    <li
      class="iris-install-flow__step"
      data-mac="Download the .pkg"
      data-win="Download the .exe installer"
      data-lin="Download the .AppImage"
    >
      <span class="iris-install-flow__num">1</span>
      <span class="iris-install-flow__label">Download the .pkg</span>
    </li>
    <li
      class="iris-install-flow__step"
      data-mac="Double-click to run"
      data-win="Run the NSIS wizard"
      data-lin="chmod +x iris.AppImage"
    >
      <span class="iris-install-flow__num">2</span>
      <span class="iris-install-flow__label">Double-click to run</span>
    </li>
    <li
      class="iris-install-flow__step"
      data-mac="Launch iris from /Applications"
      data-win="Launch iris from Start Menu"
      data-lin="Launch the AppImage"
    >
      <span class="iris-install-flow__num">3</span>
      <span class="iris-install-flow__label">Launch iris from /Applications</span>
    </li>
    <li
      class="iris-install-flow__step iris-install-flow__step--final"
      data-mac='<code>iris</code> ready on your <code>PATH</code>'
      data-win='<code>iris</code> ready on your <code>PATH</code>'
      data-lin='<code>iris</code> ready — symlink to your PATH'
    >
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
    Where it lands
  </span>
  <h2>Exactly what the installer writes</h2>
  <p>No post-install script surprises. Every path the PKG touches is listed here and reversed by the uninstall commands at the bottom.</p>
</div>

<pre class="iris-install-tree" aria-label="Filesystem paths touched by the installer"><code><span class="iris-install-tree__root">/</span>
├── <span class="iris-install-tree__dir">Applications/</span>
│   └── <span class="iris-install-tree__leaf">iris.app</span>                      <span class="iris-install-tree__note">← desktop observatory</span>
├── <span class="iris-install-tree__dir">usr/local/bin/</span>
│   └── <span class="iris-install-tree__leaf">iris</span>                          <span class="iris-install-tree__note">← CLI · agents + scripts</span>
└── <span class="iris-install-tree__dir">etc/paths.d/</span>
    └── <span class="iris-install-tree__leaf">iris</span>                          <span class="iris-install-tree__note">← adds /usr/local/bin to PATH</span>

<span class="iris-install-tree__root">~/</span>
├── <span class="iris-install-tree__dir">.iris/</span>                            <span class="iris-install-tree__note">← corpora + vector index + session shadow</span>
└── <span class="iris-install-tree__dir">Library/</span>
    ├── <span class="iris-install-tree__dir">LaunchAgents/</span>
    │   └── <span class="iris-install-tree__leaf">com.iris.desktop.plist</span>        <span class="iris-install-tree__note">← auto-start on login</span>
    └── <span class="iris-install-tree__dir">Application Support/</span>
        └── <span class="iris-install-tree__leaf">com.iris.desktop/</span>         <span class="iris-install-tree__note">← app preferences + logs</span></code></pre>

<p class="iris-install-tree__footer">
  <svg class="icon icon-sm"><use href="assets/icons.svg#shield-check-fill"/></svg>
  Everything is signed with an Apple Developer ID and stapled via <code>notarytool</code>, so Gatekeeper clears the install on first launch.
</p>

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
    iris                               # stdio JSON-RPC; `serve` is the default subcommand
    ```

<div class="iris-section-header" id="other-platforms">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#circle"/></svg>
    Other platforms
  </span>
  <h2>Linux & Windows</h2>
  <p>The desktop app builds for every platform. The PKG auto-PATH magic is macOS-only — elsewhere, installer bundles set up PATH natively.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#package"/></svg> Linux
- [`iris___IRIS_VERSION___amd64.deb`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris___IRIS_VERSION___amd64.deb) — Debian / Ubuntu
- [`iris___IRIS_VERSION___amd64.AppImage`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris___IRIS_VERSION___amd64.AppImage) — portable, no install
- [`iris-x86_64-unknown-linux-gnu.tar.gz`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-x86_64-unknown-linux-gnu.tar.gz) — CLI only
- [`iris-aarch64-unknown-linux-gnu.tar.gz`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-aarch64-unknown-linux-gnu.tar.gz) — CLI, ARM64
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#package"/></svg> Windows
- [`iris___IRIS_VERSION___x64-setup.exe`](https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris___IRIS_VERSION___x64-setup.exe) — NSIS installer, desktop app + CLI
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
shasum -a 256 -c iris-__IRIS_VERSION__.pkg.sha256
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
  <div class="iris-changelog__item iris-changelog__item--skeleton" aria-hidden="true" hidden>
    <span class="iris-changelog__tag">v__IRIS_VERSION__</span>
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
  <div>
    <p>No. Everything runs on your machine — the embedding model, the index, the session state. The only network activity iris initiates is <code>iris_fetch</code> / <code>iris_clone</code> when you explicitly ask it to add web or Git sources to your corpus. No telemetry, no analytics, no update pings.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>Do I need the desktop app, or can I just use the CLI?</summary>
  <div>
    <p>Either works. The <code>iris</code> CLI on your <code>PATH</code> is the only piece your agent talks to. The desktop app is an optional observatory — useful for inspecting corpora, replaying sessions, and tuning configuration visually. If you're headless on a server, grab the CLI-only tarball instead.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>PKG or DMG — which should I pick?</summary>
  <div>
    <p>The PKG is recommended because it wires the CLI into <code>/usr/local/bin/iris</code> and registers <code>/etc/paths.d/iris</code> during install, so <code>iris</code> works in any terminal immediately. The DMG drags <code>iris.app</code> into Applications, and the app installs the CLI to <code>~/.iris/bin/iris</code> on first launch — functionally equivalent, but the shell profile edit happens async instead of during install.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does iris work with Cursor / Zed / Windsurf / Continue / Cline?</summary>
  <div>
    <p>Yes. iris speaks the <a href="https://modelcontextprotocol.io">Model Context Protocol</a> over stdio JSON-RPC. Any MCP-compatible client can connect by registering the <code>iris</code> command. See the <a href="../client-setup/">client setup guide</a> for tested configurations.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>How much disk space does iris use?</summary>
  <div>
    <p>The app and CLI are small. Per-corpus storage scales with what you index — a typical project codebase fits comfortably alongside its own source tree.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does the installer need admin / sudo?</summary>
  <div>
    <p>The PKG does — writing to <code>/Applications</code> and <code>/usr/local/bin</code> requires root. macOS will prompt once with Touch ID or password. The DMG and the CLI tarball don't need admin: DMG lets you drag to a user-level Applications folder, and the tarball can extract anywhere on your <code>PATH</code>. If you're on a locked-down work laptop, <code>curl | bash</code> the install script — it falls back to <code>~/.local/bin</code> automatically when <code>/usr/local/bin</code> isn't writable.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>Does iris cost anything?</summary>
  <div>
    <p>No. iris is free and open source under MIT OR Apache-2.0. No paid tier, no license server, no vendor lock-in. You can fork it, embed it, ship it as part of your own product — both licenses permit that.</p>
  </div>
</details>

<details class="iris-faq__item">
  <summary>How do updates work?</summary>
  <div>
    <p>Pull the next PKG / DMG from the releases page and run it — the installer overwrites the app bundle and CLI in place. The session shadow, corpora, and indexed content in <code>~/.iris/</code> are preserved across upgrades. If you installed via Homebrew (post-1.0) or the shell install script, <code>brew upgrade iris</code> / re-running the install script does the same job.</p>
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
      "softwareVersion": "__IRIS_VERSION__",
      "downloadUrl": "https://github.com/AlrikOlson/iris-rs/releases/latest/download/iris-__IRIS_VERSION__.pkg",
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
            "text": "No. Everything runs on your machine — the embedding model, the index, the session state. The only network activity iris initiates is iris_fetch / iris_clone when you explicitly ask it to add web or Git sources to your corpus. No telemetry, no analytics, no update pings."
          }
        },
        {
          "@type": "Question",
          "name": "Do I need the desktop app, or can I just use the CLI?",
          "acceptedAnswer": {
            "@type": "Answer",
            "text": "Either works. The iris CLI on your PATH is the only piece your agent talks to. The desktop app is an optional observatory — useful for inspecting corpora, replaying sessions, and tuning configuration visually."
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
            "text": "The app and CLI are small. Per-corpus storage scales with what you index — a typical project codebase fits comfortably alongside its own source tree."
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
