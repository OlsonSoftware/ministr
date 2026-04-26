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
    ./scripts/build-pkg.sh

# Build macOS .pkg without notarization (for local testing)
pkg-dev:
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
    cargo build --release -p ministr-cli -p ministr-app
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
    if ! ~/.ministr/bin/ministr setup --bin-dir ~/.ministr/bin; then
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
    cp target/release/ministr-app /Applications/ministr.app/Contents/MacOS/ministr-app
    # Sidecar binary lives inside the bundle too; keep it in sync.
    if [ -f /Applications/ministr.app/Contents/MacOS/ministr-cli ]; then
        cp target/release/ministr /Applications/ministr.app/Contents/MacOS/ministr-cli
    fi
    # We modified the bundle contents; ad-hoc re-sign so macOS will launch it.
    codesign --force --deep --sign - /Applications/ministr.app >/dev/null 2>&1 || true
    echo "==> Launching tray app..."
    open /Applications/ministr.app
    echo "==> Done. Restart your Claude Code session to pick up the new binary."

# Clean rebuild + install CLI + Tauri app + launch tray (Windows)
[windows]
reinstall:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\reinstall.ps1

# Run all quality gates: format check + build + test + lint
validate: fmt-check lint test

# Cut a release: bump versions, update CHANGELOG, commit + tag
release version:
    #!/usr/bin/env bash
    set -euo pipefail
    # Validate version format
    if ! echo "{{version}}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "error: version must be in semver format (e.g. 0.2.0)" >&2
        exit 1
    fi
    # Bump version in every workspace crate. Must match the root
    # [workspace] members list — missing one breaks cross-crate publish
    # ordering because path deps still pin the old version.
    # Uses `-i.bak` + explicit rm so the recipe works on both GNU sed
    # (Linux CI) and BSD sed (macOS dev machines).
    for toml in \
        ministr-api/Cargo.toml \
        ministr-core/Cargo.toml \
        ministr-daemon/Cargo.toml \
        ministr-mcp/Cargo.toml \
        ministr-cli/Cargo.toml \
        ministr-app/src-tauri/Cargo.toml; \
    do
        sed -i.bak -e "s/^version = \".*\"/version = \"{{version}}\"/" "$toml"
        rm -f "$toml.bak"
    done
    # Add new section to CHANGELOG.md (inserted before the first
    # existing `## [` heading so the freshest release stays on top).
    date=$(date +%Y-%m-%d)
    printf '\n## [{{version}}] - %s\n\n### Added\n\n### Changed\n\n### Fixed\n\n' "$date" | \
        sed -i.bak -e "/^## \[/r /dev/stdin" CHANGELOG.md
    rm -f CHANGELOG.md.bak
    # Add link reference at bottom
    echo "[{{version}}]: https://github.com/OlsonSoftware/ministr/releases/tag/v{{version}}" >> CHANGELOG.md
    # Validate the workspace compiles
    cargo check --workspace
    # Commit and tag
    git add -A
    git commit -m "release: v{{version}}"
    git tag "v{{version}}"
    echo "Tagged v{{version}} — push with: git push origin main v{{version}}"
