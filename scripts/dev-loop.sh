#!/usr/bin/env bash
# ministr backend-refactor dev loop.
#
# Tiered feedback — fastest signal first, expensive only when needed.
# Use Tier 1 in a watch-mode loop during active editing; Tier 2 after
# each handler is fully migrated; Tier 3 before each commit; Tier 4 only
# after a `just reinstall` to smoke-test the live daemon path.
#
# Usage:
#   scripts/dev-loop.sh tier1            # cargo check -p ministr-mcp
#   scripts/dev-loop.sh tier2 [filter]   # existing e2e tests (LocalBackend coverage)
#   scripts/dev-loop.sh tier3            # workspace build + clippy pedantic
#   scripts/dev-loop.sh tier4            # remind to reinstall + smoke
#   scripts/dev-loop.sh watch            # bacon watch on tier1
#   scripts/dev-loop.sh all              # tier1 → tier2 → tier3 sequentially

set -euo pipefail

cmd="${1:-all}"

cd "$(dirname "$0")/.."

tier1() {
    echo "==> Tier 1: cargo check -p ministr-mcp"
    cargo check -p ministr-mcp --lib --release
}

tier2() {
    local filter="${1:-}"
    echo "==> Tier 2: e2e tests (LocalBackend coverage)"
    if [ -n "$filter" ]; then
        cargo test --release -p ministr-mcp --test e2e_mcp -- "$filter"
    else
        # Full e2e is ~3 min — skip in default tier2. Use `tier2-full` to opt in.
        cargo test --release -p ministr-mcp --test e2e_mcp --no-run
        echo "    (compiled — run with a filter to actually execute, e.g. tier2 ministr_references)"
    fi
}

tier2-full() {
    echo "==> Tier 2 (full): all e2e tests"
    cargo test --release -p ministr-mcp --test e2e_mcp
}

tier3() {
    echo "==> Tier 3a: workspace build"
    cargo build --release --workspace

    echo "==> Tier 3b: clippy pedantic on the lib"
    cargo clippy --release -p ministr-mcp --all-targets -- \
        -D warnings -W clippy::pedantic

    echo "==> Tier 3c: workspace clippy (touched crates)"
    cargo clippy --release \
        -p ministr-api -p ministr-core -p ministr-daemon -p ministr-mcp \
        --all-targets -- -D warnings -W clippy::pedantic
}

tier4() {
    cat <<'EOF'
==> Tier 4: live smoke test
The refactor changes which code path the MCP server runs. To verify the
daemon-backend path end-to-end you have to swap the binary and restart
your editor's MCP connection. From a separate terminal:

    just reinstall

Then in your Claude Code session, restart the ministr MCP connection (or
reopen the workspace). The following tools should return non-empty,
schema-correct results:

    ministr_survey       "QueryBackend trait"
    ministr_symbols      "compute_impact"
    ministr_definition   <symbol_id from symbols>
    ministr_references   <symbol_id>
    ministr_impact       <symbol_id>
    ministr_dead         kind: "function" min_lines: 5 limit: 10
    ministr_toc
    ministr_bridge       limit: 3
    ministr_read         <section_id from survey>
    ministr_extract      <section_id>
    ministr_related      <claim_id>
    ministr_compress     content_ids: [<id>]

If any returns an error, the daemon-side conversion in
ministr-mcp/src/backend/convert.rs or the trait impl in
ministr-mcp/src/backend/daemon.rs is the prime suspect.
EOF
}

watch() {
    if ! command -v bacon >/dev/null 2>&1; then
        echo "bacon not installed. Falling back to cargo-watch."
        if ! command -v cargo-watch >/dev/null 2>&1; then
            echo "cargo-watch not installed either. Run:"
            echo "    cargo install bacon       # recommended"
            echo "    cargo install cargo-watch"
            exit 1
        fi
        exec cargo watch -c -x 'check -p ministr-mcp --lib --release'
    fi
    exec bacon -j ministr-mcp-check
}

case "$cmd" in
    tier1) tier1 ;;
    tier2) tier2 "${2:-}" ;;
    tier2-full) tier2-full ;;
    tier3) tier3 ;;
    tier4) tier4 ;;
    watch) watch ;;
    all) tier1 && tier3 ;;
    *)
        echo "Unknown: $cmd"
        echo "Usage: $0 [tier1|tier2 <filter>|tier2-full|tier3|tier4|watch|all]"
        exit 1
        ;;
esac
