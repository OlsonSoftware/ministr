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
