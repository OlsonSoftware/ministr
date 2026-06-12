# ministr — task runner

set windows-shell := ["cmd.exe", "/c"]

# ── Core ─────────────────────────────────────────────────────────────

build:
    cargo build --workspace

test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# ── Web (Next.js site at web/) ───────────────────────────────────────

web-dev:
    cd web && npm run dev

web-build:
    cd web && npm run build

web-typecheck:
    cd web && npm run types:check

web-deps:
    cd web && npm install

# ── Desktop app (Tauri frontend at ministr-app/) ─────────────────────

# Storybook dev server (port 6006) — the GUI-rewrite visual-iteration loop.
storybook:
    cd ministr-app && pnpm storybook

# Static Storybook build (storybook-static/) — what CI/scrutiny consumes.
storybook-build:
    cd ministr-app && pnpm build-storybook

# Frontend test gate: vitest unit + every story in real Chromium with axe,
# light AND dark.
app-test:
    cd ministr-app && pnpm test

app-dev:
    cd ministr-app && pnpm dev

# ── Quality gates ────────────────────────────────────────────────────

# Desktop-app frontend gate — the CANONICAL pnpm path (ministr-app uses pnpm;
# only web/ uses npm). `--frozen-lockfile` fails if package.json and
# pnpm-lock.yaml drift (the exact breakage `just reinstall` hit when a dep was
# added without updating the pnpm lockfile); tsc + build catch type/bundler
# regressions; design-lint enforces the UI contract. Inside `validate` so the
# frontend can't be silently broken behind a green gate.
validate-app:
    cd ministr-app && pnpm install --frozen-lockfile
    cd ministr-app && pnpm exec tsc --noEmit
    cd ministr-app && node scripts/design-lint.cjs
    cd ministr-app && pnpm run build

# All checks: format + lint + test + desktop-app gate + black-box guard
validate: fmt-check lint test validate-app
    python3 scripts/ci/blackbox_lint.py 2>/dev/null || python scripts/ci/blackbox_lint.py

# Verify the workspace compiles on the declared MSRV (rust-version = 1.88).
# `+1.88` overrides rust-toolchain.toml (which pins the repo to 1.95.0), so
# this exercises the MSRV rather than the pinned toolchain. Self-contained:
# installs the 1.88 toolchain if missing.
msrv:
    rustup toolchain install 1.88 --profile minimal --no-self-update
    cargo +1.88 check --workspace --locked

# Pre-release: validate + MSRV + audit + eval gate + web build
release-preflight: validate msrv
    cargo audit
    cargo deny check
    cargo test --test eval_retrieval eval_retrieval_regression_gate -p ministr-core -- --nocapture
    cd web && npm run types:check && npm run build

# ── Benchmarks ───────────────────────────────────────────────────────

bench:
    cargo bench --bench search -p ministr-core

# Real-embedder retrieval-quality eval + regression gate (rq0). Loads the real
# embedding model (downloads on first run) and reports recall@k/nDCG@k/MRR over
# eval/corpus + eval/ground-truth.json. Deliberately OUTSIDE `just validate` so
# CI never downloads a model. Re-seed the BASELINE_* floors in
# tests/eval_retrieval.rs from the printed numbers to tighten the gate.
#
# MINISTR_COREML=0 pins the eval to the CPU execution provider: the macOS
# default (CoreML CPUAndGPU) is run-to-run NONDETERMINISTIC — Metal parallel
# reductions flip 3/75 near-tie queries at the 2nd nDCG decimal — while the
# CPU EP is hash-identical across runs (proven 6x, 2026-06-12). Eval-only:
# production keeps the GPU path. The exact-scan index (W1) + this pin
# together make the gate byte-deterministic end to end.
eval-quality $MINISTR_COREML="0":
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture eval_retrieval_real_embedder

# RQ1 content-loss measurement: how many embedded sections exceed the 128/256
# token cap, and what fraction of content the cap drops. Ingests eval/corpus
# with a mock embedder (model-free) and tokenizes with the real all-MiniLM
# WordPiece tokenizer (downloads tokenizer.json on first run). Run after the
# RQ1 truncation fix to quantify the recovered content; pairs with eval-quality.
eval-truncation:
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture measure_truncation_content_loss

# RQ2 embedder bake-off: benchmark candidate embedding models against the eval
# golden set and print a dim / P@5 / R@5 / MRR / nDCG@5 comparison table.
# Downloads several models on first run (some large, e.g. bge-m3). Use the
# spread to pick a default; the production swap is a separate re-index step.
# `--exact` so it doesn't also match eval_model_bakeoff_code.
eval-bakeoff:
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture --exact eval_model_bakeoff

# RQ2-followup CODE bake-off: the same comparison over the CODE-HEAVY corpus
# (eval/corpus-code + eval/ground-truth-code.json, 26 text-to-code queries over
# 6 languages). Decides jina-code vs MiniLM on a code-representative corpus —
# what agents actually retrieve. Downloads the same models as eval-bakeoff.
eval-bakeoff-code:
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture eval_model_bakeoff_code

# rq-ast-sparse-encoder gate: dense-only vs zero-model AST/BM25F hybrid
# (sparse_weight 0.6) on the code corpus, same dense model, deterministic
# exact-scan index. Asserts hybrid beats dense on nDCG@5 + MRR and holds R@5;
# prints the in-repo sparse encode cost. CPU-pinned like eval-quality.
eval-ast-code $MINISTR_COREML="0":
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture --exact eval_ast_hybrid_code

# Calibration dump (model-free): print the real content_ids the index emits for
# eval/corpus-code, used to author/verify eval/ground-truth-code.json section_ids.
eval-dump-code-ids:
    cargo test -p ministr-core --test eval_retrieval -- --ignored --nocapture dump_code_corpus_ids

# cq-throughput (BENCH): drive 25+ mixed-size corpora through the real
# IngestionCoordinator queue (cq-queue/priority/coalesce) with a fast mock
# embedder and print end-to-end throughput (corpora/s, files/s). Measures
# coordinator + pipeline orchestration, not embedding speed.
cq-throughput-bench:
    cargo test -p ministr-daemon --test throughput_bench -- --ignored --nocapture --exact coordinator_throughput_25_corpora

bench-all:
    cargo bench -p ministr-core

# ── Build & install ──────────────────────────────────────────────────

# Clean rebuild + install CLI + Tauri app + relaunch tray
[unix]
reinstall:
    bash scripts/reinstall.sh

[windows]
reinstall:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\reinstall.ps1

# ── Profiling ────────────────────────────────────────────────────────

# Relaunch the dev app with embed-batch debug timing (profile embed throughput).
[macos]
profile-embed:
    #!/usr/bin/env bash
    set -euo pipefail
    APP="${MINISTR_APP:-$HOME/Applications/ministr.app}"
    [ -d "$APP" ] || APP="/Applications/ministr.app"
    BIN="$APP/Contents/MacOS/ministr-app"
    [ -x "$BIN" ] || { echo "ministr app not found at $BIN — run 'just reinstall' first (or set MINISTR_APP)" >&2; exit 1; }
    echo "==> Stopping running ministr instances..."
    pkill -TERM -f "ministr-app" 2>/dev/null || true
    pkill -TERM -f "ministr __daemon" 2>/dev/null || true
    sleep 1
    rm -f "$HOME/.ministr/ministrd.sock" "$HOME/.ministr/ministrd.pid"
    echo "==> Launching $APP with embed-batch timing (debug) -> ~/.ministr/ministr.log"
    echo "    Re-index a corpus (or add a new folder), then: just profile-embed-report"
    exec env RUST_LOG='ministr_core=info,ministr_core::embedding::candle_impl=debug' "$BIN"

# Show recent embed-batch timing samples (tokenize vs GPU compute, batch, len).
[unix]
profile-embed-report:
    #!/usr/bin/env bash
    set -euo pipefail
    log="$HOME/.ministr/ministr.log"
    [ -f "$log" ] || { echo "no log at $log" >&2; exit 1; }
    awk '/candle embed_batch timing/{a[++n]=$0}
         END{
           if(n==0){print "no \"candle embed_batch timing\" lines yet — launch via \`just profile-embed\` (debug filter) then re-index"; exit 0}
           start=(n>20?n-19:1)
           for(i=start;i<=n;i++) print a[i]
           print "---- samples=" n " (showing last " (n-start+1) ") ----"
         }' "$log"

# Point login auto-launch at the dev bundle (~/Applications) — revert: ministr setup.
[macos]
dev-autolaunch:
    #!/usr/bin/env bash
    set -euo pipefail
    plist="$HOME/Library/LaunchAgents/ai.ministr.desktop.plist"
    [ -f "$plist" ] || { echo "no login agent at $plist — run 'ministr setup' first" >&2; exit 1; }
    dev="${MINISTR_APP:-$HOME/Applications/ministr.app}"
    [ -d "$dev/Contents/MacOS" ] || { echo "dev bundle not found at $dev — run 'just reinstall' first" >&2; exit 1; }
    cur="$(/usr/libexec/PlistBuddy -c 'Print :ProgramArguments:0' "$plist" 2>/dev/null || true)"
    [ -n "$cur" ] || { echo "could not read ProgramArguments:0 from $plist" >&2; exit 1; }
    case "$cur" in
        *"/ministr.app/"*) rel="${cur#*/ministr.app/}" ;;
        *) echo "unexpected launcher path in plist: $cur" >&2; exit 1 ;;
    esac
    newexec="$dev/$rel"
    [ -x "$newexec" ] || { echo "dev launcher missing/not executable: $newexec (run 'just reinstall')" >&2; exit 1; }
    cp "$plist" "$plist.bak"
    /usr/libexec/PlistBuddy -c "Set :ProgramArguments:0 $newexec" "$plist"
    uid="$(id -u)"
    launchctl bootout "gui/$uid/ai.ministr.desktop" 2>/dev/null || true
    launchctl bootstrap "gui/$uid" "$plist"
    echo "Login now auto-launches the DEV bundle:"
    echo "  $newexec"
    echo "Backup: $plist.bak   Revert with: ministr setup  (re-points at /Applications)"

# ── Local data ───────────────────────────────────────────────────────

# Destructive + irreversible. Wipes the daemon data dir (~/.ministr) — every
# corpus's content.db + HNSW index, logs, socket, PID, onboarding markers —
# after stopping the daemon. PRESERVES ~/.ministr/bin (so the `ministr`
# command keeps working); per-project .ministr.toml files are untouched.
# Corpora must be re-indexed afterward. For a TOTAL nuke incl. the installed
# binary: `rm -rf ~/.ministr` then `just reinstall`. Unix-only (macOS/Linux).
# Reset ALL local ministr data on this machine (stop daemon + wipe ~/.ministr, keep bin).
[unix]
[confirm("Wipe ALL local ministr corpora + index data in ~/.ministr (keeps ~/.ministr/bin)? Re-index needed afterward.")]
reset-data:
    #!/usr/bin/env bash
    set -euo pipefail
    data_dir="$HOME/.ministr"
    pid_file="$data_dir/ministrd.pid"
    # 1. Stop the daemon first so it isn't writing while we wipe (and can't
    #    flush in-memory state back to disk on shutdown). Prefer the PID file;
    #    fall back to a pattern match for a stale/missing PID file.
    if [ -f "$pid_file" ]; then
        pid="$(cat "$pid_file" 2>/dev/null || true)"
        if [ -n "${pid:-}" ] && kill -0 "$pid" 2>/dev/null; then
            echo "stopping ministr daemon (pid $pid)…"
            kill "$pid" 2>/dev/null || true
            for _ in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.25; done
            kill -9 "$pid" 2>/dev/null || true
        fi
    fi
    pkill -f '__daemon' 2>/dev/null || true
    # 2. Wipe everything under the data dir EXCEPT the installed binary.
    if [ -d "$data_dir" ]; then
        echo "wiping $data_dir (preserving bin/)…"
        find "$data_dir" -mindepth 1 -maxdepth 1 ! -name bin -exec rm -rf {} +
    fi
    echo "ministr data reset — re-index your corpora to rebuild."

# ── Docker ───────────────────────────────────────────────────────────
docker-build:
    docker build -t ministr .

docker-run *args:
    docker run -p 8080:8080 -v ministr_data:/data ministr {{args}}
