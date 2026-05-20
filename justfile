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

# ── Local cloud development (see DEMO.md) ──────────────────────────────

# Bring up the local cloud-mode dependencies (Postgres on :55432).
# Blob storage in dev mode uses the filesystem (FilesystemBlobStore),
# so no Azurite container is needed.
dev-cloud-up:
    docker compose -f docker-compose.dev.yml up -d
    @echo ""
    @echo "Postgres ready on localhost:55432 (user=ministr db=ministr_dev)."
    @echo "Next: copy .env.dev.example → .env.dev, fill the Stripe + GitHub bits, then:"
    @echo "  source .env.dev && cargo run -p ministr-cli -- serve --transport http --oauth"
    @echo ""
    @echo "Once running, verify with:"
    @echo "  cargo run -p ministr-cli -- cloud check"

# Stop the local cloud-mode dependencies. Volume is preserved so the
# next `dev-cloud-up` keeps your data.
dev-cloud-down:
    docker compose -f docker-compose.dev.yml down

# Stop AND wipe Postgres data. Useful when you want to start clean.
dev-cloud-reset:
    docker compose -f docker-compose.dev.yml down --volumes

# Follow the Postgres log.
dev-cloud-logs:
    docker compose -f docker-compose.dev.yml logs -f postgres

# Open a psql shell against the dev DB.
dev-cloud-psql:
    docker compose -f docker-compose.dev.yml exec postgres \
        psql -U ministr -d ministr_dev

# Smoke-check every wired cloud integration (Postgres, Stripe keys,
# GitHub App key, base URL). Reads the same env vars that `ministr
# serve --transport http --oauth` consumes; prints a tick/cross table.
# Run AFTER `dev-cloud-up` + `source .env.dev`.
dev-cloud-check:
    cargo run -p ministr-cli -- cloud check

# Run the full local cloud demo end-to-end: Postgres up → seed sample
# corpus → start serve → mint bearer → `ministr cloud demo` →
# survey query → cleanup. ~30s after warm cache, ~3min cold (first
# build pulls the embedding model). KEEP=1 leaves the stack running.
demo-local *args:
    ./scripts/demo-local.sh {{args}}

# Watch the deployed Azure cloud index a real repo. Reads the public URL
# from `pulumi -C deploy/azure stack output publicBaseUrl` (override with
# MINISTR_CLOUD_BASE_URL=…), runs `cloud check` against it, then runs
# `cloud demo --clone-url …` so the live SSE progress prints in your
# terminal. Pass CLONE_URL=<repo> to swap the demo repo (defaults to a
# small fixture). No daemon spawn — the deployed cloud is the daemon.
demo-remote *args:
    ./scripts/demo-remote.sh {{args}}

# ── Azure deployment (see DEMO.md §13) ─────────────────────────────────
#
# Typical first run:
#   just azure-init      # one-time: npm ci + pulumi stack init prod
#   just azure-demo      # build+push image, pulumi up, run demo-remote
#
# Day-to-day after code changes:
#   just azure-demo      # rebuilds image with current git sha + re-rolls
#
# Inspection:
#   just azure-status    # stack outputs + /healthz probe
#   just azure-logs      # tail ACA container logs

# One-time Azure setup: npm ci + pulumi stack init prod (idempotent).
azure-init:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v pulumi >/dev/null || { echo "pulumi CLI not found — install from https://www.pulumi.com/docs/install/"; exit 1; }
    command -v az     >/dev/null || { echo "az CLI not found — install from https://learn.microsoft.com/cli/azure/install-azure-cli"; exit 1; }
    command -v docker >/dev/null || { echo "docker not found"; exit 1; }
    eval "$(./scripts/azure-env.sh)"
    cd deploy/azure
    [ -d node_modules ] || npm ci
    pulumi stack ls 2>/dev/null | grep -q '^prod ' || pulumi stack init prod
    echo "✓ azure-init complete — run 'just azure-demo' next"

# Build the linux/amd64 image, tag it with the current git sha, push to
# the stack's ACR, and bump pulumi's `imageTag` config so the next
# `azure-up` creates a fresh ACA revision (ACA only rolls when the tag
# changes — same-tag pushes are ignored).
#
# Build + push current code to ACR; set pulumi imageTag to git sha.
azure-push:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    # Default tag = `<sha>-<unix-ts>`. The unique suffix ensures pulumi
    # sees a config diff on every push — without it, pushing a rebuilt
    # image with the same git sha makes ACA think the revision is
    # already current and skip the roll, even though image *content*
    # changed (e.g. Dockerfile edits, dependency bumps).
    TAG="${TAG:-$(git rev-parse --short HEAD)-$(date +%s)}"
    REGISTRY=$(pulumi -C deploy/azure stack output registryServer 2>/dev/null || true)
    if [ -z "$REGISTRY" ]; then
        # Fall back to the deterministic naming convention from lib/naming.ts.
        # Outputs only commit on a fully successful `pulumi up`, so a partial
        # first apply leaves them empty even when the ACR itself exists.
        PROJ=$(pulumi -C deploy/azure config get projectName 2>/dev/null || echo "ministr")
        REGISTRY="${PROJ}acr.azurecr.io"
        echo "▶ stack output empty — falling back to ${REGISTRY} from projectName=${PROJ}"
    fi
    echo "▶ az acr login --name ${REGISTRY%%.*}"
    az acr login --name "${REGISTRY%%.*}"
    echo "▶ docker buildx build --platform linux/amd64 --push -t ${REGISTRY}/ministr:${TAG} ."
    docker buildx build --platform linux/amd64 --push \
        -t "${REGISTRY}/ministr:${TAG}" .
    echo "▶ pulumi config set imageTag ${TAG}"
    pulumi -C deploy/azure config set imageTag "${TAG}"
    echo "✓ pushed ministr:${TAG} — run 'just azure-up' to roll the revision"

# Run `pulumi up` against the prod stack. First run provisions
# everything (~5-7 min); subsequent runs only diff the changed config.
# PULUMI_FLAGS=--yes skips confirmation.
#
# Run pulumi up against the prod stack.
azure-up:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure up ${PULUMI_FLAGS:-}

# Dry-run pulumi up against the prod stack. Prints the resource diff
# without applying anything. Use this before any large architectural
# change (PHASE6 chunk 4b, etc.) to confirm the planned deletions and
# creations match expectation.
#
# Dry-run pulumi up (no changes applied).
azure-preview:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure preview

# One-shot, idempotent: provision (if needed), push, roll revision,
# then run the full `azure-smoke` (demo-remote → restart-app → repeat)
# so the registry + bundle-restore + streaming-worker path is
# end-to-end exercised. Safe to re-run any time — this is the single
# command for "deploy current code and verify it works."
#
# Full one-shot: pulumi up + push + roll + azure-smoke.
azure-demo:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    REGISTRY=$(pulumi -C deploy/azure stack output registryServer 2>/dev/null || true)
    if [ -z "$REGISTRY" ]; then
        echo "▶ first-time deploy — provisioning Azure resources (~5 min)"
        pulumi -C deploy/azure up --yes
    fi
    just azure-push
    echo ""
    echo "▶ pulumi up to roll the new revision with the fresh image"
    pulumi -C deploy/azure up --yes
    echo ""
    echo "▶ waiting 30s for ACA to roll the revision"
    sleep 30
    echo ""
    just azure-smoke

# Show stack outputs and probe the live /healthz.
azure-status:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure stack output
    URL=$(pulumi -C deploy/azure stack output publicBaseUrl 2>/dev/null || true)
    if [ -n "$URL" ]; then
        echo ""
        echo "▶ GET ${URL}/healthz"
        curl -sS "${URL}/healthz" && echo
    fi

# PHASE6 chunk 4a — sanity-check the Azure OpenAI resource after a
# pulumi up. Prints the endpoint + deployment name from stack outputs
# and probes the deployment with a no-auth request so the operator can
# distinguish "resource exists" from "MI role propagation pending".
# A 401 means the resource is up and the OpenAI account is healthy;
# a connection error means the resource isn't deployed yet.
#
# Sanity-check the cloud's Azure OpenAI resource.
azure-openai-status:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    ENDPOINT=$(pulumi -C deploy/azure stack output openaiEndpoint 2>/dev/null || true)
    DEPLOYMENT=$(pulumi -C deploy/azure stack output openaiDeployment 2>/dev/null || true)
    if [ -z "$ENDPOINT" ] || [ -z "$DEPLOYMENT" ]; then
        echo "✗ OpenAI not provisioned — run 'just azure-up' first."
        exit 1
    fi
    echo "▶ endpoint:   $ENDPOINT"
    echo "▶ deployment: $DEPLOYMENT"
    echo ""
    URL="${ENDPOINT}/openai/deployments/${DEPLOYMENT}/embeddings?api-version=2024-10-21"
    echo "▶ POST $URL (no auth — expect 401)"
    STATUS=$(curl -sS -o /dev/null -w "%{http_code}" \
        -X POST "$URL" \
        -H "Content-Type: application/json" \
        -d '{"input":["healthcheck"]}' || true)
    case "$STATUS" in
      401) echo "✓ 401 — resource reachable; MI role grant (or API key) determines whether real calls succeed" ;;
      404) echo "✗ 404 — deployment not found; pulumi up may still be provisioning the model" ;;
      000) echo "✗ no response — endpoint unreachable; check Pulumi state" ;;
      *)   echo "? HTTP $STATUS — unexpected; inspect manually" ;;
    esac

# PHASE6 chunk 4a — bootstrap path for the first-deploy MI propagation
# lag. The Cognitive Services User role assignment can take a couple of
# minutes to fully propagate on first apply; if the WorkerLoop returns
# 403 from /embeddings, this recipe pulls the OpenAI primary key into
# Pulumi config as MINISTR_AZURE_OPENAI_API_KEY so the embedder's
# OpenAiAuth::ApiKey path takes over immediately. Once `just azure-demo`
# completes successfully, run `just azure-openai-revoke-key` to drop
# the bootstrap key and rely on MI alone.
#
# Bootstrap the OpenAI API key into Pulumi config (MI-fallback).
azure-openai-bootstrap-key:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    ACCOUNT=$(az cognitiveservices account list \
        --resource-group "$RG" \
        --query "[?kind=='OpenAI'].name | [0]" -o tsv)
    if [ -z "$ACCOUNT" ]; then
        echo "✗ no OpenAI account in $RG — run 'just azure-up' first."
        exit 1
    fi
    echo "▶ account: $ACCOUNT"
    KEY=$(az cognitiveservices account keys list \
        --resource-group "$RG" --name "$ACCOUNT" \
        --query "key1" -o tsv)
    [ -n "$KEY" ] || { echo "✗ could not read primary key"; exit 1; }
    pulumi -C deploy/azure config set --secret openaiApiKey "$KEY"
    echo "✓ openaiApiKey set as Pulumi secret"
    echo "  next: lib/app.ts must wire MINISTR_AZURE_OPENAI_API_KEY from this secret"
    echo "  (left as a one-line wiring follow-up so the default deploy stays MI-only)"

# Drop the bootstrap API key from Pulumi config; the deployment will
# go back to MI-only auth on the next 'just azure-up'.
#
# Drop the bootstrapped OpenAI API key.
azure-openai-revoke-key:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure config rm openaiApiKey 2>/dev/null || true
    echo "✓ openaiApiKey removed from Pulumi config"

# Tail live ACA container logs (Ctrl-C to stop).
azure-logs:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    APP=$(pulumi -C deploy/azure stack output appName)
    az containerapp logs show \
        --name "$APP" \
        --resource-group "$RG" \
        --follow --tail 100

# The scheduled trigger (PHASE3 chunk 6, cron every 1 min) spawns short
# replicas; `--show-previous` rolls across them so you can see the
# claim_next + ingest + upload sequence from any recent tick.
#
# Open a psql session against the cloud Postgres flex-server. Azure
# blocks the flex-server's TCP port from arbitrary public IPs by
# default, so this temporarily adds a firewall rule for the caller's
# current public IP, runs psql in a one-shot postgres:16 docker
# container, and removes the rule on exit (via `trap`). Pass `-c
# "SQL"` for one-shot queries or no args for interactive REPL.
#
#   just azure-psql -c "SELECT corpus_id, status FROM indexer_jobs;"
#   just azure-psql                                  # interactive
#
# `az postgres flexible-server firewall-rule create/delete` can take
# 30-90s each — the recipe emits ▶ progress markers so it doesn't
# appear hung. `--no-wait` is intentionally NOT used: we need the
# rule active before psql connects, so synchronous is correct.
#
# Reconcile Pulumi state with the role-assignment GUIDs currently
# live in Azure. Background:
#
#   - PHASE3 chunk 6 introduced a deterministic UUID-v5 role-assignment
#     name (see lib/role-assignment.ts). The deterministic GUID is
#     computed from (scope, principalId, roleDefinitionId).
#   - Earlier deploys with auto-generated GUIDs left orphans on the
#     storage account. A partial pulumi up after the deterministic-
#     name change may also have created the resources on Azure but
#     failed to record them in state (the dreaded `RoleAssignmentExists
#     ... ID X` 409). State and reality diverge.
#
# This recipe re-aligns them: looks up the deterministic GUIDs by
# (storage, principal, role) against live Azure, removes whatever
# pulumi *thinks* it owns for those logical resources, then imports
# the live Azure resources under the correct logical names.
#
# After running this, `just azure-up` should diff-clean for the
# app-blob-rw role assignment.
#
# PHASE6 chunk 3 retired the indexer Job + its blob-data role + the
# jobs-operator role; this recipe now only reconciles the queryApp's
# Storage Blob Data Contributor grant. All names are derived from
# pulumi stack outputs — no `ministrv2*` hardcoding.
#
# Re-import role-assignment state from Azure (one-time reconcile).
azure-rbac-reconcile:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    SUB=$(az account show --query id -o tsv)
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    APP=$(pulumi -C deploy/azure stack output appName)
    STORAGE=$(pulumi -C deploy/azure stack output storageAccount)
    STORAGE_ID=$(az storage account show --name "$STORAGE" \
        --resource-group "$RG" --query id -o tsv)
    SBDC="ba92f5b4-2d11-453d-a403-e96b0029c9fe"
    APP_MI=$(az containerapp show --name "$APP" \
        --resource-group "$RG" --query identity.principalId -o tsv)
    echo "▶ app MI: $APP_MI"

    APP_GUID=$(az role assignment list --scope "$STORAGE_ID" \
        --query "[?principalId=='$APP_MI' && contains(roleDefinitionId, '$SBDC')].name | [0]" \
        -o tsv)
    [ -n "$APP_GUID" ] || { echo "no live app role assignment found"; exit 1; }
    echo "▶ live app role assignment: $APP_GUID"

    # Pulumi logical name for the assignment is `<projectName>-app-blob-rw`
    # (see lib/naming.ts::named); derive from the registry to stay
    # portable across project-name changes.
    REGISTRY=$(pulumi -C deploy/azure stack output registryServer)
    PROJECT="${REGISTRY%%acr.azurecr.io}"
    URN="urn:pulumi:prod::ministr-azure::azure-native:authorization:RoleAssignment::${PROJECT}-app-blob-rw"
    echo "▶ pulumi state delete $URN"
    pulumi -C deploy/azure state delete "$URN" --yes 2>/dev/null || true

    APP_RA="/subscriptions/$SUB/resourceGroups/$RG/providers/Microsoft.Storage/storageAccounts/$STORAGE/providers/Microsoft.Authorization/roleAssignments/$APP_GUID"
    echo "▶ pulumi import ${PROJECT}-app-blob-rw"
    pulumi -C deploy/azure import azure-native:authorization:RoleAssignment \
        "${PROJECT}-app-blob-rw" "$APP_RA" --yes --skip-preview

    echo ""
    echo "✓ state reconciled. Re-run 'just azure-up' to verify diff-clean."

# Open a psql session against the cloud Postgres (auto-firewall).
azure-psql *args:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "▶ [1/5] resolving cloud env"
    eval "$(./scripts/azure-env.sh)"
    echo "▶ [2/5] reading pulumi stack outputs"
    PGURL=$(pulumi -C deploy/azure stack output pgConnectionString --show-secrets)
    PGHOST=$(pulumi -C deploy/azure stack output pgHost)
    # Flex server is `<host>.postgres.database.azure.com`; the
    # firewall-rule API wants the short server name.
    PGSERVER="${PGHOST%%.*}"
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    IP=$(curl -sS --max-time 5 https://api.ipify.org)
    [ -n "$IP" ] || { echo "could not resolve public IP"; exit 1; }
    RULE="temp-claude-$(date +%s)"
    echo "▶ [3/5] opening firewall: rule=$RULE ip=$IP server=$PGSERVER (~30-90s)"
    az postgres flexible-server firewall-rule create \
        --resource-group "$RG" \
        --name "$PGSERVER" \
        --rule-name "$RULE" \
        --start-ip-address "$IP" \
        --end-ip-address "$IP" >/dev/null
    trap 'echo "▶ [5/5] removing firewall rule $RULE"; \
          az postgres flexible-server firewall-rule delete \
              --resource-group "'"$RG"'" \
              --name "'"$PGSERVER"'" \
              --rule-name "'"$RULE"'" \
              --yes >/dev/null || true' EXIT
    echo "▶ [4/5] running psql"
    docker run --rm -i postgres:16 psql "$PGURL" {{args}}

# PHASE3 smoke uses this between an initial demo-remote (which leaves
# a bundle in blob + a cloud_corpora row) and a follow-up query, to
# validate that (a) the durable registry survived the restart and
# (b) chunk 5's on-demand restore re-populates pod-local /data on
# first touch.
#
# Restart the serve-pod's currently active revision.
azure-restart-app:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    APP=$(pulumi -C deploy/azure stack output appName)
    REV=$(az containerapp revision list \
        --name "$APP" \
        --resource-group "$RG" \
        --query "[?properties.active].name | [0]" \
        -o tsv)
    [ -n "$REV" ] || { echo "no active revision found"; exit 1; }
    echo "▶ restarting revision ${REV}"
    az containerapp revision restart \
        --name "$APP" \
        --resource-group "$RG" \
        --revision "$REV"
    echo "▶ waiting 15s for the new pod to become ready"
    sleep 15
    just azure-status

# PHASE4 chunk 6 — list state drift between `cloud_corpora` and
# `indexer_jobs`, plus orphan role-assignments on the storage account
# whose principalIds no longer resolve.
#
# Why this matters: PHASE3 surfaced two ways drift accumulates —
# (a) RBAC role-assignment GUIDs orphaned by Pulumi replace operations
# (the `lib/role-assignment.ts` deterministic-GUID fix stops *new*
# drift but not historic), and (b) `cloud_corpora` rows whose worker
# crashed pre-`claimed_at`-reclaim (PHASE4 chunk 2) left no
# `completed` indexer_jobs row behind. Catching both before the
# operator notices via a failed demo run.
#
# Uses the same auto-firewall + docker postgres:16 pattern as
# `azure-psql` so it doesn't depend on the operator's local psql or
# leave a firewall rule behind on Ctrl-C.
#
# State-drift report: orphan cloud_corpora + orphan role-assignments.
azure-orphans:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "▶ [1/4] resolving cloud env"
    eval "$(./scripts/azure-env.sh)"
    PGURL=$(pulumi -C deploy/azure stack output pgConnectionString --show-secrets)
    PGHOST=$(pulumi -C deploy/azure stack output pgHost)
    PGSERVER="${PGHOST%%.*}"
    RG=$(pulumi -C deploy/azure stack output resourceGroup)
    IP=$(curl -sS --max-time 5 https://api.ipify.org)
    [ -n "$IP" ] || { echo "could not resolve public IP"; exit 1; }
    RULE="temp-orphans-$(date +%s)"
    echo "▶ [2/4] opening firewall: rule=$RULE ip=$IP server=$PGSERVER (~30-90s)"
    az postgres flexible-server firewall-rule create \
        --resource-group "$RG" \
        --name "$PGSERVER" \
        --rule-name "$RULE" \
        --start-ip-address "$IP" \
        --end-ip-address "$IP" >/dev/null
    trap 'echo "▶ removing firewall rule $RULE"; \
          az postgres flexible-server firewall-rule delete \
              --resource-group "'"$RG"'" \
              --name "'"$PGSERVER"'" \
              --rule-name "'"$RULE"'" \
              --yes >/dev/null || true' EXIT
    echo "▶ [3/4] corpora with no completed indexer_jobs row:"
    docker run --rm -i postgres:16 psql "$PGURL" -A -F $'\t' -c "
        SELECT c.corpus_id,
               c.display_name,
               c.status                                              AS corpus_status,
               COALESCE(
                   (SELECT j.status FROM indexer_jobs j
                      WHERE j.corpus_id = c.corpus_id
                      ORDER BY j.created_at DESC LIMIT 1),
                   '(no job rows)'
               )                                                     AS latest_job_status,
               to_timestamp(c.created_at / 1000.0)::timestamptz       AS created
          FROM cloud_corpora c
         WHERE NOT EXISTS (
                   SELECT 1 FROM indexer_jobs j
                    WHERE j.corpus_id = c.corpus_id
                      AND j.status = 'completed'
               )
         ORDER BY c.created_at DESC;
    " | sed -n '1,/^(/p'
    echo ""
    echo "▶ [4/4] storage role-assignments whose principal no longer resolves:"
    STORAGE=$(pulumi -C deploy/azure stack output storageAccount 2>/dev/null) || {
        echo "(storage account not yet provisioned — skipping role-assignment scan)"
        exit 0
    }
    STORAGE_ID=$(az storage account show --name "$STORAGE" \
        --resource-group "$RG" --query id -o tsv 2>/dev/null) || {
            echo "(storage account not yet provisioned — skipping role-assignment scan)"
            exit 0
        }
    # `az role assignment list` returns one row per assignment scoped at
    # or above the storage account. `principalName` is empty when the
    # principal is deleted; that's the orphan signal we want.
    az role assignment list --scope "$STORAGE_ID" \
        --query "[?principalName==''].{name:name, principalId:principalId, role:roleDefinitionName}" \
        -o table 2>/dev/null \
        || echo "(no orphan role-assignments — RBAC clean)"

# The canonical end-to-end smoke for the deployed Azure stack. Each
# new PHASE extends this in place — `just azure-demo` always calls
# `just azure-smoke` at its tail, so there's only one command to
# remember regardless of which phase you're validating.
#
# Sequence (idempotent — safe to re-run any time):
#   1. demo-remote: clone-url → POST /api/v1/corpora returns pending
#      instantly; progress SSE streams from Postgres while the
#      in-process WorkerLoop (PHASE6 chunk 2) drains the queue and
#      calls Azure OpenAI for embeddings (PHASE6 chunk 1); then a
#      survey query against the corpus succeeds.
#   2. azure-restart-app: drops pod-local /data.
#   3. demo-remote again: same CLONE_URL → deterministic corpus_id
#      hits the existing cloud_corpora row (PHASE3 chunk 1), the
#      survey query lazy-downloads the bundle from blob (PHASE3
#      chunk 5) and succeeds — proving end-to-end durability across
#      pod recycle.
#
# What this validates (as of PHASE6):
#   - PHASE3: corpus registry, queue-backed SSE, on-demand bundle restore
#   - PHASE5 chunk 2: streaming HNSW persist gate (no "nb point 0" WARNs)
#   - PHASE5 chunk 3: embeddings_done flows to the SSE during ingest
#   - PHASE6: in-process WorkerLoop + Azure OpenAI embedder + MI auth
#
# Extension policy: when you add PHASE N, append the new validations
# here. CLONE_URL is forwarded to demo-remote; defaults to its
# built-in small fixture.
#
# End-to-end smoke for the deployed Azure stack (extend per phase).
azure-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "▶ azure-smoke step 1 / 3 — first demo-remote (clone + index + query)"
    just demo-remote
    echo ""
    echo "▶ azure-smoke step 2 / 3 — restart serve pod (drops pod-local /data)"
    just azure-restart-app
    echo ""
    echo "▶ azure-smoke step 3 / 3 — re-run demo-remote (must lazy-restore from blob)"
    just demo-remote
    echo ""
    echo "✓ azure-smoke passed — registry + streaming worker + bundle restore survived pod recycle"

# Tear down the entire Azure stack (DESTRUCTIVE — asks for confirmation).
azure-down:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    pulumi -C deploy/azure destroy ${PULUMI_FLAGS:-}

# Use this when the Pulumi passphrase is lost or the stack is
# unrecoverably wedged. Asks for confirmation. Run `just azure-demo`
# after this to redeploy. Resource-group name is derived from the
# live stack output when available; falls back to a Pulumi-config
# lookup if the stack is too broken to enumerate outputs.
#
# Full reset: delete the Azure RG + Pulumi state + encrypted secrets.
azure-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    eval "$(./scripts/azure-env.sh)"
    # Pulumi stack output may fail on a wedged stack; fall back to
    # reading the projectName config + lib/naming.ts's convention.
    RG=$(pulumi -C deploy/azure stack output resourceGroup 2>/dev/null || true)
    if [ -z "$RG" ]; then
        PROJECT=$(pulumi -C deploy/azure config get projectName 2>/dev/null || echo "ministr")
        RG="${PROJECT}-rg-prod"
        echo "▶ stack output unreachable — falling back to RG=${RG} from projectName"
    fi
    echo "This will DELETE:"
    echo "  - Azure resource group ${RG} (all resources inside)"
    echo "  - Pulumi stack 'prod' from azblob://pulumi-state"
    echo "  - Encrypted secrets in deploy/azure/Pulumi.prod.yaml"
    echo ""
    read -p "Type 'yes' to confirm: " CONFIRM
    [ "$CONFIRM" = "yes" ] || { echo "aborted"; exit 1; }
    echo ""
    echo "▶ kicking off async RG delete"
    az group delete --name "$RG" --yes --no-wait 2>/dev/null || true
    echo "▶ wiping Pulumi stack state"
    pulumi -C deploy/azure stack rm prod --force --yes 2>/dev/null || true
    echo "▶ clearing encrypted secrets from Pulumi.prod.yaml"
    # Strip the encryptionsalt + githubWebhookSecret lines so the
    # re-init starts fresh. The other config keys have defaults in
    # Pulumi.yaml so losing them is harmless.
    awk '
        /^encryptionsalt:/ {next}
        /githubWebhookSecret:/ {skip=1; next}
        skip && /^    secure:/ {skip=0; next}
        {print}
    ' deploy/azure/Pulumi.prod.yaml > deploy/azure/Pulumi.prod.yaml.tmp
    mv deploy/azure/Pulumi.prod.yaml.tmp deploy/azure/Pulumi.prod.yaml
    echo "▶ re-initialising stack with empty passphrase"
    pulumi -C deploy/azure stack init prod
    echo "▶ setting fresh githubWebhookSecret"
    pulumi -C deploy/azure config set --secret githubWebhookSecret "$(openssl rand -hex 32)"
    echo "▶ waiting for RG delete to finish (can take 5-10 min)"
    az group wait --name ministr-rg-prod --deleted --timeout 900 2>/dev/null || true
    echo ""
    echo "✓ azure-reset complete. Next: just azure-demo"

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
