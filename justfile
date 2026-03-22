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
