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

# Clean rebuild + install CLI + Tauri app + restart daemon
[unix]
reinstall:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Killing existing ministr daemons..."
    pkill -f "ministr-app" || true
    pkill -f "ministr serve" || true
    rm -f ~/.ministr/ministrd.sock ~/.ministr/ministrd.pid
    sleep 1
    echo "==> Clean rebuild (release)..."
    cargo clean -p ministr-mcp -p ministr-cli -p ministr-daemon -p ministr-app
    cargo build --release -p ministr-cli
    # Tauri's externalBin (tauri.conf.json) requires the sidecar at
    # `ministr-app/src-tauri/binaries/ministr-cli-<host-triple>` before
    # the ministr-app build script runs. Mirror scripts/reinstall.ps1.
    HOST_TRIPLE=$(rustc -vV | awk '/^host:/ { print $2 }')
    mkdir -p ministr-app/src-tauri/binaries
    cp target/release/ministr "ministr-app/src-tauri/binaries/ministr-cli-${HOST_TRIPLE}"
    # Tauri's `generate_context!` proc macro reads `frontendDist` from
    # tauri.conf.json (`../dist`) at compile time, so the Vite output
    # must exist before `cargo build -p ministr-app`. `tauri build` would
    # run beforeBuildCommand for us; raw cargo doesn't.
    # Always sync — checking `-d node_modules` skips a partial install
    # (lockfile drift, interrupted prior run) and leaves vite to fail at
    # build time. `--frozen-lockfile` is a no-op when in sync and fails
    # loudly if package.json and pnpm-lock.yaml disagree.
    echo "==> Syncing frontend deps (pnpm install --frozen-lockfile)..."
    (cd ministr-app && pnpm install --frozen-lockfile)
    echo "==> Building frontend (vite)..."
    (cd ministr-app && pnpm run build)
    cargo build --release -p ministr-app
    echo "==> Installing CLI to ~/.ministr/bin/ministr (canonical dev location)..."
    # Remove stale copies from other locations to prevent shadow binaries.
    rm -f ~/.cargo/bin/ministr
    rm -f /usr/local/bin/ministr 2>/dev/null || true
    mkdir -p ~/.ministr/bin
    cp target/release/ministr ~/.ministr/bin/ministr
    # Hand off PATH wiring to `ministr setup` (onpath crate). Detects
    # installed shells and writes the right rc-file edits. Idempotent
    # — re-runs of this dev recipe won't duplicate entries. Non-fatal:
    # the binary is at ~/.ministr/bin/ministr regardless, so PATH-wiring
    # trouble shouldn't abort the rest of the reinstall.
    echo "==> Adding ministr to PATH via \`ministr setup\`..."
    if ! ~/.ministr/bin/ministr setup; then
        echo "   ministr setup failed — add manually with:" >&2
        echo "     export PATH=\"\$HOME/.ministr/bin:\$PATH\"" >&2
    fi
    echo "==> Installing Tauri app..."
    if [ ! -d /Applications/ministr.app/Contents/MacOS ]; then
        echo "   ministr.app bundle not found at /Applications/ministr.app." >&2
        echo "   This recipe only updates the inner binary; it cannot build the" >&2
        echo "   .app bundle from scratch. Run \`just pkg-dev\` (or \`just pkg\` for" >&2
        echo "   a signed+notarized build), install the produced .pkg, then" >&2
        echo "   re-run this recipe." >&2
        exit 1
    fi
    # Bundles installed from a signed .pkg are owned by root; bundles
    # built locally with `cargo build -p ministr-app` are owned by the
    # current user. Use sudo only when needed so dev re-runs don't
    # prompt for a password unnecessarily.
    SUDO=""
    if [ ! -w /Applications/ministr.app/Contents/MacOS/ministr-app ]; then
        SUDO="sudo"
        echo "   bundle is root-owned (.pkg-installed) — using sudo for in-place updates"
    fi
    $SUDO cp target/release/ministr-app /Applications/ministr.app/Contents/MacOS/ministr-app
    # Sidecar binary lives inside the bundle too; keep it in sync.
    if [ -f /Applications/ministr.app/Contents/MacOS/ministr-cli ]; then
        $SUDO cp target/release/ministr /Applications/ministr.app/Contents/MacOS/ministr-cli
    fi
    # We modified the bundle contents; ad-hoc re-sign so macOS will launch it.
    $SUDO codesign --force --deep --sign - /Applications/ministr.app >/dev/null 2>&1 || true
    echo "==> Launching tray app..."
    open /Applications/ministr.app
    echo "==> Done. Restart your Claude Code session to pick up the new binary."

# Clean rebuild + install CLI + Tauri app + launch tray (Windows)
[windows]
reinstall:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\reinstall.ps1

# Enforce the ministr-app UI design contract (see ministr-app/DESIGN.md)
design-lint:
    cd ministr-app && node scripts/design-lint.cjs

# Run all quality gates: format check + build + test + lint + UI contract
validate: fmt-check lint test design-lint

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
