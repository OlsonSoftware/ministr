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

# All checks: format + lint + test + UI contract + black-box guard
validate: fmt-check lint test
    cd ministr-app && node scripts/design-lint.cjs
    python3 scripts/ci/blackbox_lint.py 2>/dev/null || python scripts/ci/blackbox_lint.py

# Pre-release: validate + audit + eval gate + web build
release-preflight: validate
    cargo audit
    cargo deny check
    cargo test --test eval_retrieval eval_retrieval_regression_gate -p ministr-core -- --nocapture
    cd web && npm run types:check && npm run build

# ── Local cloud dev ──────────────────────────────────────────────────

dev-cloud-up:
    docker compose -f docker-compose.dev.yml up -d
    @echo "Postgres on localhost:55432. Next: source .env.dev && cargo run -- serve --transport http --oauth"

dev-cloud-down:
    docker compose -f docker-compose.dev.yml down

dev-cloud-reset:
    docker compose -f docker-compose.dev.yml down --volumes

dev-cloud-psql:
    docker compose -f docker-compose.dev.yml exec postgres psql -U ministr -d ministr_dev

# E2E cloud harness (F-Test). KEEP=1 leaves stack running; PORT=... overrides 8088.
e2e-cloud-local *args:
    ./scripts/e2e-cloud-local.sh {{args}}

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

# Signed + notarized macOS .pkg
pkg:
    #!/usr/bin/env bash
    set -euo pipefail
    [ -f .env.signing ] && set -a && . ./.env.signing && set +a
    ./scripts/build-pkg.sh

# ── Azure (see deploy/azure/README.md) ───────────────────────────────

azure-init:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    cd deploy/azure && [ -d node_modules ] || npm ci
    pulumi stack ls 2>/dev/null | grep -q '^prod ' || pulumi stack init prod

azure-push:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    TAG="${TAG:-$(git rev-parse --short HEAD)-$(date +%s)}"
    REGISTRY=$(pulumi -C deploy/azure stack output registryServer 2>/dev/null || echo "$(pulumi -C deploy/azure config get projectName 2>/dev/null || echo ministr)acr.azurecr.io")
    az acr login --name "${REGISTRY%%.*}"
    docker buildx build --platform linux/amd64 --push -t "${REGISTRY}/ministr:${TAG}" .
    pulumi -C deploy/azure config set imageTag "${TAG}"

azure-up:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure up ${PULUMI_FLAGS:-}

azure-status:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure stack output
    URL=$(pulumi -C deploy/azure stack output publicBaseUrl 2>/dev/null || true)
    [ -n "$URL" ] && echo "" && curl -sS "${URL}/healthz" && echo

azure-logs:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    az containerapp logs show \
        --name "$(pulumi -C deploy/azure stack output appName)" \
        --resource-group "$(pulumi -C deploy/azure stack output resourceGroup)" \
        --follow --tail 100

azure-down:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure destroy ${PULUMI_FLAGS:-}

# Docker
docker-build:
    docker build -t ministr .

docker-run *args:
    docker run -p 8080:8080 -v ministr_data:/data ministr {{args}}
