# iris — task runner recipes

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
    cargo bench --bench search -p iris-core

# Run embedding throughput benchmarks (requires ~80MB model download)
bench-embedding:
    cargo bench --bench embedding -p iris-core

# Run ingestion pipeline benchmarks (no model download required)
bench-ingestion:
    cargo bench --bench ingestion -p iris-core

# Run prefetch cache benchmarks (no model download required)
bench-prefetch:
    cargo bench --bench prefetch -p iris-core

# Run evaluation retrieval test with metrics output
bench-eval:
    cargo test --test eval_retrieval -p iris-core -- --nocapture

# Run retrieval quality regression gate (fails build if metrics drop)
eval-gate:
    cargo test --test eval_retrieval eval_retrieval_regression_gate -p iris-core -- --nocapture

# Compare embedding model retrieval quality (requires ~1GB model downloads)
bench-models:
    cargo test --test eval_model_comparison -p iris-core --release -- --nocapture --ignored

# Compare a single model (pass model name, use @dim suffix for Matryoshka)
bench-model model:
    IRIS_EVAL_MODELS="{{model}}" cargo test --test eval_model_comparison -p iris-core --release -- --nocapture --ignored

# Run all benchmarks
bench-all:
    cargo bench -p iris-core

# Test Candle vs ONNX vector equivalence (macOS only, requires ~160MB model downloads)
test-backend-equiv:
    cargo test --test backend_equivalence -p iris-core --features candle --release -- --ignored --nocapture

# Install documentation dependencies (Python: mkdocs-material)
docs-deps:
    pip install -r docs/requirements.txt

# Compile Tailwind v4 CSS (tokens.css → extra.css)
docs-css:
    tailwindcss -i docs/styles/tokens.css -o docs/src/stylesheets/extra.css --minify

# Watch Tailwind source, recompile on changes (run alongside docs-serve)
docs-css-watch:
    tailwindcss -i docs/styles/tokens.css -o docs/src/stylesheets/extra.css --watch

# Build MkDocs documentation site (Tailwind must be compiled first)
docs: docs-css
    cd docs && mkdocs build --strict

# Serve documentation locally with live reload (MD only; CSS stays static)
docs-serve: docs-css
    cd docs && mkdocs serve --open

# Live dev loop: Tailwind watcher + mkdocs serve in parallel (Ctrl-C cleans up both)
docs-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT INT TERM
    echo "▸ Starting Tailwind watcher + mkdocs serve"
    echo "  site     → http://127.0.0.1:8000"
    echo "  css src  → docs/styles/tokens.css  (auto-recompile)"
    echo "  md src   → docs/src/**/*.md        (auto-reload)"
    echo "  js src   → docs/src/javascripts/*  (auto-reload)"
    tailwindcss -i docs/styles/tokens.css -o docs/src/stylesheets/extra.css --watch &
    cd docs && mkdocs serve --open

# Rebuild Phosphor icon sprite from phosphor-icons/core
docs-icons:
    bash scripts/build-icon-sprite.sh

# Build Docker image
docker-build:
    docker build -t iris .

# Run iris in Docker with HTTP transport
docker-run *args:
    docker run -p 8080:8080 -v iris_data:/data iris {{args}}

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
reinstall:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> Killing existing iris daemons..."
    pkill -f "iris-app" || true
    pkill -f "iris serve" || true
    rm -f ~/.iris/irisd.sock ~/.iris/irisd.pid
    sleep 1
    echo "==> Clean rebuild (release)..."
    cargo clean -p iris-mcp -p iris-cli -p iris-daemon -p iris-app
    cargo build --release -p iris-cli -p iris-app
    echo "==> Installing CLI..."
    # Canonical dev location: ~/.iris/bin/iris (first in PATH).
    # Remove stale copies from other locations to prevent shadow binaries.
    rm -f ~/.cargo/bin/iris
    rm -f /usr/local/bin/iris 2>/dev/null || true
    mkdir -p ~/.iris/bin
    cp target/release/iris ~/.iris/bin/iris
    echo "==> Installing Tauri app..."
    cp target/release/iris-app /Applications/iris.app/Contents/MacOS/iris-app
    echo "==> Launching tray app..."
    open /Applications/iris.app
    echo "==> Done. Restart your Claude Code session to pick up the new binary."

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
    # Bump version in all workspace crates
    for toml in iris-cli/Cargo.toml iris-core/Cargo.toml iris-mcp/Cargo.toml; do
        sed -i'' -e "s/^version = \".*\"/version = \"{{version}}\"/" "$toml"
    done
    # Add new section to CHANGELOG.md
    date=$(date +%Y-%m-%d)
    printf '\n## [{{version}}] - %s\n\n### Added\n\n### Changed\n\n### Fixed\n\n' "$date" | \
        sed -i'' -e "/^## \[/r /dev/stdin" CHANGELOG.md
    # Add link reference at bottom
    echo "[{{version}}]: https://github.com/AlrikOlson/iris-rs/releases/tag/v{{version}}" >> CHANGELOG.md
    # Validate the workspace compiles
    cargo check --workspace
    # Commit and tag
    git add -A
    git commit -m "release: v{{version}}"
    git tag "v{{version}}"
    echo "Tagged v{{version}} — push with: git push origin main v{{version}}"
