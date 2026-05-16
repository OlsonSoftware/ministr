# Changelog

All notable changes to ministr will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Language & tech-stack coverage
- Tree-sitter grammars for 17 more languages, all default-on: Bash/Shell,
  PHP, Scala, Lua, Elixir, Haskell, OCaml (impl + interface), Dart, R,
  HCL/Terraform, JSON, YAML, TOML, SQL, Zig, Protobuf, and Svelte
  (single-file components). These file types previously fell back to
  text-only chunking; `ministr_symbols` / `ministr_definition` /
  `ministr_references` now work across ~29 languages. (Dockerfile, Vue,
  and Astro have no ABI-current Rust grammar and keep the lossless text
  fallback.)
- **Activated the dormant `LanguageRefinement` system** — `refinement_for`
  was defined but never called, so every non-Rust language used only the
  generic heuristic. It is now wired into the extractor, with refinements
  for Protobuf (`message`/`enum`/`service`), HCL/Terraform (`resource.`/
  `module.`/`variable.`/`output.` block addresses), and SQL (`CREATE
  TABLE`/`VIEW`/`FUNCTION`/…).
- Import cross-references for PHP, Kotlin, and Scala (`ministr_references`).
- `ministr_bridge` — four new bridge kinds (11 total): **cgo** (Go
  `C.func` ↔ C), **JNI** (Java/Kotlin `native` ↔ C/C++ `Java_*`),
  **UniFFI** (Rust `#[uniffi::export]` ↔ Swift/Kotlin/Python), and
  **gRPC** (`.proto` `service` ↔ generated stubs), with framework
  auto-detection signals for each.
- `ministr init` language rules now also cover PHP and Ruby.

#### Maximal coverage expansion
- Tree-sitter grammars for **11 more languages**, all default-on: CSS/SCSS,
  GraphQL, Groovy/Gradle, Nix, Erlang, PowerShell, Solidity, Objective-C
  (+ObjC++), Julia, CMake, and Make. `ministr_symbols` /
  `ministr_definition` / `ministr_references` now work across ~40
  languages. (Markdown and HTML keep their dedicated prose/markup
  parsers, which outperform a code AST; Clojure has no ABI-current Rust
  grammar — its crates.io latest hard-pins legacy tree-sitter — so it
  keeps the lossless text fallback, alongside Dockerfile/Vue/Astro.)
- **21 new `LanguageRefinement` implementations** so previously
  generic-heuristic languages get accurate symbol kinds: Ruby, PHP,
  Scala, C#, JavaScript, Bash, Lua, Haskell, OCaml, Dart, R, Zig, plus
  the structure-heavy newcomers CSS, GraphQL, Groovy, Solidity, Erlang,
  Julia, CMake, and Make. (Delegate-on-unknown — never a regression.)
- Import cross-references for **Java, C#, Swift, and Ruby**
  (`ministr_references`) — JVM-style dotted imports and Ruby
  `require`/`require_relative`/`load`/`autoload`.
- `ministr_bridge` — **two new bridge kinds (13 total)**: **Flutter
  platform channels** (Dart `MethodChannel`/`EventChannel`/
  `BasicMessageChannel` ↔ native Kotlin/Java/Swift/ObjC) and **Electron
  IPC** (`ipcMain.handle`/`on` ↔ `ipcRenderer.invoke`/`send`/`on`), with
  `pubspec.yaml` and `electron`-in-`package.json` auto-detection.
- `ministr init` language rules now also cover C#, Kotlin, Swift, Scala,
  C/C++, Elixir, and JavaScript (manifest-detected via `*.csproj`/`*.sln`,
  `*.gradle.kts`, `Package.swift`, `build.sbt`, `CMakeLists.txt`,
  `mix.exs`, and tsconfig-less `package.json`) — 13 languages total.

#### Smarter project / ignore autodetection
- Global ignore overhaul (sourced from the canonical `github/gitignore`
  templates). `ALWAYS_IGNORE_DIRS` now prunes committed vendored-dep
  trees (`3rdparty`, `third_party`, `extern`, `deps`, `_deps`,
  `bower_components`, …) and per-ecosystem cache/build dirs
  (`.dart_tool`, `.svelte-kit`, `.turbo`, `CMakeFiles`, `Pods`,
  `DerivedData`, `.elixir_ls`, `.eggs`, …). New directory *glob*
  ignores: `bazel-*`, `cmake-build-*`, `*.egg-info`, `*.xcodeproj`,
  `*.xcworkspace`, `*.framework`. New generated-binding file ignores
  (`*.pb.go`, `*_pb2.py`, `*_pb2_grpc.py`, `*.pb.cc/.h`, `*.g.dart`,
  `*.Designer.cs`, `moc_*.cpp`, …). A vendored-dep C/C++ tree no longer
  drowns the real engine code in semantic/symbol search.
- Project-type-gated ignores in `ministr init`: `bin/`+`obj/` for .NET,
  `Library/`/`Temp/`/`Obj/`/`Logs/` for Unity, `Binaries/`/
  `Intermediate/`/`Saved/`/`DerivedDataCache/` for Unreal, `.build/`
  for SwiftPM — names too generic to ignore globally, unambiguous once
  the project type is known.
- `detect_source_paths` is now polyglot: conventional source roots for
  every detected language (Go `cmd`/`internal`/`pkg`, JVM
  `src/main/{java,kotlin,scala}`, C/C++ `src`/`include`/`Source`, Swift
  `Sources`, Elixir/Dart `lib`, …) instead of the old rust/node/python
  trio — additive only, so a misdetection can never hide real code.
- Informal polyglot monorepos (≥2 language ecosystems at the root with
  no workspace manifest) are now classified as `Monorepo`.

### Changed

### Fixed

## [0.2.0] - 2026-04-28

### Added

#### Code navigation
- `ministr_symbols`, `ministr_definition`, `ministr_references` — code symbol index across 12 languages via tree-sitter
- `ministr_bridge` — cross-language bridge detection across seven kinds: Tauri commands and events, napi-rs, PyO3, wasm-bindgen, HTTP routes (actix-web / axum / rocket), and raw FFI

#### Retrieval
- Two-stage Matryoshka retrieval — corpus-configurable target dimension (`corpus.dimension`) with full-dimension HNSW rescoring (`corpus.rerank_depth`)
- SPLADE sparse embeddings + dense vectors with reciprocal rank fusion
- Optional cross-encoder reranking — when enabled, rescores the top vector-search candidates and blends the cross-encoder score with the upstream retrieval score (`RERANK_BLEND = 0.8`) before truncation to `top_k`
- Candle Metal GPU embedding backend (optional, Apple Silicon)

#### Session & eviction
- Attention-position-aware eviction scoring (Lost in the Middle bias)
- FSRS spaced-repetition memory model for context retention
- Multi-tier compression with pluggable strategies and quality scoring

#### Multi-source corpora
- `ministr_fetch` — fetch and index web content
- `ministr_clone` — clone and index git repositories
- `ministr_refresh` — detect and re-fetch stale sources

#### Architecture
- `ministr-daemon` — HTTP API over Unix domain socket
- `ministr-api` — shared request/response types and `DaemonClient`
- `ministr-app` — Tauri v2 desktop app with system tray and dashboard
- `/activity` endpoint with `?limit` and `?since` for incremental polling
- Automatic stdio-to-HTTP proxy when a second client connects
- Streamable HTTP transport for remote deployments
- Concurrent producer-consumer ingestion pipeline

#### MCP protocol
- Structured output schemas and tool annotations for all tools
- Elicitation prompts for budget, compression, and search decisions
- Async tasks for long-running operations (fetch, clone, index)

#### Distribution
- Docker image with cargo-chef cached builds
- Fly.io and Railway deployment configs with Caddy/nginx templates
- Signed and notarized macOS `.pkg` installer
- `ministr init` — project scaffolding with `.ministr.toml` and MCP client configs
- Hot-add of new corpus paths in `.ministr.toml` — newly-added entries under `[corpus] paths` are ingested without restarting the MCP session. Other config changes (path removals, model swaps, `[agent]` rule edits) still require a restart.
- Retrieval evaluation suite with MRR/nDCG and CI regression gate

#### Desktop app
- **Dashboard overhaul** — Overview home with aggregate budget ring, cache-hit history, and live turn stream
- `BudgetRing`, `TurnBlock`, `CorpusChip`, `StatusDot`, and `ActivityFeed` UI primitives
- Command palette (`⌘K`) with corpus navigation and theme/tab actions
- Keyboard shortcut sheet (`?`) and theme toggle (System / Dark / Light) in the TopBar
- Tray submenus for active sessions, recent corpora, and quick actions
- Live tool-call **activity stream** — every `ministr_*` MCP call is recorded in a 500-event in-memory ring buffer on the daemon, exposed via `GET /activity` and surfaced in the app Overview
- **Coherence feed** — rich per-file `CoherenceEvent` (kind + path + affected sections) broadcast from the per-corpus watcher, mirrored in a 500-event daemon-wide ring buffer, exposed via `GET /coherence-events` + `CoherenceFeed` UI primitive replacing the Overview placeholder
- Onboarding "dashboard preview" tile so users see the dashboard aesthetic before reaching the dense dashboard
- `CorpusTreemap` re-themed with OKLCH language colors that share the ministr design tokens

#### Documentation
- New documentation site built with Fumadocs on Next.js 16, deployed to Cloudflare Pages
- Mermaid diagrams throughout the architecture docs (replacing ASCII art)
- ⌘K search, reading-progress indicator, and keyboard shortcuts on the docs site
- Asciinema terminal demo on the landing page

### Changed

- Release pipeline consolidated to a single workflow on a single tag.
  `vX.Y.Z` now produces one GitHub Release containing every artifact
  for that version: CLI tarballs (`ministr-<target>.tar.gz` / `.zip`)
  for headless installs, plus desktop installers
  (`ministr-desktop-<target>.<dmg|exe|deb|AppImage>`) for macOS aarch64,
  Windows x86_64, and Linux x86_64. The previous two-tag dance
  (`vX.Y.Z` for CLI + `vX.Y.Z-app` for the Tauri app) is gone — the
  separate `app-release.yml` workflow has been deleted, the
  `bundle-windows` duplication in `release.yml` is folded into the
  unified `desktop` matrix, and `tauri.conf.json` no longer carries a
  separate `version` field (Tauri reads `ministr-app/src-tauri/Cargo.toml`
  directly, so `just release X.Y.Z` only bumps one source of truth).
- `x86_64-apple-darwin` dropped from the build matrix. `ort-sys`
  2.0.0-rc.11 stopped shipping prebuilts for the target, and macOS 26
  dropped Intel x86_64 entirely. Apple Silicon (`aarch64-apple-darwin`)
  is the supported Mac target; Intel Macs on older macOS can run via
  Rosetta 2 or build from source.
- Primary brand domain migrated from `AlrikOlson.github.io/ministr-rs` to
  `https://ministr.ai`. Docs now deploy at the site root via
  `docs-next/public/CNAME`; the `DOCS_BASE_PATH=/ministr` env var is
  no longer set by the deploy workflow
- Install-script URL shortened to `curl -fsSL https://ministr.app/install.sh | bash`.
  The canonical `install.sh` lives in `docs-next/public/` so
  `https://ministr.ai/install.sh` also resolves. `ministr.app` is served by
  a Cloudflare Single Redirect ruleset (no separate hosting project) that
  301s every path to `https://ministr.ai$path`
- GitHub source repo moved from `AlrikOlson/ministr-rs` to
  `OlsonSoftware/ministr`; CI badges, clone/cargo-install commands, and
  issue links now point at the new location. The original public release
  under the old name stays referenced historically in the `[0.1.0]`
  changelog footer
- Release assets served via Cloudflare Worker at `dl.ministr.app`
  (`workers/release-proxy/`). The Worker fronts the now-private source
  repo's GitHub Releases using a read-only fine-grained PAT, so
  unauthenticated `curl` downloads still work:
  `https://dl.ministr.app/v<tag>/<filename>` → 302 → GitHub CDN. When
  `OlsonSoftware/ministr` is made public the Worker can be deleted and
  `install.sh` rewired straight at
  `github.com/OlsonSoftware/ministr/releases/download/...` — one-file change
- **Breaking:** Tauri bundle identifier changed from `com.ministr.desktop`
  to `ai.ministr.desktop` (reverse-DNS of the primary domain). macOS
  treats existing installs as a separate app — auto-updater won't see
  old installs, keychain entries under the prior identifier become
  orphaned. Launchd plist files and PKG component identifiers updated
  to match
- Workspace `Cargo.toml` gained a `homepage = "https://ministr.ai"` field;
  every crate now inherits via `homepage.workspace = true`
- Workspace expanded from 3 crates to 6 (`ministr-api`, `ministr-daemon`, `ministr-app`)
- Prefetch engine overhauled — `PriorityCache`, adaptive alpha, cache invalidation
- Ingestion pipeline split into focused submodules
- Session budget tracking integrated into daemon for cross-session awareness
- Documentation site rebuilt on Fumadocs/Next.js (replaced earlier mdBook prototype)
- User-facing copy clarified: ministr manages what it sends into context, it does
  not edit the agent's context window

### Fixed

- Navigation router on the docs site no longer breaks on SVG `<use>` elements;
  icons are inlined at build time so Material's URL normalizer has no
  `SVGUseElement.href` to trip on

## [0.1.0] - 2026-03-21

### Added

- **MCP server** with stdio transport — 7 tools for LLM context management:
  - `ministr_survey` — semantic search across a document corpus at multiple resolutions
  - `ministr_read` — retrieve full section content with heading paths
  - `ministr_extract` — extract atomic claims from sections, optionally ranked by query relevance
  - `ministr_related` — follow dependency chains between claims (references, contradicts, depends_on, updates)
  - `ministr_budget` — context budget status with eviction recommendations
  - `ministr_compress` — generate compressed summaries for eviction candidates
  - `ministr_evicted` — explicit eviction feedback from the agent
- **MCP resources** — `ministr://status` for index/session state, `ministr://corpus/{path}` for document metadata
- **Multi-resolution indexing** — documents, section summaries, section text, and atomic claims are embedded and indexed separately
- **Session shadow** — tracks what content has been delivered to the agent, deduplicates repeat deliveries, and detects fault-based evictions
- **Budget tracker** — estimates context window token usage, reports pressure levels, and ranks eviction candidates
- **Prefetch engine** — six prefetch strategies backed by an LRU cache. Post-read: sequential, structural, topical, cross-session (four strategies in default single-process mode; the daemon-proxy path has cross-session scaffolded but not yet triggered). Post-survey: survey-expand, agent-plan (intent-based)
- **Coherence subsystem** — file watcher triggers re-indexing and invalidates stale session entries
- **Cross-session analytics** — tracks section access patterns and feeds co-access data into prefetch
- **Session persistence** — session state survives server restarts via SQLite storage
- **Parsers** — Markdown (via comrak), HTML (via scraper), PDF (via pdf-extract), with auto-detection by file extension
- **Claim relationship index** — directed relationships between claims with confidence scores
- **Extractive summarization** — sentence-level extraction for compress and document summaries
- **HNSW vector index** — fast approximate nearest neighbor search (hnsw_rs)
- **FastEmbed embeddings** — local embedding model via fastembed (no API keys required)
- **CLI** — `ministr` binary with `--corpus` and `--config` flags
- **Configuration** — TOML config file at `~/.ministr/config.toml` with sensible defaults
- **Cross-platform builds** — CI produces binaries for Linux (x86_64, aarch64), macOS (aarch64), and Windows (x86_64)
- **Quality gates** — clippy pedantic, cargo-audit, cargo-deny, and full test suite in CI
- **mdBook documentation** — architecture guide, MCP client setup, and API reference

[Unreleased]: https://github.com/OlsonSoftware/ministr/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/OlsonSoftware/ministr/releases/tag/v0.2.0
[0.1.0]: https://github.com/AlrikOlson/ministr-rs/releases/tag/v0.1.0
