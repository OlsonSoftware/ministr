# ministr — task runner recipes

# On Windows, use cmd.exe for recipe bodies that don't carry their own
# shebang. The just default is `sh`, which fails on Windows boxes without
# Git Bash / MSYS installed ("could not find the shell: program not found").
# Unix/macOS recipes are unaffected — `set windows-shell` only applies on
# Windows, and shebang recipes bypass this entirely on every platform.
set windows-shell := ["cmd.exe", "/c"]

# Build all workspace crates
build:
    cargo build --workspace

# Run all tests
test:
    cargo test --workspace

# Run clippy with pedantic lints
lint:
    cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Generate HTML coverage report
coverage:
    cargo llvm-cov --workspace --html

# Run cargo audit for known vulnerabilities
audit:
    cargo audit

# Run cargo deny checks (licenses, bans, advisories, sources)
deny:
    cargo deny check

# Run HNSW search benchmarks (no model download required)
bench:
    cargo bench --bench search -p ministr-core

# Run embedding throughput benchmarks (requires ~80MB model download)
bench-embedding:
    cargo bench --bench embedding -p ministr-core

# Run ingestion pipeline benchmarks (no model download required)
bench-ingestion:
    cargo bench --bench ingestion -p ministr-core

# Run prefetch cache benchmarks (no model download required)
bench-prefetch:
    cargo bench --bench prefetch -p ministr-core

# Run evaluation retrieval test with metrics output
bench-eval:
    cargo test --test eval_retrieval -p ministr-core -- --nocapture

# Run retrieval quality regression gate (fails build if metrics drop)
eval-gate:
    cargo test --test eval_retrieval eval_retrieval_regression_gate -p ministr-core -- --nocapture

# Compare embedding model retrieval quality (requires ~1GB model downloads)
bench-models:
    cargo test --test eval_model_comparison -p ministr-core --release -- --nocapture --ignored

# Compare a single model (pass model name, use @dim suffix for Matryoshka)
bench-model model:
    MINISTR_EVAL_MODELS="{{model}}" cargo test --test eval_model_comparison -p ministr-core --release -- --nocapture --ignored

# Emit a rust-analyzer LSIF index of this repo for the ministr-vs-LSP
# code-navigation benchmark (eval/lsp-nav/). Heavy + minutes-long; the
# comparison runner that diffs this against ministr is Phase 2.
bench-lsp-index:
    rustup component add rust-analyzer
    rustup run stable rust-analyzer lsif . > eval/lsp-nav/ra.lsif

# Run the ministr-vs-LSP code-navigation benchmark: emit the RA LSIF
# index, then diff ministr's answers against it over the hand-verified
# ground truth. Report-only (not a regression gate). Minutes-long.
bench-lsp: bench-lsp-index
    cargo test --test eval_lsp_nav -p ministr-core --release -- --nocapture --ignored

# Run all benchmarks
bench-all:
    cargo bench -p ministr-core

# Test Candle vs ONNX vector equivalence (macOS only, requires ~160MB model downloads)
test-backend-equiv:
    cargo test --test backend_equivalence -p ministr-core --features candle --release -- --ignored --nocapture

# ─────────────────────────────────────────────────────────────────────────────
# Documentation site (Fumadocs, Next.js) — docs-next/
# ─────────────────────────────────────────────────────────────────────────────

# Install node deps for the docs site
docs-deps:
    cd docs-next && npm install

# Build the docs site as a static export (output at docs-next/out/)
docs-build:
    cd docs-next && npm run build

# Run the Next.js dev server with hot reload (http://localhost:3000)
docs-dev:
    cd docs-next && npm run dev

# Serve the pre-built static export (http://localhost:3000)
docs-serve:
    cd docs-next && npm run start

# TypeScript + MDX + Next.js type-check (no build)
docs-typecheck:
    cd docs-next && npm run types:check

# Build Docker image
docker-build:
    docker build -t ministr .

# Run ministr in Docker with HTTP transport
docker-run *args:
    docker run -p 8080:8080 -v ministr_data:/data ministr {{args}}

# Build signed + notarized macOS .pkg installer
pkg:
    #!/usr/bin/env bash
    set -euo pipefail
    [ -f .env.signing ] && set -a && . ./.env.signing && set +a
    ./scripts/build-pkg.sh

# Build macOS .pkg without notarization (for local testing)
pkg-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    [ -f .env.signing ] && set -a && . ./.env.signing && set +a
    SKIP_NOTARIZE=1 ./scripts/build-pkg.sh

# Generate installer background images (requires librsvg)
pkg-backgrounds:
    ./installer/generate-backgrounds.sh

# Clean rebuild + install CLI + Tauri app + relaunch tray (macOS / Linux).
# Logic lives in scripts/reinstall.sh — parallels scripts/reinstall.ps1
# so the two stay in lockstep.
[unix]
reinstall:
    bash scripts/reinstall.sh

# Clean rebuild + install CLI + Tauri app + launch tray (Windows)
[windows]
reinstall:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\reinstall.ps1

# Enforce the ministr-app UI design contract (see ministr-app/DESIGN.md)
design-lint:
    cd ministr-app && node scripts/design-lint.cjs

# Closed-source guard: public surfaces (README, docs site, scaffolded
# agent rules) must stay black-box — no internal source paths/jargon.
blackbox-lint:
    #!/usr/bin/env bash
    set -euo pipefail
    # `python3` on Linux/macOS, `python` on Windows (see win_setup.ps1).
    if command -v python3 >/dev/null 2>&1; then py=python3; else py=python; fi
    "$py" scripts/ci/blackbox_lint.py

# Run all quality gates: format check + build + test + lint + UI + black-box
validate: fmt-check lint test design-lint blackbox-lint

# Release pre-flight gates — run before merging the release-plz PR
release-preflight: validate deny eval-gate
    cargo audit
    cd docs-next && npm run types:check && npm run build

# Releases are automated by release-plz (see RELEASE.md). Versions +
# CHANGELOG are bumped on a bot "release" PR from Conventional Commits;
# merging it pushes the `vX.Y.Z` tag that drives release.yml. There is
# no manual version-bump recipe anymore — this just points you there.
release:
    #!/usr/bin/env bash
    set -euo pipefail
    cat >&2 <<'EOF'
    Releases are automated by release-plz.

      1. Land your changes on `main` (Conventional Commit messages).
      2. release-plz keeps a "release" PR updated (version + CHANGELOG).
      3. Run `just release-preflight`, then merge that PR.
      4. release-plz pushes `vX.Y.Z`; release.yml builds + publishes.

    See RELEASE.md. To preview locally:
      release-plz update --config release-plz.toml   # dry-run diff
    EOF
    exit 1
