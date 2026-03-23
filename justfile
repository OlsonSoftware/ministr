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

# Run all benchmarks
bench-all:
    cargo bench -p iris-core

# Build mdBook documentation site
docs:
    mdbook build docs

# Serve documentation locally with live reload
docs-serve:
    mdbook serve docs --open

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
    sed -i'' -e "/^## \[/i\\
## [{{version}}] - ${date}\\
\\
### Added\\
\\
### Changed\\
\\
### Fixed\\
" CHANGELOG.md
    # Add link reference at bottom
    echo "[{{version}}]: https://github.com/alrik/iris-rs/releases/tag/v{{version}}" >> CHANGELOG.md
    # Validate the workspace compiles
    cargo check --workspace
    # Commit and tag
    git add -A
    git commit -m "release: v{{version}}"
    git tag "v{{version}}"
    echo "Tagged v{{version}} — push with: git push origin main v{{version}}"
