# Changelog

All notable changes to ministr will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Near-instant daemon/CLI restart: the persisted HNSW dump is now loaded as a validated derived *cache* instead of being rebuilt from SQLite on every start. A `{cache-version, model, dimension, vector-count, generation}` validity token (the `generation` is a monotonic counter bumped inside every indexed-vector mutation, stored in the new `index_meta` table) gates the load — any mismatch falls back to a full rebuild, so ADR 0001 D4's no-drift guarantee is preserved while the common unchanged-corpus restart skips the O(N·log N·M) graph construction that cost ~10–18s on large corpora.
- Added a **real end-to-end token-economics measurement** (`ministr-mcp` test `token_economics_e2e`): it indexes a real multi-language corpus, runs real `ministr_survey` calls through the MCP path, and counts the literal response-payload tokens against a real grep+read — measuring ~66% fewer tokens per lookup instead of a synthetic model.
- Added a **side-by-side agent benchmark suite** (`benchmarks/agent-task`): the same headless coding agent solves the same deterministic task with vs without ministr (ministr MCP vs grep/glob), across a difficulty ladder of fixtures and real OSS repos (SWE-bench-style — the repo's own failing→passing tests as a validator), reporting validator pass/fail, tokens, turns, and cost. Real-repo tasks index the repo once and reuse it across the whole matrix.

### Changed
- Recalibrated the public token-economics claim to a real measurement. The homepage figure and `/docs/benchmarks` now report ~66% fewer tokens per lookup (measured end-to-end with ministr's own tokenizer) and honest per-lookup framing, replacing the previous synthetic best-case (~99.7%, "a flat 68 tokens"), which is kept only as an explicitly-labelled illustrative scaling model.

### Fixed
- Fixed `ministr_read` reporting `claims_available: 0` for sections served from the agent-intent prefetch cache: intent-prefetched sections hardcoded a zero claim count, so a warm read could wrongly tell the agent there were no claims to `ministr_extract`. The prefetch now stamps the true per-section claim count. (Also de-flakes the `full_flow_survey_read_extract_via_call_tool` e2e test, whose intermittent failure was this bug surfacing.)

## [0.6.0](https://github.com/OlsonSoftware/ministr/releases/tag/v0.6.0) - 2026-05-19

### Added
- Added a new `ministr_solid` detector and resolver auto-heal flow, plus a backend trait for MCP integration.
- Added per-session attribution for corpus-wide tool calls and a code-intelligence-first session panel in the desktop app.

### Changed
- Reworked the MCP backend architecture to support daemon, local, and multi-daemon backends through a shared conversion layer.
- Redesigned desktop activity rows for denser single-line timeline rendering, clearer labels, repo-relative paths, and pinned count badges.
- Hardened reinstall behavior with atomic replacement of running binaries and macOS dev-bundle install to `~/Applications`.

### Fixed
- Fixed MCP smoke CI reliability by pinning bash with `pipefail`, starting the daemon before smoke checks, and preserving stderr diagnostics.
- Fixed desktop activity rendering issues including broken fresh-event pulses and activity-row column bleed.
- Fixed PATH discovery for app-launched flows by probing common binary directories when `launchd` omits `claude`/`codex`.

### Removed
- Removed the legacy standalone MCP proxy backend implementation in favor of modular backend components.

## [0.5.1](https://github.com/OlsonSoftware/ministr/releases/tag/v0.5.1) - 2026-05-18

### Added
- Added a ministr-vs-LSP code navigation evaluation suite, including runner scaffolding, ground-truth data, and benchmark documentation.

### Changed
- Refined release automation to use a single source of truth for release state and reduced CI workflow duplication.
- Updated repository licensing and contributor-facing guidance for closed-source distribution and code-intelligence branding.

### Fixed
- Fixed release-PR CI skip-guard logic so it no longer depends on the `release` label.
- Corrected benchmark LSP coverage metrics with strict path matching and tightened `cargo-deny` private ignore handling for proprietary crates.

### Removed
- Removed split MIT/Apache license files in favor of a consolidated proprietary license file.

## [0.5.0](https://github.com/OlsonSoftware/ministr/releases/tag/v0.5.0) - 2026-05-18

### Added
- Linked projects support lets one agent session query indexes from other configured projects.

### Changed
- Release automation now supports manual `workflow_dispatch` kick-off for release runs.

### Fixed
- Release detection in CI is now deterministic and fails loudly when release state is invalid.

## [0.4.0](https://github.com/OlsonSoftware/ministr/releases/tag/v0.4.0) - 2026-05-18

### Added
- Index Terraform `.tfvars` files as HCL so variable files are parsed as code.
- Index C++20 module and template-implementation header patterns for symbol extraction.

### Changed
- Release automation now prepares release authoring via the GitHub Copilot cloud agent flow.
- Release workflows avoid redundant build matrices and skip request-release while a version bump is prepared but not yet tagged.

### Fixed
- Extension detection coverage now explicitly includes `.tfvars` and `.hcl`.

### Breaking changes — migration
- Release authoring workflow: manual/previous release-authoring path -> GitHub Copilot cloud-agent-authored release PR flow.

## [0.3.0](https://github.com/OlsonSoftware/ministr/releases/tag/v0.3.0) - 2026-05-17

### Added
- [**breaking**] Agent-rules fixes + budget→usage rename + reproducible benchmark
### Changed
- Correct agent-rules / PreToolUse hook behavior
## [0.2.4](https://github.com/OlsonSoftware/ministr/releases/tag/v0.2.4) - 2026-05-17

### Changed
- Fix false budget_status claim in getting-started- Tool-reference accuracy + Concepts code-intelligence overhaul- Accuracy, branding, focus + de-expose internals- Rebuild /install page in the manuscript aesthetic ([#104](https://github.com/OlsonSoftware/ministr/pull/104))- Rewrite landing page in a manuscript aesthetic ([#103](https://github.com/OlsonSoftware/ministr/pull/103))- Rewrite Architecture section with code-intelligence focus ([#102](https://github.com/OlsonSoftware/ministr/pull/102))
## [0.2.3](https://github.com/OlsonSoftware/ministr/releases/tag/v0.2.3) - 2026-05-17

### Changed
- Rebrand ministr as a code intelligence MCP server ([#100](https://github.com/OlsonSoftware/ministr/pull/100))- Rewrite README in concise manuscript style; gate release automation on non-doc pushes
## [0.2.2](https://github.com/OlsonSoftware/ministr/releases/tag/v0.2.2) - 2026-05-17

### Added
- *(release)* Replace release-plz with git-cliff (no cargo packaging) ([#81](https://github.com/OlsonSoftware/ministr/pull/81))
### Changed
- Wip
### Fixed
- *(ci)* Set git identity before annotated tag in tag-and-release ([#99](https://github.com/OlsonSoftware/ministr/pull/99))- *(ci)* Signing preflight tests each cert in isolation (no false pass) ([#98](https://github.com/OlsonSoftware/ministr/pull/98))- *(ci)* MacOS .pkg signing + fast signing preflight ([#97](https://github.com/OlsonSoftware/ministr/pull/97))- *(ci)* Pnpm install via cmd on Windows (PowerShell ExecutionPolicy=Restricted) ([#96](https://github.com/OlsonSoftware/ministr/pull/96))- *(ci)* Bootstrap Node in win_setup.ps1 (runner has no npm) ([#95](https://github.com/OlsonSoftware/ministr/pull/95))- *(ci)* Pnpm standalone install (self-hosted Windows has no npm) ([#94](https://github.com/OlsonSoftware/ministr/pull/94))- *(ci)* Bulletproof version echoes — Show-Version helper (no abort) ([#93](https://github.com/OlsonSoftware/ministr/pull/93))- *(release)* Reset main to v0.2.1 — restore manifest==last-tag invariant ([#90](https://github.com/OlsonSoftware/ministr/pull/90))- *(release)* Release-pr stands down while a release is pending ([#89](https://github.com/OlsonSoftware/ministr/pull/89))- *(ci)* Don't merge native stderr into version echoes (NativeCommandError) ([#87](https://github.com/OlsonSoftware/ministr/pull/87))- *(ci)* Python-build-standalone (no MSI) + release-pr open-PR check ([#85](https://github.com/OlsonSoftware/ministr/pull/85))- *(release)* Gate on untagged version, not commit subject ([#84](https://github.com/OlsonSoftware/ministr/pull/84))- *(ci)* Win_setup.ps1 pure ASCII — PowerShell 5.1 mis-decodes em-dash ([#83](https://github.com/OlsonSoftware/ministr/pull/83))- *(release)* Winget-free Windows bootstrap + build-then-tag pipeline ([#80](https://github.com/OlsonSoftware/ministr/pull/80))
## [0.2.1](https://github.com/OlsonSoftware/ministr/releases/tag/v0.2.1) - 2026-05-17

### Added
- *(install)* Unified cross-platform installer experience ([#60](https://github.com/OlsonSoftware/ministr/pull/60))- *(build)* Enhance macOS installer scripts for better error handling and version retrieval- *(parser)* Logos C++ fallback for tree-sitter parse timeouts ([#50](https://github.com/OlsonSoftware/ministr/pull/50))- *(parser)* Logos-driven HLSL/GLSL/MSL/WGSL symbol extraction ([#49](https://github.com/OlsonSoftware/ministr/pull/49))- *(parser)* Index shader source files (HLSL / GLSL / MSL / WGSL) ([#48](https://github.com/OlsonSoftware/ministr/pull/48))- *(ui)* Field Manual aesthetic redesign + EntityPanel + multi-select export ([#46](https://github.com/OlsonSoftware/ministr/pull/46))- *(install)* Centralize install funnel on /install — single source of truth- FFI extractor, subagent session isolation, Tauri UI cleanup ([#38](https://github.com/OlsonSoftware/ministr/pull/38))- *(cli)* `ministr setup` subcommand wraps onpath; consolidate PATH writes ([#33](https://github.com/OlsonSoftware/ministr/pull/33))- *(installer)* Windows desktop NSIS installer + curl-iex install.ps1 ([#32](https://github.com/OlsonSoftware/ministr/pull/32))- *(release)* Serve binaries via dl.ministr.app Worker proxy ([#24](https://github.com/OlsonSoftware/ministr/pull/24))- *(app)* Tray menu routes directly to the Overview / Sessions tabs- *(app)* CorpusChip quick-jump strip on the Projects page- *(app)* Port SessionDashboard to the Overview primitives- *(app)* "cache observatory" UX overhaul — Overview home, Cmd+K, radial vitals- *(app)* Polish ContextSimulator, CorpusTreemap, SymbolGraph- *(app)* Polish Onboarding, ProjectDetail, Settings, LogViewer, Sessions, Ingestion, Search- *(docs)* Restyle D2 diagrams to match iris design language- *(app)* Premium UI refresh — iris brand, polished shell, richer states- *(app)* Live-updating tray menu with Recent corpora + Indexing submenus- *(docs)* Animated Chart.js benchmarks- *(docs)* Command palette, shortcuts, reading progress, interactive diagrams- Iris_ask tool, inference engine, and prefetch priority improvements- Prefetch engine overhaul — PriorityCache, adaptive alpha, cache invalidation- Robust backend selection, backend-aware cache keys, quantized model hints- Wire two-stage Matryoshka retrieval end-to-end- Add two-stage Matryoshka rescore in QueryService.survey()- Add rerank_depth config and dual-embed ingestion path- Add two-stage Matryoshka retrieval foundation (schema + embedder types)- Add macOS installer with code signing, notarization, and custom resources- *(iris-app)* Unified installer — bundle CLI as sidecar with first-launch setup- Complete EMBED2, LAUNCH1, TRAY4 phases- Finish WIP phases — EVICT2, SCAFFOLD2, MEASURE1- Enable Metal GPU and Accelerate for Candle embeddings- Add Candle Metal GPU embedding backend- *(parser)* Add heuristic assembly parser for .asm/.s/.S/.inc files- *(parser)* Add assembly language file support (.asm, .s, .S, .inc)- *(gui)* Granular ingestion progress with phase, current file, and dual progress bars- *(mcp)* Lazy tool registration — prune irrelevant tools at startup- *(cli)* Hot-reload .iris.toml on config changes- *(cli)* Iris hooks test — simulate tool calls against installed hooks- *(scaffold)* Language-specific rule templates for Rust, TypeScript, Python, Go, Java- *(scaffold)* Custom rule injection from .iris.toml [agent] section- *(compress2)* Compression quality scoring eval suite- *(compress2)* Structured claim compression strategy- *(compress2)* Pluggable compression strategy trait + auto-tier selection- *(evict2)* Eviction explainability with factor breakdown- *(evict2)* Integrate salience with FSRS memory model- *(evict2)* Salience-aware eviction scoring- *(init)* Project type classification (monorepo/library/cli/web-app/api)- *(scaffold)* Add Windsurf hooks + Continue.dev rules support- *(schema1)* Compress tool descriptions 50%, add schema token metric- *(measure1)* Session token economics metrics- *(scaffold)* Hard-block Bash exploration + VS Code hooks- Multi-agent setup — scaffold configs for Claude Code, Cursor, Copilot CLI- Add 6 dashboard views — sessions, ingestion, search, treemap, symbols, simulator- Fix tray app logging and enhance GUI dashboard- Persist corpus registrations across tray app restarts- Auto-detect daemon and run as proxy when tray app is running- Iris init writes .mcp.json for Claude Code + GitHub Copilot- TRAY3 — platform polish, autostart, notifications, log viewer, CI, docs- TRAY2 — React 19 + shadcn/ui project management dashboard- TRAY1 — dynamic tray menu, add/remove project, auto-detect, tooltip- DAEMON2.4+2.6+2.7 — coherence SSE, bundle export/import, cloud fetch- DAEMON2.3+2.5+2.9 — SSE progress streaming, session persistence, rate limiting- DAEMON2.0–2.2+2.8 — compress, eviction endpoints + proxy delegation- DAEMON1.12+1.13 — daemon integration & stress tests, extract iris-daemon crate- DAEMON1.7–1.11 — daemon lifecycle, resilience & cross-platform IPC- DAEMON1.6 — prefetch engine runs daemon-side, shared across sessions- DAEMON1.4+1.5 — session management API for daemon-owned state- CLOUD0.6 — OAuth 2.1 scoped tokens for cloud index access- CLOUD0.5 — HTTP bundle endpoints for remote iris servers- CLOUD0.4 — versioned index bundles with commit SHA for staleness detection- CLOUD0.3 — [[corpus.cloud]] config for pre-built index bundles- CLOUD0.1+0.2 — iris export/import CLI commands- CLOUD0.0 — index export format with bundle module- EMBED1.3 — embedding model retrieval quality benchmark- A2A1 — Agent-to-Agent protocol support with agent card and task endpoints- MEMORY1 — FSRS-based spaced repetition context management- CONTEXT1 — context engineering optimizations for KV-cache, eviction, and prefetch- TRANSPORT1 — Docker image, Fly.io/Railway deploy, reverse proxy configs- EMBED1 — code-specialized embeddings with 44-model registry and safe model switching- PERF1 — concurrent indexing with producer-consumer embedding pipeline- Complete DX1 — developer experience polish- Complete daemon API + CLI status/search + auto-start daemon- MCP proxy mode (--proxy) delegates to iris daemon- Background indexing in CorpusRegistry with status tracking- Add daemon client and Svelte frontend- Add iris-app Tauri v2 desktop app with daemon API- Add iris-api crate with shared daemon API types- Single-instance-per-repo with automatic stdio↔HTTP proxy- Auto-scaffold .claude/rules/ agent config on first startup- Per-repo .iris.toml configuration with auto-discovery- Wire bridge extraction pipeline into ingestion for cross-language linking- Concurrent refresh pipeline with git subprocess timeouts- Multi-language ingestion pipeline and DRY import extraction- Content-addressable embedding cache with warm-load on session restart- MCP cancellation support with CancellationToken threading through pipelines- Persist clone roots in CLI and add multi-corpus clone integration tests- Namespace section/symbol IDs by root ID and resolve clone source paths- Additive multi-corpus management — root-scoped clone ingestion- Unified response pagination for iris_symbols, iris_references, and response size guard- OAuth 2.1 auth framework and cross-crate dependency reference linking- Multi-root corpus with per-directory metadata and language stats- Cross-package import graph and cross-crate Rust reference resolution- Session federation with multi-agent context sharing- Workspace detection for Cargo, npm, pnpm, Yarn, Turborepo, and Nx- HttpRouteExtractor, bridge test fixtures, and semantic bridge fallback- FFI bridge extractors — NapiExtractor, WasmBindgenExtractor, PyO3Extractor- Tauri bridge edge cases, test fixture, and iris_bridge MCP tool- Tauri bridge extractors — command, event, and registration validation- Cross-language bridge storage, framework detection, confidence scoring, and references integration- Cross-language bridge framework — core types, extractor trait, and linker pipeline- Generic extractor improvements — annotations, nested types, visibility inference- Swift and Kotlin language refinements for symbol extraction- Java, C, and C++ language refinements for symbol extraction- MCP Server Card resource for pre-connection discovery (SEP-1649)- MCP extensions declaration and negotiation for iris capabilities (SEP-1724)- MCP prompts and completions for session insight and navigation- MCP elicitation for interactive budget, compression, and search- Mtime-based fast skip and HNSW cleanup for instant warm starts- Protocol-native MCP Tasks for async tool execution (SEP-1686)- Mark multi-tier compression tasks as complete in roadmap- Advanced eviction and multi-tier compression pipeline- Structured output, output schemas, and tool annotations for all 15 MCP tools- Upgrade rmcp to 1.2.0 targeting MCP 2025-11-25 protocol spec- LLM-assisted abstractive compression via MCP sampling- Async task support for iris_fetch and iris_clone- Resource subscriptions for iris://status with coherence push notifications- Streamable HTTP transport and rmcp 0.16 migration- Multi-language ref extraction for Python, JS/TS, and Go imports- Attention-position-aware eviction scoring for Lost in the Middle bias- Cyclomatic complexity metric and impact analysis for iris_symbols- Retrieval evaluation benchmark with MRR, nDCG, and CI gate- Proactive eviction recommendations in every tool response- Update tool guide and workflow documentation for iris tools; enhance MCP tool priorities and navigation instructions- MCP progress notifications during background ingestion- Call graph and type usage reference extraction- Cross-encoder reranking — Reranker trait, FastReranker, configurable rerank pipeline- Hybrid search — SPLADE sparse embeddings, inverted index, RRF fusion- Operational hardening — ingestion progress, ref insertion, quantized models- Release pipeline polish — x86_64-mac target, release recipe, installer, MCP smoke test- Iris_toc ingestion state reporting and code ingestion integration tests- Add iris index and iris serve subcommands for headless indexing- Smart file discovery, method-level symbols, stale index cleanup, ingestion status- Multi-language tree-sitter support with grammar registry and generic symbol extractor (C7)- Unified code + doc search with symbol embedding in HNSW vector index (C6)- Add iris_symbols, iris_definition, iris_references MCP tools (C5)- Add symbol storage schema with SQLite tables and Storage trait CRUD- Add AST-aware code chunker with ParserKind::Code and multi-resolution sections- Add Symbol extraction and SymbolTable with visibility, doc comments, signatures- Add tree-sitter foundation with AstParser and Rust AST walker- Unified git and web staleness detection in iris_refresh- Unified corpus URL scheme parsing and multi-source ingestion- Add iris_clone MCP tool for git repo cloning and indexing- Add GitFetcher with sparse clone, metadata tracking, and cache reuse- Add web cache staleness detection and iris_refresh MCP tool- Add sitemap.xml parser with parallel page fetching and rate limiting- Add iris_fetch MCP tool for web content fetching and indexing- Add web fetch pipeline with WebFetcher orchestrator, cache, and content ingestion- Add llms.txt fetcher and parser for LLM-friendly site discovery- Add HTML-to-markdown converter with readability content extraction- Add HTTP client foundation with async HttpClient and URL normalization- Add multi-path corpus support with glob patterns- Add survey-triggered prefetch to pre-warm parent sections of claim hits- Add adaptive section merging to reduce fragment count- Add iris_toc tool for corpus table of contents- Wire coherence engine into CLI and verify analytics round-trip- Wire ingestion pipeline and server persistence into CLI- Add cross-compilation CI and installation methods- Add benchmark suite and evaluation corpus with ground-truth annotations- Add cross-session analytics and feed into prefetch engine- Add HTML and PDF parsers with auto-detection by file extension- Add coherence subsystem — file watcher, re-indexing, and session invalidation- Add claim relationship index and iris_related tool- Expose MCP resources — iris://status and iris://corpus/{path}- Add topical and structural prefetch strategies with per-strategy hit rate metrics- Add sequential prefetch engine with LRU cache- Add session persistence to SQLite for crash recovery- Add eviction ranking, iris_budget and iris_compress tools- Add delta updates, fault-based correction, and iris_evicted tool- Add budget_status to tool responses and session deduplication- Add session shadow, window estimator, and budget tracker- Add tracing spans and user-friendly error handling to MCP tool handlers- Implement iris_survey, iris_read, iris_extract MCP tools- Add MCP server handler and CLI entry point with stdio transport- Add multi-resolution embedding, query pipeline, and incremental vector updates- Add VectorIndex trait and HnswIndex implementation with hnsw_rs- Add Embedder trait and FastEmbedder implementation with fastembed- Add ingestion pipeline orchestrator with incremental re-indexing- Add claim extraction, extractive summarization, and token counting- Add document parser trait, markdown parser with comrak, and structural nodes- Add SQLite storage layer with migrations, async wrapper, and integration tests- Add core error types, domain types, tracing, config, and miette integration- Initialize Cargo workspace with iris-core, iris-mcp, iris-cli
### Changed
- *(ci)* Consolidated workflow architecture — zero build-logic redundancy ([#74](https://github.com/OlsonSoftware/ministr/pull/74))- UI transformation ([#59](https://github.com/OlsonSoftware/ministr/pull/59))

* feat(ask): desktop Ask surface + multi-stage RAG pipeline with verification

Replace the MCP-exposed `ask` tool with a human-facing Tauri Ask tab and
rebuild the underlying pipeline as a multi-stage RAG flow tuned for
codebase Q&A. Single-shot retrieval was too coarse — the new pipeline
trades 1 LLM call for 4 in exchange for substantially higher precision,
gated by an always-on three-way verification stage.

Pipeline (each stage anchored to 2026 research):

  1. Cache lookup (verified hash invalidation)
  2. Query analysis — HyDE doc + sub-questions + symbol hints +
     bridge-relevance flag (1 LLM call, JSON)
  3. Multi-strategy retrieval — survey(raw) | survey(HyDE) |
     fuzzy symbol search | bridge query (parallel per sub-question)
  4. Reciprocal Rank Fusion with per-resolution authority weights
     (Haque et al. 2026, domain-grounded tiered retrieval)
  5. LLM rerank, score 0-10, drop floor (1 LLM call, JSON)
  5b. Adaptive re-retrieval when top score < 6.0 — generate alternate
      phrasings and re-survey (Guo et al. 2026)
  6. Context curation + coverage map
  7. Coverage-aware synthesis with `[NO_EVIDENCE: …]` refusal sentinel
     (Pawlik & Deniziak 2026, Applied Sciences)
  8. Verification — three complementary checks always run on fresh
     answers, all feeding a single confidence note:
       a) deterministic numeric/identifier grounding (regex extract +
          verbatim scan against cited sources)
       b) cross-encoder per-sentence entailment using the existing
          loaded reranker as an NLI scorer (Jin et al. 2026, VerifAI
          2026) — no new model load, sigmoid-normalized threshold 0.35
       c) JSON-mode LLM misrepresentation check
  9. Cache

Backend:

- ministr-daemon/src/ask.rs: replace ask::ask with ask_with_progress
  orchestrating all 9 stages; new AskEvent variants for phased UI;
  rrf_merge with authority_weight; analyze_query; multi_retrieve;
  llm_rerank; adaptive_reretrieve; verify_answer; entailment_check;
  check_grounded_numerics; split_sentences; is_factual_sentence
- ministr-daemon/src/inference.rs: infer_json helper with one repair
  retry; CREATE_NO_WINDOW on Windows so spawning `claude -p` from the
  GUI app doesn't pop a cmd window
- ministr-core/src/service/mod.rs: expose QueryService::reranker()
  so the verification stage can borrow the loaded model
- ministr-mcp/src/proxy.rs: remove the agent-facing `ministr_ask`
  tool — synthesis is now a human-facing feature only

Frontend:

- ministr-app/src/components/AskView.tsx (new ~750 LOC): brutalist Q&A
  surface — omnibar, animated phase rail with sub-question chips,
  markdown answer with clickable [N] citation chips, sources panel
  routing through the global EntityPanel, persisted recent-answers
  strip per corpus, inference-health probe
- ministr-app/src-tauri/src/commands.rs: new ask_corpus command using
  tauri::ipc::Channel<AskPhase> for streaming phase events;
  inference_health PATH probe; read_section accessor
- ministr-app: register `ask` tab in App.tsx, CommandPalette,
  usePreferences, lib/shortcuts (g a binding); BrutalAsk icon;
  react-markdown + remark-gfm deps

Tests: 19 new ask unit tests covering RRF consensus + authority
tie-breaking, coverage, citation-marker stripping, precision-token
extraction, grounded-numerics flagging, sentence splitting (decimals,
versions, punctuation), is_factual_sentence hedge skipping, sigmoid,
and entailment-flag behavior on a mock reranker that mirrors the
BLTE 0x18-vs-0x08 contradiction. cargo fmt --check, clippy --pedantic,
and the full workspace test suite all pass.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat: Refactor onboarding and project detail components for improved UI and functionality

- Replace Card component with a div in Onboarding for a more flexible layout.
- Update Done component's button text to "Ask your first question" for clarity.
- Enhance ProjectDetail to include an inline collapsible structure zone with a treemap.
- Modify navigation actions to support new explore mode in ProjectDetail.
- Introduce Zone component for consistent section styling across the app.
- Consolidate logs and simulator into a Diagnostics zone in Settings for better organization.
- Implement corpus-relative path display in various components for improved user experience.
- Update shortcuts to reflect new navigation structure, replacing symbols and bridge with explore.

* refactor: update typography and component styles across the application

- Replaced text sizes with new utility classes for consistency.
- Updated various components (EntityRow, FileView, SectionView, etc.) to use `text-mono-mini` and `text-mono-micro` for improved readability.
- Introduced `MetricTile` component for displaying metrics in a grid format.
- Removed deprecated `Stat` component and replaced its usage with `MetricTile`.
- Enhanced button and toggle styles for better visual feedback.
- Added new `FilterPill` and `Heading` components for better UI structure.
- Updated UI tokens to centralize styling and improve maintainability.
- Adjusted layout and spacing in several components for a cleaner look.

* feat: add Drawer component for slide-up drawer functionality

feat: implement SourcePane component for pinned source management

feat: create StatusBar component for persistent workspace status display

feat: develop WorkspaceShell component for three-pane layout with resizable dividers

feat: introduce useInvestigations hook for managing investigation state and interactions

feat: add investigations storage logic with localStorage for persistence

feat: implement motion constants and utility functions for refined-brutalist animations

* feat(ui): collapse to 3-surface IA (Ask / Projects / Settings)

Wipe the workspace-shell + center-mode + 4-drawer pattern in favor of
the flat surface switcher from the elegant-seeking-graham plan: a
persistent project picker in the top bar, a 3-icon sidebar, and one of
Ask / Projects / Settings as the main pane.

- New chrome/ (Sidebar, TopBar, ProjectPicker)
- New surfaces/ask/ (AskSurface, AskInput, AskAnswer, AskStatus,
  AskCitation, AskEmpty, PinnedAnswers, internals)
- Stub surfaces for Projects, Settings, AiAssistants (M2/M3 fill-in)
- Drop AskView, CommandPalette, ask/{InlineCitation,InvestigationTabs,
  PhaseStrip}
- Prune shortcuts to the three-surface nav set (drop explore/sessions/
  logs nav and the rail toggle)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): M2 — Projects surface + 3-step onboarding + push progress

Backend
- New `indexing_progress_events` Tauri command streams per-corpus
  IndexingProgressEvent over a tauri::Channel. Polls the existing
  IngestionProgress atomics on a 250ms tick, emits on change, and
  layers a rate-based ETA on top. Frontend no longer polls.

Frontend
- `useIndexingProgress` hook subscribes once and exposes
  Record<corpusId, event> reactively.
- ProjectsSurface rebuilt as a master-detail surface: ProjectCard
  on the left with live progress, ProjectDetail on the right,
  scan/add/reindex/remove all inline.
- Onboarding rewritten as a 3-step wizard (Pick → Index → Connect)
  with a step indicator. Step 2 reads from the new progress
  Channel; step 3 is a stub for the M3 MCP wizard.
- New `ConfirmDialog` primitive replaces the parallel
  ModalShell + TypedConfirmModal patterns; type-to-confirm is
  opt-in via `confirmToken` and used for project removal.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): M3 — in-app MCP wizard for Claude Code / Cursor / VS Code / Codex

Backend
- Refactor `write_mcp_configs` in ministr-core into per-client functions
  (`write_mcp_config(McpClientId, root)`) so the wizard can target one
  client at a time and report which file changed. Bulk wrapper kept for
  `ministr init`.
- Add Codex support — user-global `~/.codex/config.toml` with a
  `[mcp_servers.ministr]` section. Text-patched (not parsed) to preserve
  hand-edited adjacent sections.
- New Tauri commands in ministr-app:
  - `mcp_detect_clients(project_root)` returns one `McpClientInfo`
    per supported client with installed / configured / config_path.
  - `mcp_write_config(project_root, client_id)` writes one client's
    config and returns the absolute path.
  - `mcp_test_connection(project_root, client_id)` shells out to
    `claude mcp list` / `codex mcp list` for CLI clients; for Cursor /
    VS Code returns a config-file validation result with a
    `manual_verify_needed: true` flag.

Frontend
- New `useMcpClients` hook that derives a `McpClientState`
  (not_installed → not_configured → configured → connected) per client
  and orchestrates write + test flows.
- `AiAssistantsPanel` rewritten as a real wizard: one row per client
  with state-appropriate actions (Connect / Re-test / Open file).
- Wired into Settings → AI assistants and onboarding step 3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): M4 — Developer panel + drop legacy ProjectList

- New DeveloperPanel hosts Sessions / Logs / Explore / Query Playground
  as sub-tabs inside Settings → Developer. Each sub-tab reuses the
  existing component so behavior parity is preserved; only the route
  by which the user reaches them has changed.
- SettingsSurface drops the M1 placeholder grid in favor of the real
  panel and threads setActiveCorpusId through (Explore + Query
  Playground both need the setter to switch corpus from inside).
- Delete the now-unreferenced ProjectList.tsx — the new
  surfaces/ProjectsSurface owns project management end-to-end.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): slim Cmd+K palette scoped to the 3-surface IA

Replaces the M1 no-op binding for ⌘K with a focused palette: nav
between Ask / Projects / Settings, switch the active project, add a
project. Anything deeper now belongs in Settings → Developer or the
EntityPanel.

Substring-matching on label + keywords; arrow-key navigation; Enter
activates; Esc closes (palette beats shortcut sheet in z-order).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(ui): hoist formatEta + formatRelativeTime into lib/format.ts

ProjectsSurface and Onboarding had duplicate copies. lib/format.ts
already existed (hosting formatTokens), so the new helpers join it
there. Onboarding now uses the bare variant (no "left" suffix) the
fixed-width row needs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(tray): simplify menu to {Open, Add project, Quit}

Pre-IA-collapse the tray hosted recent-corpora + indexing submenus,
plus Sessions / Logs entries. With the three-surface IA those all live
in the main window (top-bar project picker, Projects surface,
Settings → Developer), so the tray serves a narrower role: window
restore, add a project, quit.

The 10s tooltip refresh stays — corpora count, session breakdown, RSS
remain visible on hover. The menu itself is now static, so
rebuild_menu drops out and the refresh loop is tooltip-only.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* ui(settings): jargon pass on the General tab

Apply the plan's glossary to user-visible labels in the legacy Settings
component:

- "DAEMON" zone → "SERVER"; version/embedding-model rows spelled out
- Drop the standalone DIM (model dimension) row — internal indexing
  detail per the glossary, not user-facing
- "corpora" → "projects" in copy: clear-cache modal body, autostart
  hint, reset-preferences modal, toast detail
- "Daemon log" → "Server log"; "ministr daemon" → "ministr server"
- "corpus query" → "project query" in the context-simulator hint

No structural change — Settings.tsx stays monolithic. The 5-tab split
(separate Server / About panels) remains a future refactor.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): split Settings into 5 tabs — General/AI/Server/Developer/About

Decompose the monolithic Settings.tsx into the panel structure the plan
called for:

- GeneralSettings — theme, default surface, density, autostart
- ServerSettings — read-only server vitals + collapsible diagnostics
  (log viewer, context simulator); owns the ministr-settings-scroll
  listener
- AboutPanel — maintenance grid, version footer, danger zone; now uses
  the unified ConfirmDialog instead of the local TypedConfirmModal,
  finishing the single-confirmation-pattern goal
- settings-primitives — shared PrefRow / MetaRow / MaintAction /
  DiagnosticSection / formatUptime

SettingsSurface goes from 3 tabs to 5 (General / AI assistants /
Server / Developer / About). The old `Settings.tsx` is deleted.

Also narrow the DefaultTab preference to the real 3-surface IA
(ask / projects / settings) — the old explore/sessions options
pointed at routes that no longer exist.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(ui): wire the Default tab preference to the launch surface

The preference dropdown existed but AppInner hard-coded the initial
surface to "ask", so the setting was dead UI. Initialize `surface`
from useDefaultTab() instead. The no-projects → Projects bounce still
overrides on a cold install.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* chore(ui): delete workspace shell + investigations dead code

The IA collapse (M1) stopped importing the old workspace shell, and
M1 also dropped InvestigationTabs. These had zero remaining importers:

- components/workspace/{WorkspaceShell,CorpusRail,SourcePane,
  StatusBar,Drawer}.tsx
- hooks/useInvestigations.ts
- lib/investigations.ts (only useInvestigations consumed it)

The plan called for deleting these at M4. tsc + vite build stay green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* style: rustfmt the M3 MCP code

cargo fmt --check flagged the write_codex_mcp text-patch block and the
mcp_* command helpers added in the M3 commit. Pure formatting — no
behavior change. `just validate` (fmt-check + lint + test) is now green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(budget): env-driven context window, drop misleading hardcoded 100k

The budget tracker pinned max_context_tokens at 100_000, so pressure
fired on agents that actually had 2–10x the room (200k Claude, 1M
Opus). MCP gives the server no channel to learn the connected model's
real window, so the source of truth is now MINISTR_CONTEXT_WINDOW —
set in the env block of the MCP client config (.mcp.json /
.vscode/mcp.json / .cursor/mcp.json / ~/.codex/config.toml).

- Add default_max_context_tokens(): parse MINISTR_CONTEXT_WINDOW,
  else FALLBACK_CONTEXT_TOKENS (200k — the current Claude floor, so
  it never under-reports the way 100k did).
- BudgetConfig::default() and ProxyServer::new() both use it instead
  of a hardcoded literal.
- Tests updated to the new contract + a parser-contract test for the
  env override.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(mcp): stop surfacing budget pressure to agents; keep internal tracking

Agents were treating ministr's per-response budget numbers as a signal
that they were almost out of context and abandoning work — even though
the figures were anchored to a configured window, not the model's real
context window. Budget pressure is still tracked internally (BudgetTracker
keeps recording, so dedup + optional compression still work, and the
explicit ministr_budget tool still reports real numbers on request), but
it is no longer pushed at the agent unsolicited.

- ToolResponse.budget_status / eviction_recommendations: kept on the
  struct (constructors unchanged) but #[serde(skip_serializing)] +
  #[schemars(skip)] so they never reach the model.
- build_response_with no longer computes eviction candidates.
- build_next_actions drops the pressure-driven compress/evict entries;
  coherence re-reads and per-handler hints remain.
- DEFAULT_INSTRUCTIONS: removed the "Budget protocol" section and the
  "react to eviction_recommendations" guidance; added an explicit note
  that ministr does NOT report context-budget pressure and must not be
  treated as a low-context signal.
- ministr_budget de-advertised in build_instructions and reworded in
  both server and proxy tool descriptions ("advisory only, safe to
  ignore"); the tool stays callable for deliberate use.
- Tests updated to the new contract: a shared assert_no_budget_hints
  guard, accumulation/eviction now verified via the explicit
  ministr_budget tool, and regression tests that a saturated budget
  still leaks nothing.

cargo fmt-check + workspace clippy pedantic clean; ministr-mcp lib
(193), e2e_mcp (60), ministr-core session (289) all green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* checkpoint(ui): in-flight session redesign + live-data primitives

Snapshot the mid-flight ui-transformation work before the AAA
reinvention so there is a clean restore point and the reinvention
diff is reviewable in isolation:

- entity/session/* deep-dive drawer (SessionView/Hero/Economics/
  Budget/ActivityTimeline/Lineage)
- shared live session store + history + activity hooks
- ProjectSessions surface; budget-bar/sparkline/token-economics-bar
  ui primitives; lib/sessions.ts; status/types updates

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): Cockpit design-system foundation

Replace neo-brutalist tokens with the dark-first "Cockpit" language:
elevation tiers, soft radii/shadows, accent glow, fluid type, motion
tokens. Add `motion` lib + MotionProvider (reduced-motion aware).
Rebuild core ui primitives (button/card/badge/status-dot/progress/
metric-tile/empty-state), add smoothed sparkline mode + NumberTicker.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): Cockpit shell re-architecture

- Promote Sessions to a first-class surface (live motion board);
  4-surface nav rail with spring shared-layout active pill
- Nav history (back/forward, Cmd+[ / Cmd+]) + animated surface swaps
- Command palette with mode prefixes (> @ # ?), fuzzy, spring pop
- EntityPanel: spring slide-over, resizable, animated breadcrumb/body
- TopBar live vitals tickers + Cmd+K entry; restyle ProjectPicker/
  DaemonDot/ToastTray to the Cockpit language; wire shortcuts/prefs

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): live-data layer (liveBus + lifecycle toasts)

Derive typed session lifecycle events (started/ended/turn-advanced/
pressure-critical) from the shared store without extra polling; toast
connect/end/critical moments. Add reduced-motion-safe .ministr-flash
compat alias so legacy callers keep an effect until the sweep.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): surface redesigns — entity drawer, Ask, session hero

Rebuild the shared entity primitives (EntitySection/EntityRow) on the
Cockpit language with spring mount + chevron motion (propagates to all
EntityPanel views). Choreograph AskStatus, restyle AskInput. Restyle
the flagship SessionHero (rounded, soft, entrance motion).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): consistency sweep — high-traffic surfaces to Cockpit

Smooth interactive transitions + soft/rounded treatments on the
user-facing surfaces (AskAnswer sources/pins, ActivityTimeline,
ProjectsSurface hover, ProjectSessions live pill/heartbeat).

Lower-traffic developer-tools views (QueryPlayground, Bridge,
SymbolGraph, LogViewer) remain token-coherent via the new design
system and are flagged for a later hand-polish pass.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(ui): consistency contract + primitive sweep (Cockpit p2 A/B)

Phase A: add canonical role tokens (transitionInteractive, focusRing,
dividerRow/Section, chip/chipActive) to lib/ui-tokens.ts; add DESIGN.md
contract. Phase B: rebuild remaining ui/* primitives on the contract —
zone, confirm-dialog (shared popIn/scrim motion), filter-pill (chip
tokens), turn-block, toggle, corpus-chip/select, activity/coherence
feeds, budget-bar (restore eased fill — .motion-data was removed),
token-economics-bar, labeled-row, vital-card.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(ui): codemod sweep to the Cockpit contract (p2 C)

Scripted codemod across 34 files (~325 literals): tracking-[0.05em]
->0.08em, transition-none->transitionInteractive, font-serif->sans,
border-2->hairline, rounded-sm->rounded-md, ministr-flash->pulse.
Hand-fix AskCitation dead .motion-data/.ministr-pin-in (-> motion
popIn + .ministr-skeleton); retire the .ministr-flash compat alias.
Remaining banned-literal count: 0. tsc + build green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ui): consistency guardrail + final verify (p2 D/E)

Add scripts/design-lint.cjs (comment-aware banned-literal gate),
pnpm design:lint, and a just design-lint recipe wired into
just validate. Fixes the two arbitrary shadows it caught
(SessionHero danger ring + QueryPlayground selected row → ring-*).
tsc + build + design-lint all green; zero contract violations.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): register 16 new tree-sitter grammars

Add bash, php, scala, lua, elixir, haskell, ocaml (impl+interface),
dart, r, hcl/terraform, json, yaml, toml, sql, zig, protobuf — all
default-on via lang-all, ABI-compatible with tree-sitter 0.26 through
the tree-sitter-language 0.1 shim. These extensions were already routed
to the code parser but fell back to text-only chunking; they now get
real ASTs. Dockerfile deferred (only crate pins legacy ts 0.20).
Repoint the grammarless-fallback test off .zig (now supported) to .asm.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): Protobuf symbol refinement

Add ProtoRefinement so .proto message/enum/service surface as
Struct/Enum/Trait symbols (names live in *_name children; bare
message/service kinds aren't caught by the generic heuristic).
High-value for gRPC/API discovery. Deep HCL/SQL symbol refinement
deferred — those need walker-depth changes; they still index at
text + generic level (strictly better than the pre-grammar state).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): PHP/Kotlin/Scala import xref extractors

extract_refs now dispatches php (namespace_use_clause), kotlin
(import qualified_identifier), scala (import_declaration + brace
selectors) so ministr_references resolves imports for these
Phase-1 languages. Node shapes empirically verified. Elixir/Lua/
proto import refs deferred (lower value; symbols already work).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): cgo cross-language bridge (Go <-> C)

New BridgeKind::Cgo + CgoExtractor: C top-level function_definition
=> Export(c); Go `C.func(...)` selector => Import(go). Linker pairs
them so only C functions actually called from Go surface. Detector
adds Cgo when a go.mod and C sources coexist. Full enum/parse/linker
plumbing + extractor/detector tests (174 bridge tests green).

UniFFI / gRPC-protobuf / JNI deferred (documented) — each needs
non-trivial UDL/generated-stub heuristics; scoped out to land cgo
correctly rather than four kinds shakily.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* docs(code): record language/bridge coverage expansion + clippy fix

Changelog [Unreleased] + readme: ~28 tree-sitter languages, Protobuf
refinement, PHP/Kotlin/Scala xrefs, cgo bridge (8th kind). Fix a
needless-borrow clippy::pedantic lint in cgo.rs. Workspace builds;
ministr-core tests + doctests + clippy pedantic all green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): Svelte SFC grammar; Dockerfile/Vue/Astro investigated

Add tree-sitter-svelte-ng (modern -ng bindings) for .svelte
single-file components, default-on via lang-all. Route .vue/.astro
extensions to the code parser for text-level indexing.

Dockerfile, Vue, Astro definitively cannot get an AST: tree-sitter-
dockerfile (crates.io + upstream main) and tree-sitter-vue 0.0.3 both
declare links="tree-sitter" against legacy 0.20, which hard-conflicts
with our 0.26 pin; tree-sitter-astro is unpublished. Documented in
Cargo.toml; these keep the (lossless) text fallback.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(code): activate the LanguageRefinement system + HCL/SQL refinements

refinement_for() was defined but never called anywhere — the entire
code::lang refinement registry (incl. the Phase-2 Proto refinement)
was dead code; all non-Rust symbols came solely from the generic
heuristic. Wire it in: generic_extract_symbols_for(language) consults
the per-language LanguageRefinement (classify + name) as an override,
threaded through extract_from_node/extract_nested_members; ingestion
passes the detected language. Add `statement`/`config_file`/`body`
wrapper unwrapping so refinement-driven languages reach their nested
declarations.

New HclRefinement (Terraform blocks -> resource.aws_s3_bucket.web,
variable.region, module.vpc) and SqlRefinement (CREATE TABLE/VIEW/
FUNCTION/...). Proto refinement now actually takes effect. 410 code
tests green — activating refinements for go/java/kotlin/etc. caused
no regressions (they're supersets of the generic classifier).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): JNI, UniFFI, and gRPC cross-language bridges

Completes the deferred bridge set (cgo landed earlier):

- JNI: Java `native` / Kotlin `external` (Import) <-> C/C++
  Java_pkg_Class_method JNIEXPORT (Export), keyed by method name.
- UniFFI: Rust #[uniffi::export]/derive(uniffi::*) (Export) <->
  Swift/Kotlin/Python imports of the generated module (Import).
- gRPC: .proto `service` (Export) <-> generated FooClient/FooStub/
  NewFooClient/... references in Go/Python/TS/Java/Kotlin (Import),
  keyed by the recovered service name.

Full enum/parse/linker plumbing + detector signals (Cargo uniffi/
tonic/prost/grpcio/jni, npm @grpc/*, pyproject grpcio, and a .proto
filesystem signal). 184 bridge tests green (extractor + detector +
full-link integration tests for each kind). ministr_bridge now spans
11 kinds.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(init): PHP/Ruby project rules; docs + final validation

init.rs Language gains Php (composer.json) and Ruby (Gemfile/
*.gemspec) with concise best-practice rule templates emitted by
`ministr init`. Update changelog/readme for the full coverage set
(~29 languages, 11 bridge kinds, refinement-system fix, Svelte).
Workspace builds; ministr-core tests + doctests + clippy pedantic
all green; fixed an unused-import lint in sql.rs tests.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): 11 new tree-sitter grammars (CSS/GraphQL/Groovy/Nix/Erlang/PowerShell/Solidity/ObjC/Julia/CMake/Make)

All use the tree-sitter-language 0.1 ABI shim and link cleanly against
tree-sitter 0.26. Clojure dropped to documented text-fallback (its
crates.io latest hard-pins tree-sitter 0.25 via links). Markdown/HTML
deliberately not added — dedicated prose/markup parsers take precedence
in detect_parser_kind. New extensions wired into ALL_CODE_EXTENSIONS;
CMakeLists.txt/GNUmakefile added to filename detection.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): 21 language refinements for symbol extraction

Adds LanguageRefinement impls (classify_node_kind, delegate-on-unknown
so never a regression) for ruby, php, scala, csharp, javascript, bash,
lua, haskell, ocaml(+interface), dart, r, zig — previously generic-
heuristic only — plus the structure-heavy new grammars css, graphql,
groovy, solidity, erlang, julia, cmake, make. Wired into refinement_for.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): cross-reference extractors for Java, C#, Swift, Ruby

ministr_references now resolves imports for four more languages whose
grammars were already routed. Java/C#/Swift use JVM-style dotted-import
last-segment extraction (mirrors KotlinImports); Ruby walks require/
require_relative/load/autoload calls to the required file stem.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(init): project rules for C#/Kotlin/Swift/Scala/C++/Elixir/JS

Extends the Language enum + detect_project manifest signals (*.csproj/
*.sln, *.gradle.kts, Package.swift, build.sbt, CMakeLists.txt, mix.exs,
package.json-without-tsconfig) and adds best-practice rule blocks for
each. ministr init now emits language guidance for 13 languages.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(code): Flutter platform-channel + Electron IPC bridges

Adds BridgeKind::FlutterChannel and ElectronIpc (13 kinds total).
flutter.rs links Dart MethodChannel/EventChannel/BasicMessageChannel
string keys to native (Kotlin/Java/Swift/ObjC) registrations; electron.rs
links ipcMain.handle/on to ipcRenderer.invoke/send/on by channel name.
FrameworkDetector gains pubspec.yaml (Flutter) and electron-in-
package.json signals; both wired into create_linker_for_kinds.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* docs+chore: changelog for maximal coverage expansion; bump 0.2.2→0.2.3

Documents the +11 grammars, +21 refinements, +4 cross-ref languages,
+7 init languages, and Flutter/Electron bridge kinds in the Unreleased
changelog. fmt + clippy-pedantic clean across the workspace; all
crates bumped in lockstep to 0.2.3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(ingestion): overhaul global ignore-dir/file autodetection

Sourced from the canonical github/gitignore templates across C++/CMake/
Bazel/Node/Python/JVM/Go/Swift/Xcode/.NET/Elixir/Dart/Unity/UE.

- ALWAYS_IGNORE_DIRS: +vendored-dep trees (3rdparty/third_party/extern/
  deps/_deps/bower_components/...), +per-ecosystem cache/build dirs
  (.dart_tool/.svelte-kit/.turbo/CMakeFiles/Pods/DerivedData/.elixir_ls/
  .eggs/...). Ambiguous names (bin/obj/Library/Debug/Release) left OUT
  on purpose — those get project-type-gated in init instead.
- New ALWAYS_IGNORE_DIR_GLOBS (bazel-*, cmake-build-*, *.egg-info,
  *.xcodeproj/.xcworkspace/.framework) with a tiny prefix/suffix matcher.
- ALWAYS_IGNORE_PATTERNS: +generated bindings (*.pb.go, *_pb2.py,
  *_pb2_grpc.py, *.pb.cc/.h, *.g.dart, *.Designer.cs, moc_*.cpp, ...).
- Extracted the duplicated walker setup in discover_files +
  compute_corpus_stat_merkle into one ignored_walk() so the indexed set
  and the change-detection fingerprint can never drift.

This is the WebWowViewerCpp class of problem fixed at the source: a
vendored-dep C++ tree no longer drowns the real engine code.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(init): polyglot project + source-dir detection overhaul

- default_ignore_patterns now keyed off the full ProjectDetection and
  emits project-type-gated build-output dir ignores (.NET bin/obj,
  Unity Library/Temp/Obj/Logs, Unreal Binaries/Intermediate/Saved/
  DerivedDataCache, SwiftPM .build) — scoped via .ministr.toml so the
  generic names stay safe.
- detect_source_paths is polyglot/additive: conventional roots for Go,
  JVM, C/C++, C#, Swift, Elixir, PHP, Ruby, Dart in addition to the
  rust/node/python paths, never removing the '.' safety net.
- Informal polyglot monorepos (>=2 ecosystems, no workspace file) now
  classify as Monorepo via ecosystem_count().
- detect_project reordered to build the detection first, then derive
  source/ignore paths from the full picture.
- just validate green (fmt + clippy-pedantic + tests + doctests).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(robustness): unify corpus identity + awaited teardown/verifiable deletes

Phase 1 - corpus identity unification (data-loss fix):
- New ministr_core::corpus_id: single source of truth for canonical
  corpus paths + id. The CLI (infra::corpus_data_dir_name) and daemon
  (registry) derived DIFFERENT on-disk dir names for the same project,
  silently splitting a corpus across two directories. Both now call the
  shared module. Hardened: strips Windows verbatim prefixes, rejects
  empty/whitespace paths (CorpusIdError), byte-identical ids for normal
  paths so existing daemon corpora keep resolving.
- RegistryError::InvalidPath; register/restore/update_corpus_paths
  propagate.
- CLI gains best-effort legacy-corpus-dir migration so existing CLI
  indexes are not orphaned by the scheme change.

Phase 2 - awaited teardown + verifiable destructive ops:
- ministr_core::fs_util::remove_dir_all_robust: in-house bounded
  exponential backoff for the Windows handle-close / sharing-violation
  race; NotFound = success; symlink-safe via std.
- CorpusHandle.tasks tracks every spawned background task; unregister
  now cancels AND awaits them (5s bounded) before returning, so SQLite
  and watcher handles are closed before the dir is deleted.
- save_manifest returns Result; unregister/update_corpus_paths
  propagate, register logs-and-continues.
- Tauri remove_project/remove_project_by_id surface deletion failure
  instead of logging false success; trigger_reindex propagates
  unregister and truly purges data_dir before re-register.

Verified: build + clippy pedantic (-D warnings, all targets) + tests +
rustfmt clean across ministr-core/daemon/cli/app.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(init): overhaul agent-config scaffolding — steer, not overzealous wall

The generated PreToolUse hooks were the root cause of two real problems
and `ministr init` would auto-heal any manual fix straight back to the
broken template:

1. Crash: deny reasons were embedded in single-quoted `printf '...'`.
   A reason containing an apostrophe ("Don't pipe...") closed the shell
   quote and crashed the hook (unexpected EOF) instead of returning a
   decision — it broke unrelated commands like `git commit`.
2. Overzealous: `Bash(*|*grep *)` etc. blocked legitimate pipelines
   (`cargo test | grep`, `git log | tail`).

Changes (Claude Code path — the enforced/auto-healed one):
- New STEER_SCRIPT template written to `.claude/hooks/steer-to-ministr.sh`
  (auto-healed). All decisions emitted from shell variables, ASCII-only,
  never inside a quoted literal — the apostrophe-crash class is now
  structurally impossible. Reasons are precise redirects naming the
  exact ministr tool (the documented steering mechanism).
- build_claude_hooks/steer_hook delegate to that script: Grep/Glob
  tools -> deny (frictionless); leading shell grep/find -> ask (a
  speed-bump, approve when filtering output / fs op); NO pipe rules,
  leading-anchored so pipelines are never intercepted.

Same sentiment applied across every generator and advisory blob:
- Copilot hooks (bash + powershell): tools deny, leading search ask,
  pipe-scan branch removed.
- Cursor hooks: ask (not deny), pipe branch removed.
- Windsurf hooks: advisory hint then ALLOW (exit 0) — Windsurf has no
  "ask" and must not hard-block; pipe branch + exit 2 removed.
- MINISTR_SCOPE, TOOLS, WINDSURF_RULES/CONTINUE_RULES, CURSOR_RULES,
  COPILOT_INSTRUCTIONS, AGENTS_MD: absolutist "MUST/NEVER/PROHIBITED/
  BLOCKED/ANY piped" language replaced with the accurate policy
  (prefer ministr for exploration; pipelines/git/build-test output and
  `find -delete` run unrestricted).

Tests updated for the new design (26 scaffold tests green); full
ministr-core suite green (1440). clippy pedantic + rustfmt clean.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(app): repair_agent_config autofix + steering never prompts

Two changes that finish the agent-config hardening arc.

1. Steering no longer prompts the user. An autonomous agent hitting an
   `ask` decision turns into an endless yes/no. The steering hooks now:
   - Grep/Glob TOOLS -> `deny` (silent redirect to ministr; no prompt).
   - leading shell grep/find/rg/etc -> `allow` + a one-line advisory
     hint (auto-approved; never prompts).
   - pipelines/compound commands -> still never intercepted.
   Applied across the Claude steer script, Copilot (bash+pwsh), and
   Cursor generators; advisory docs reworded accordingly. Tests assert
   no `ask` decision is ever emitted.

2. New Tauri command `repair_agent_config`: idempotently re-scaffolds
   every AI-assistant config file for all registered corpus roots via
   ministr_core::scaffold::scaffold_agent_config. Non-destructive —
   advisory .md files created only if missing, machine hook files healed
   only on drift, .claude/settings.json merged (unrelated keys
   preserved), nested sub-roots de-duplicated. Registered in
   generate_handler!. Gives users a one-click fix instead of relying on
   silent startup autoheal.

26 scaffold tests green; clippy pedantic + rustfmt clean (ministr-core,
ministr-app).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(app-ui): Agent config repair card in AI assistants panel

Surfaces the `repair_agent_config` command in Settings → AI assistants
(AiAssistantsPanel), native to the existing design system.

- New AgentConfigCard: idle → running (disabled, spinner, aria-busy,
  "Repairing…") → result, with the result in an aria-live=polite region.
- Idempotent no-op is explicitly confirmed ("Already up to date — N
  projects checked") so a no-change run never reads as a failure;
  otherwise shows "Repaired N projects · X created · Y healed" plus the
  list of roots.
- Errors render in the existing danger box and the button becomes
  "Retry". Reuses Button/tokens/lucide/formatTestStamp — design:lint
  clean (Cockpit contract).
- RepairReport type added to lib/types.ts.

Verified: tsc --noEmit clean, design:lint clean, vite build OK.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(config): update corpus paths to include only the root directory

* fix(app): run `claude mcp list` test in the project root

`mcp_test_connection` shelled out to `claude mcp list` (and `codex mcp
list`) via `test_via_cli`, which never set a working directory — so the
command ran in the Tauri app's cwd. Claude Code's `.mcp.json` is
PROJECT-SCOPED: `claude mcp list` only enumerates it when invoked from
that project directory. Result: a freshly-written, correct config was
reported as "ran but didn't list ministr" (false negative).

- test_via_cli now takes the project root and sets `.current_dir(root)`.
- Not-listed message is now actionable: explains the config is
  project-scoped and that Claude Code needs the ministr server approved
  on first use in the project, then a re-test.

clippy pedantic + rustfmt clean (ministr-app).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(app): resolve and spawn CLI clients correctly from the GUI (Windows)

Why "Test connection" failed from the Tauri app even though `claude mcp
list` works in a shell: three Windows-specific defects in test_via_cli /
which_on_path:

1. which_on_path only tried the literal name (and test_via_cli only
   `name` / `name.exe`). Claude Code / Codex installed via npm are
   `claude.cmd` / `codex.cmd` shims — never matched, so the test
   reported "not found on PATH" or fell through.
2. test_via_cli then spawned the BARE name (`Command::new("claude")`),
   ignoring the resolved path. A GUI process's CreateProcess does not
   search PATHEXT, so this finds nothing.
3. Even resolved, a `.cmd`/`.bat` cannot be spawned directly on Windows
   (Rust >=1.77 hard-errors; it must go through `cmd /c`).

Fixes:
- which_on_path now walks PATHEXT for extensionless names and returns
  the resolved absolute path with its real extension (also improves
  client *detection*, not just the test).
- test_via_cli builds the command from that resolved path, routing
  `.cmd`/`.bat` through `cmd /c`, sets CREATE_NO_WINDOW (GUI binary has
  no console), and keeps `current_dir = project root` so project-scoped
  `.mcp.json` is seen.
- "not found" message now explains the GUI-PATH-vs-shell-PATH gap.

clippy pedantic + rustfmt clean (ministr-app).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(daemon): prune dead corpus manifest entries on restore

`restore()`'s doc claimed it "skips entries whose source paths no longer
exist on disk", but the loop registered every manifest entry
unconditionally. So a stale entry like `/tmp/ministr-e2e-test` (a path
left by an e2e run, nonexistent on this host) was re-registered on every
daemon start — its corpus dir recreated and the project reappearing in
the UI after every `just reinstall`.

- New `entry_is_live`: an entry is dead iff every path is a local path
  that no longer exists (remote http/git paths keep it alive; empty set
  is dead).
- restore() now partitions entries: only live ones are registered.
- Self-heal: dead entries' corpus dirs are best-effort removed
  (remove_dir_all_robust) and, when nothing live remains to trigger a
  save via register(), the pruned manifest is persisted directly — so
  the stale project stops coming back.

Note (out of scope here): the underlying test-hygiene bug — an e2e test
writing into the real ~/.ministr manifest instead of a tempdir — should
be fixed separately; this change makes the daemon self-correct
regardless.

clippy pedantic + rustfmt clean; ministr-daemon tests green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(scaffold): forward-only autoheal + create-only on `serve`

Answers "why does autoheal revert to a bad state, and why does it keep
coming back every session": autoheal converged to *whatever template the
running binary has*. A stale `ministr` (an old build shadowing PATH)
therefore "healed" hooks back to its own old/bad template on every
`ministr serve` (every new Claude session).

Two structural guards so this cannot happen regardless of which binary
runs:

1. ScaffoldMode::CreateOnly vs Heal. `ministr serve` now scaffolds
   CreateOnly: it creates missing files but NEVER rewrites an existing
   `.claude/settings.json` hooks block. Healing is reserved for the
   explicit, user-initiated paths (`ministr init`, desktop Repair),
   which are run with a known-current binary on purpose.
2. Forward-only autoheal. settings.json carries a monotonic
   `_ministrHooksVersion`. Even under Heal, a binary refuses to
   overwrite hooks stamped at an equal-or-newer version — a stale
   binary can no longer downgrade good hooks. Autoheal converges to the
   newest state, not "this binary's state".

write_claude_hooks now also merges non-destructively (preserves user
keys) and stamps the version. 26 scaffold tests green; clippy pedantic
+ rustfmt clean (ministr-core, ministr-cli).

Note: the underlying "stale binary on PATH" cause (a legacy
%LOCALAPPDATA%\ministr bundle install shadowing the dev install) is
addressed separately by consolidating the install locations.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(setup): make `ministr setup` the single install source of truth

Root cause of "just reinstall still runs a stale binary / hooks revert
every session": there were two divergent install locations. The dev
scripts PATH-added ~/.ministr/bin; an older packaged installer left
%LOCALAPPDATA%\ministr (bin + loose exes) which was FIRST on PATH and
nothing ever removed it, so an old build permanently shadowed the new
one and every `ministr serve` ran stale.

`cmd_setup` is now the one routine every channel funnels through
(dev `just reinstall`, the app's first-launch setup.rs, the NSIS
installer hooks — all already call it):

- Canonical location is always <daemon_data_dir>/bin (~/.ministr/bin),
  independent of where the running binary lives. The legacy `--bin-dir`
  arg is accepted but no longer changes the target — every entry point
  converges here.
- The running binary is staged into the canonical dir (so the packaged
  app / NSIS, whose ministr lives elsewhere, still puts the *current*
  binary on the canonical PATH).
- Known legacy/duplicate ministr roots (%LOCALAPPDATA%\ministr[\bin],
  ~/.cargo\bin) are de-PATHed and their shadowing CLI binaries
  refreshed with the current build. Refresh handles the Windows
  running-exe lock via rename-aside-then-copy, with orphan sweep.

Verified live: user PATH now contains only ~/.ministr\bin; the
%LOCALAPPDATA% shadow is de-PATHed and its ministr.exe refreshed to the
current build (old one moved to .exe.stale, auto-swept).

clippy pedantic + rustfmt clean; ministr-cli tests green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* chore(reinstall): funnel dev scripts through the consolidated `ministr setup`

Follow-up to the install SSOT consolidation. The dev reinstall recipes
duplicated logic that `ministr setup` now owns:

- scripts/reinstall.ps1: dropped the manual `~/.cargo\bin\ministr.exe`
  deletion (setup de-PATHs + refreshes every legacy/duplicate shadow,
  including %LOCALAPPDATA%\ministr) and the now-ignored `--bin-dir`
  argument — the call is just `ministr setup`.
- justfile [unix] reinstall: dropped the now-ignored `--bin-dir`
  argument for cross-platform parity. The unix-specific `rm` of
  ~/.cargo/bin and /usr/local/bin is kept (defensive; setup's legacy
  purge is currently Windows-focused).

No behaviour change beyond removing redundancy — `ministr setup` is the
single source of truth for install location + PATH + legacy cleanup.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(daemon): drop corpora-map guard before per-corpus info await

Phase 3 (lock discipline) — the indexer-hot path.

set_status / update_stats / update_symbols_count held the corpora-map
read guard across `handle.info.write().await`. The indexer calls these
repeatedly during every index run, while register / unregister /
restore may be taking the map write lock — serialising those writers
behind every per-corpus info write and risking lock-order inversion.

`info` is an `Arc<RwLock<CorpusInfo>>` exactly so it can be cloned out.
New `info_handle()` clones the Arc under the map read guard, drops the
guard, and only then the caller awaits `info.write()`. No public
signature change, no cross-crate ripple.

Remaining Phase 3 items (larger blast radius, separate pass):
`CorpusRegistry::get` returning a live `RwLockReadGuard`, `list()` /
sessions needing an `Arc<Mutex>` to drop the map guard, and the
`register` check-then-insert race.

clippy pedantic + rustfmt clean; ministr-daemon tests green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(daemon): atomic check-and-insert in register (close TOCTOU race)

Two concurrent register()s of the same corpus_id could both pass the
read-lock contains_key check and the second insert would overwrite —
orphaning the first handle's resources. The check is now performed
under the same write lock as the insert; the race loser discards its
freshly-created handle (no tasks spawned yet) and returns the
idempotent (id, false).

clippy pedantic + rustfmt clean; ministr-daemon tests green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(daemon): list() snapshots Arcs and drops the map guard

CorpusHandle.sessions is now Arc<Mutex<SessionRegistry>> (accessors
unchanged — Arc derefs for .lock()/.try_lock()). list() now clones each
corpus's info/progress/index/sessions Arcs under the corpora-map read
guard, DROPS the guard, then awaits per-corpus info.read()/sessions
.lock(). A concurrent register/unregister (map write lock) is no longer
serialised behind N per-corpus awaits, and the lock-order-inversion
risk is gone.

clippy pedantic + rustfmt clean; ministr-daemon + ministr-app green
(session_invalidation, prefetch_invalidation pass).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(daemon): get() returns Arc<CorpusHandle>, not a map guard

Final Phase 3 item. The corpora map is now HashMap<String,
Arc<CorpusHandle>>; CorpusRegistry::get() clones the Arc out and drops
the read guard, so callers hold only a detached handle — never a
RwLockReadGuard across their .await. This removes the last place a
request could serialise register/unregister or invert lock order.

- get() -> Result<Arc<CorpusHandle>, RegistryError> (was Result<
  RwLockReadGuard<HashMap<..>>>).
- get_corpus! macro yields the handle Arc directly; all 18 HTTP
  handlers updated (guard[&id] -> handle; redundant &guard[&id]
  shadows removed; early drops now release the detached Arc).
- tick_session_turn collapsed to a single get().await.
- register inserts Arc::new(handle); corpora() accessor + 3 test
  helpers updated. App commands / indexer / registry-internal access
  is Arc-deref-compatible (unchanged).

Resource lifetime is now correct-by-construction: a corpus's
SQLite/index live until the last Arc (unregister's, plus any in-flight
request) drops — no use-after-free, no guard contention.

clippy pedantic + rustfmt clean; ministr-daemon (incl. daemon_roundtrip
22, daemon_stress 4, session/prefetch_invalidation) + ministr-app green.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(index): atomic HNSW persist with crash-recovery backup

`HnswIndex::persist` was non-atomic: it deleted stale dumps, then
`file_dump`, then wrote the id-map sidecar separately with no fsync.
A crash between any step left a corrupt/partial on-disk index.

Now stage the full index (dump + sidecar) into a fresh sibling temp
dir, fsync every file and the dir, then swap atomically via a rename
dance (dir -> dir.bak, tmp -> dir, rm dir.bak) with rollback on
failure. `load` falls back to `<dir>.bak` when the primary dir is
missing or has no id-map, recovering the prior consistent index after
an interrupted swap.

New sync fs_util helpers: fsync_file (write-handle so Windows
FlushFileBuffers works), fsync_dir (no-op on Windows), rename_robust,
remove_dir_all_robust_sync. Crash-injection + no-leftover tests added.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(index): typed dim/model desync guard on index load

`load()` previously returned a usable index even when its stored
vectors were built with a different embedding model or dimension than
the active embedder. The daemon never validated this at all (it only
passed `dim` and ignored the stored model), so a model change would
silently produce a wrong-space index and fail later at query time.

Add `IndexError::ModelMismatch` and `HnswIndex::check_compatible`
(dimension always strict; model strict when one was stored; a legacy
no-model index is adopted). Both load sites now validate via the typed
check and rebuild on mismatch:
- CLI infra: replaces the ad-hoc dim/model comparison.
- daemon registry: now threads `config.default_model` through
  `load_or_create_index`, stamps the model name on fresh and adopted
  indexes (previously daemon indexes were always model-less), and uses
  the Windows-robust retrying remove for the discard path.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(daemon): index/content desync integrity probe at register

After a corpus is registered, compare the persisted SQLite section
count against the loaded vector index size. When one side is empty
while the other is not (lost/failed-to-load dump, or an orphaned
index), emit a structured warning with an actionable repair path
(`ministr reindex` / app Re-index button). Never auto-deletes — the
background indexer reconciles real content drift; this surfaces the
stale-merkle-short-circuit case that would otherwise leave search
silently degraded.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(core): remove panicking .expect in AstParser::new

`AstParser::new()` used `.expect("failed to load tree-sitter Rust
grammar")` — a panic in library code (violates CLAUDE.md). Add
`AstParser::try_new() -> Result<Self, ParseError>` and make `new()`
delegate, degrading to a language-less parser (parse() then yields
ParseError::Failed) with an error log instead of panicking. The three
production call sites (parser/code.rs, ingestion/symbols.rs,
ingestion/pipeline.rs) now use `try_new()` and propagate/skip
explicitly; test code keeps the infallible `new()`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(core): bound the SQLite pool Condvar wait

`Pool::acquire` used an unbounded `Condvar::wait`, so a leaked
PoolGuard or a deadlocked query would silently park a blocking-pool
thread forever with no signal. Wait in 5s slices via `wait_for`,
re-checking the pool each wake and logging with escalating severity
(warn, then error past 30s) so a stall is diagnosable. Also drops the
`.expect("pool invariant…")` by popping inside the loop instead of
asserting non-emptiness after the wait.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(app): async-safe project scan + cross-platform home

`detect_projects` (a #[tauri::command] async fn) and `auto_detect_projects`
both ran blocking `read_dir`/`exists` directly on the async runtime and
keyed off `std::env::var("HOME")` (broken on Windows). Extract one
cross-platform sync scanner (`scan_ministr_projects`, via the existing
`home_pathbuf()` HOME/USERPROFILE helper) and run it through
`spawn_blocking` from both call sites. Normalize separators in the
expanded remainder of `expand_tilde` so a mixed `~/a\b` input resolves.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(api): retry transient 5xx/timeout on idempotent GET

The daemon client issued every request exactly once: a transient 5xx
or a request timeout failed the call even though a safe, idempotent
GET could be replayed without risk. Add `get_with_retry` — bounded
backoff (200ms/500ms/1s) retry, gated to GET and to transient faults
only (5xx response or ClientError::Timeout). Connect-failure, 4xx and
decode errors are returned with their kind intact, never retried;
non-idempotent POST/PUT/DELETE bypass retry entirely. No new
dependency (ministr-api stays tracing-free; retry is silent).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* chore(app): log previously-discarded fs/process results

`let _ =` swallowed several filesystem/process failures: the
first-launch sentinel writes (main.rs, x3 → shared `mark_done` helper
that warns), the setup version marker (setup.rs), the launchd
`launchctl load` invocation (setup.rs, now branches on spawn error vs
non-zero exit), and the daemon socket/pid cleanup on tray-quit
(tray.rs, NotFound ignored, other errors warned). Behaviour is
unchanged (all remain non-fatal) but a persistently unwritable data
dir or a failing launchctl is now diagnosable instead of silent.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* refactor(app): structured CommandError for Tauri commands

Replace the stringly-typed `Result<T, String>` / `map_err(|e|
e.to_string())` pattern across all 35 #[tauri::command] fns with a
classified `CommandError { kind, message }` (kinds: registry/io/
not_found/invalid_input/internal) plus `From` impls (RegistryError,
io::Error, String/&str) so command bodies use `?` and typed
constructors instead of ad-hoc `.to_string()` mapping.

Wire form is deliberately the bare message string (not a {kind,message}
object): the React frontend renders failures with `String(e)` in ~20
sites with no central invoke chokepoint, so an object would render as
"[object Object]". This is byte-identical to prior behaviour while the
Rust side gains the structure. `Serialize` is the single command→
frontend chokepoint, so it also emits a structured `tracing::warn!`
carrying the `kind` the wire drops — server-side observability with no
per-command instrumentation. A future UI-coordinated pass can switch
the wire to an object and consume `kind`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* perf(app): visibility-aware polling for the shell heartbeat

ministr runs hidden in the tray by default, yet `useDaemonStatus`
polled `daemon_status` every 2s forever (and `useSessionHistory`
re-read storage every 5s) even with the window invisible — wasted
IPC/CPU/battery, inconsistent with `useSessions`/`useSessionActivity`
which already pause on `document.hidden`.

Rework the daemon-status heartbeat to a scheduled poll loop: paused
while hidden (re-polls instantly on re-show via visibilitychange),
exponential backoff on error capped at 30s (recovers to base cadence
on next success), `refresh()` still forces an immediate poll. Public
hook API unchanged. `useSessionHistory` skips its tick while hidden
and catches up on show.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(app): actionable daemon error banner

The top-of-shell error band rendered just an icon + raw string with no
recourse. Replace it with a classified banner that distinguishes a
daemon we can't reach (no status — stopped/starting) from a transient
command failure (status present, one call errored), and surfaces the
two actions that help: Retry (forces an immediate heartbeat poll) and
Logs (when a log path is known). Styling stays on the Cockpit
contract (existing Button primitive, danger band).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* fix(app): give the add-project / open-logs / scan flows real feedback

These primary actions swallowed every error as "user cancelled" /
"ignore". Worse, `add_project_dialog` resolves to `null` on a
cancelled picker (not a throw), so the old code toasted "Project
added" and refreshed even when the user backed out, and hid genuine
failures entirely.

Now: distinguish cancel (null → no-op) from failure (danger toast with
the reason) in both add-project entry points (TopBar/palette via App,
and the Projects surface button); open-logs failure toasts instead of
vanishing; the project scan reports found/none/failed instead of a
silent console.error. Other `catch {}` sites are genuinely best-effort
(localStorage prefs, history ring, poll deltas) and left annotated.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(app): shared useDialog — real modal a11y for overlays

The overlays painted role=dialog/aria-modal but skipped the behaviour
that makes it true. EntityPanel's "Close · Esc" was a lie (no key
handler at all); the destructive ConfirmDialog had no Escape and left
focus on the trigger behind the scrim; none trapped Tab or restored
focus on close.

New `useDialog(open, onClose, { initialFocus })`: Escape-to-close
(document capture + stopImmediatePropagation so it fires once and
never double-runs with the app-level shortcut handler), focus moves
into the dialog and is restored to the prior element on close, and
Tab/Shift+Tab are trapped. Applied to EntityPanel, ShortcutSheet,
ConfirmDialog (focus → token field when type-to-confirm, else the safe
Cancel) and CommandPalette (adds focus-restore + trap; keeps its input
autofocus via initialFocus). `Button` is now forwardRef so the dialog
can target Cancel. ToastTray already had aria-live.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

* feat(app): command-palette completeness

The palette was the keyboard spine but missing primary actions, so
they were only reachable by mouse on specific surfaces. Add under the
> actions mode: Re-index active project, Open logs, Cycle theme
(System→Dark→Light), and Inspect <active project> (opens the corpus
entity inspector via the existing openEntity, no new backend). App
owns the handlers and surfaces feedback (toasts) / refreshes the
heartbeat, consistent with the other action paths. Symbol search is
intentionally out of scope (needs an async search backend in the
palette) and left for a future unit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>

---------

Co-authored-by: Claude Opus 4.7 (1M context) <noreply@anthropic.com>- *(indexer)* UE5-grade index speedup (6 phases) ([#47](https://github.com/OlsonSoftware/ministr/pull/47))- Revert "ci(release): forward Apple Developer ID secrets to tauri-action"

This reverts commit 7f87e05df1d7bf68883d25bad7f9ca4ac37a7cde.- *(positioning)* Third pass — strip CPU-cache analogies from concept docs- *(positioning)* Second pass — strip remaining cache + OSS leaks- *(positioning)* Drop OSS signals + reframe focal point as codebase intelligence- *(install)* Close gaps between docs and shipped artifacts- *(changelog)* Note ministr.app is now a Cloudflare redirect ruleset ([#23](https://github.com/OlsonSoftware/ministr/pull/23))- Animated session trace + VHS terminal demo for landing- Polish pass — accurate on-disk layout, honest install options, tool-page icons, 404, OG card- Stop overclaiming — iris doesn't manage the agent's context window- Rebuild styling with Tailwind v4 + Phosphor icons + iridescent identity- Replace all ASCII box diagrams with D2- Replace Mermaid with D2 for diagrams- Enhance landing page with stats, architecture diagram, session trace- Migrate website from mdBook to Material for MkDocs- CLI reference page, eval README, and template polish- Polish pass across user-facing files- Add reference pages for all missing MCP tools- Redesign README with centered hero and tighter structure- Overhaul README and user-facing presentation- Update mdBook for current architecture and .iris.toml config- Add embedding research papers and presentation notes- Clean up import order and formatting in embedding modules- Parking_lot, #[must_use], long-params cleanup- *(service)* Decompose service.rs into service/ module directory- *(mcp)* Decompose server.rs into focused sub-modules- Format code for improved readability and consistency- Decompose main.rs into commands, infra, ingestion modules- Pipeline DRY, dead code cleanup, and clippy fixes- *(tray)* Add verbose logging to diagnose disappearing corpora- Overhaul with 11 research-informed phases (65 new tasks)- Update workspace structure from 3 crates to 6- Narrow pipe blocking to search-only tools- Add autoheal for hook files + fix double-scaffold- Add Cursor hooks (.cursor/hooks.json) enforcement- Split ingestion.rs into submodules with shared processing pipeline- Split ingestion into submodules with deduplicate_ids- OSS2 — launch content, landing page, post drafts, registry submissions- Clean architecture — SOLID, DRY, no warnings- Sync ROADMAP.md with completed OSS1 and ML1 phases- Add architecture deep dive, cloud plan, and launch checklist- Rewrite README for open source launch and add CONTRIBUTING.md- Add Claude Code rules for iris tool usage and scope- Async batch embedding with progress logging and tokio yielding- Deferred batch embedding — parse all files first, embed once- Enable CoreML execution provider for GPU/Neural Engine on Apple Silicon- Final hardening — tracing levels, dead code, deny.toml cleanup- Add dual MIT/Apache-2.0 licenses, README, and repository metadata- Replace Selector::parse().expect() with OnceLock statics in HTML parsers- Add MCP client setup guide and run final audit- Build mdBook documentation site- Add doc tests for public APIs in iris-core
### Fixed
- *(release-plz)* Version requirements on internal path deps + vendored readme ([#78](https://github.com/OlsonSoftware/ministr/pull/78))- *(release-plz)* Git_only=true — version detection from git tags, not the empty registry ([#77](https://github.com/OlsonSoftware/ministr/pull/77))- Restore version invariant (0.2.3 → 0.2.1) so release-plz can run ([#75](https://github.com/OlsonSoftware/ministr/pull/75))- *(release)* Windows release path 100% bash-free (idempotent PS bootstrap + Python) ([#73](https://github.com/OlsonSoftware/ministr/pull/73))- *(release)* Make shell:bash use Git Bash, not WSL, on self-hosted Windows ([#72](https://github.com/OlsonSoftware/ministr/pull/72))- *(release-plz)* Release_always = false — gate releases to merged release PR ([#70](https://github.com/OlsonSoftware/ministr/pull/70))- *(changelog)* Strip corrupted release-plz dup blocks + unify future sections ([#68](https://github.com/OlsonSoftware/ministr/pull/68))- *(release-plz)* Exclude vendored crate, fix version baseline, kill semver-checks ([#64](https://github.com/OlsonSoftware/ministr/pull/64))- Subagent session isolation + Tauri UI cleanup ([#37](https://github.com/OlsonSoftware/ministr/pull/37))- *(daemon)* Guard live-merge against stuck IngestionProgress ([#36](https://github.com/OlsonSoftware/ministr/pull/36))- *(daemon)* Merge live progress into CorpusInfo for status reads ([#35](https://github.com/OlsonSoftware/ministr/pull/35))- *(installer)* NSIS hook works (uses nsExec, not EnVar) + reinstall recipes via ministr setup ([#34](https://github.com/OlsonSoftware/ministr/pull/34))- *(docs)* Convert remaining ASCII diagrams + MCP tool mapping- *(docs)* Convert key ASCII diagrams to D2 + auto-style every D2 block- *(docs)* Diagrams adopt the same card aesthetic as the rest of the site- *(docs)* Diagrams now show borders + Inter labels everywhere- *(docs)* Center comparison table without stretching it- *(docs)* Center comparison table — force Material's table wrapper to block- *(docs)* Rewrite comparison table + tighten cross-component design- *(docs)* Style VHS demo to match Claude Code's terminal UI- *(docs)* Rewrite VHS tape so viewers see iris output, not printf scaffolding- *(docs)* Restore white text on 404 primary CTA- *(docs)* Restyle code-block copy button for Material 9.7- *(docs)* Inline Phosphor icons at build time — fixes nav breakage on published site- *(docs)* Hero CTA pointed to getting-started.md (404)- Replace alrik/iris-rs with AlrikOlson/iris-rs everywhere- Replay delivered items into budget tracker on session restore- Proxy reads now track session budget via session-aware daemon endpoint- Resolve method-level cross-references by matching function symbols in from_context- *(iris-app)* Ad-hoc code sign to fix macOS Gatekeeper 'damaged' error- Remove missing .icns/.ico icon references from tauri bundle config- Vendor Metal-compatible BERT model for GPU embedding- Candle Metal broadcast shape mismatch in mean pooling- Enable Candle backend for daemon and Tauri app- Detect co-located frontends in Cargo workspaces + fix duplicate sessions in GUI- *(daemon)* Clear stale sessions on proxy startup- *(proxy)* Eager session creation so GUI shows sessions immediately- *(daemon)* Deadlock in unregister — write lock held during save_manifest- *(tray)* Handle IPC bridge unavailability after webview reload- *(tray)* Add CSP for IPC protocol and console error logging- *(tray)* Replace blocking_read with async .await in trigger_reindex- *(tray)* Prevent duplicate corpus registrations and fix card display- *(symbols)* Improve iris_symbols reliability with smarter query tokenization- *(mcp)* Mark skip_serializing_if fields as optional in JSON schema- *(scaffold)* Use Copilot CLI native hook format for .github/hooks/- Implement light/dark theme switching and add onboarding reset- Set files_done=files_total on mtime fast-skip path- Use IngestionProgress status field instead of file count comparison- Add Tauri v2 capabilities and fix ingestion progress invocation- Handle stale documents causing UNIQUE constraint on source_path- LogViewer auto-scroll toggle ignoring user preference- Improve symbol search for multi-word queries and module filters- Show total file/section counts in tray app after restart- Proxy RunningService dropped immediately, killing tools/list- Derive project name from common ancestor path, not first path leaf- Remove dynamic tray menu rebuild that caused macOS crash- Resolve CoreML memory leak and add RSS profiler- Stream embeddings incrementally and add hard ignore-dir guard- Plug memory leaks in session subsystem- Recover from corrupted HNSW index instead of crashing- Bridge detector walks up to .iris.toml/git root to find manifests- Centralize fastembed model cache in ~/.iris/models/- Bridge false positives, missing Tauri commands, and warm-restart ref gap- Clone persistence across restarts and ordering-dependent ref resolution- Reference graph completeness, ref kind misclassification, and symbol compress/extract- TOC pagination, module filter path derivation, and indexing placeholder filtering- Skip identity compression for small sections in iris_compress- Survey dedup before truncation, symbol cross-reference extraction- Enable parent .gitignore resolution for subdirectory corpus roots- Robust ingestion with background startup, unique document IDs, and section dedup- Skip re-delivery of unchanged content in iris_read to save context tokens
