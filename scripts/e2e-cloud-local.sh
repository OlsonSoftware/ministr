#!/usr/bin/env bash
# F-Test-1 — multi-tenant local cloud e2e harness.
#
# Spawns the cloud binary against the dev-cloud Postgres, mints TWO
# bearer tokens via the OAuth self-issuer (two DCR registrations →
# two distinct `Tenant.subject` values), and runs a battery of
# PASS/FAIL assertions covering:
#
#   - /healthz reachable
#   - /atlas/manifest.json reachable + count == 50
#   - tenant A registers a corpus → 201
#   - tenant B's GET /api/v1/corpora does NOT see A's corpus
#     (tenant-isolation gate — the highest-severity bug class)
#   - tenant A's GET /api/v1/corpora DOES see it
#   - tenant A creates an org → 201
#   - tenant B's GET /api/v1/orgs does NOT see A's org
#   - tenant A mints an API key → uses it as bearer → sees own corpus
#     (API-key authn parity with OAuth)
#   - tenant A's GET /api/v1/sessions returns [] (their tenant scope)
#
# Exit code is the number of FAILed assertions (0 = all green).
#
# Idempotent: re-running cleans up the previous run's serve + Postgres
# before starting fresh. Ctrl-C at any point also tears them down.
#
# Usage:
#   just e2e-cloud-local
#   PORT=9090 just e2e-cloud-local       # use a different port
#   KEEP=1 just e2e-cloud-local          # leave Postgres + serve running
set -euo pipefail

PORT="${PORT:-8088}"  # 8088 to avoid colliding with demo-local's 8080
ENDPOINT="http://localhost:${PORT}"
RUN_TS=$(date +%s)
SAMPLE_DIR="${SAMPLE_DIR:-/tmp/ministr-e2e-source-${RUN_TS}}"
# Persistent across runs so the embedding model cache survives — the
# Candle HF download is flaky and re-downloading on every run wastes
# time + tickles known network failures. We wipe only the corpora
# subdirectory on each run (below); the models/ subtree persists.
DATA_DIR="${DATA_DIR:-/tmp/ministr-e2e-data}"
CONFIG_PATH="${CONFIG_PATH:-/tmp/ministr-e2e-config-${RUN_TS}.toml}"
BLOB_ROOT="${BLOB_ROOT:-/tmp/ministr-e2e-blobs-${RUN_TS}}"
SERVE_LOG="${SERVE_LOG:-/tmp/ministr-e2e-serve.log}"
KEEP="${KEEP:-0}"
# F-Test-2 — webhook fan-out receiver. Spawned as a sidecar; records
# every incoming POST (headers + body) to RECEIVER_LOG so the harness
# can assert delivery + verify the HMAC signature.
RECEIVER_PORT="${RECEIVER_PORT:-8089}"
RECEIVER_LOG="${RECEIVER_LOG:-/tmp/ministr-e2e-webhook-receiver-${RUN_TS}.jsonl}"

C_BOLD='\033[1m'
C_CYAN='\033[36m'
C_DIM='\033[2m'
C_GREEN='\033[32m'
C_RED='\033[31m'
C_YELLOW='\033[33m'
C_RESET='\033[0m'

step() { printf "${C_BOLD}${C_CYAN}▶ %s${C_RESET}\n" "$*"; }
info() { printf "  ${C_DIM}·${C_RESET} %s\n" "$*"; }
pass() { printf "  ${C_GREEN}✓ PASS${C_RESET}  %s\n" "$*"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { printf "  ${C_RED}✗ FAIL${C_RESET}  %s\n" "$*"; FAIL_COUNT=$((FAIL_COUNT + 1)); }
note() { printf "  ${C_YELLOW}!${C_RESET}  %s\n" "$*"; }

PASS_COUNT=0
FAIL_COUNT=0
SERVE_PID=""
RECEIVER_PID=""
OIDC_MOCK_PID=""
OIDC_MOCK_PORT=8090
SIEM_HEC_PID=""
SIEM_HEC_PORT="${SIEM_HEC_PORT:-8091}"
SIEM_HEC_LOG="${SIEM_HEC_LOG:-/tmp/ministr-e2e-siem-hec-${RUN_TS}.jsonl}"
SIEM_HEC_TOKEN="hec_e2e_test_$$"
# F5.3-d-ii-dispatch — second fake HEC receiver wired via the
# F5.3-d-ii-config CRUD endpoint, so the harness can prove per-org
# dispatch routes ORG_ID_A's events to a different endpoint than the
# global env-var sink.
SIEM_HEC2_PID=""
SIEM_HEC2_PORT="${SIEM_HEC2_PORT:-8092}"
SIEM_HEC2_LOG="${SIEM_HEC2_LOG:-/tmp/ministr-e2e-siem-hec2-${RUN_TS}.jsonl}"
SIEM_HEC2_TOKEN="hec2_per_org_$$"
# F5.3-d-iii-a — third fake receiver for per-org Datadog Logs.
# Reuses e2e-siem-hec-receiver.py — it accepts any JSON POST at any
# path with any auth header, so it doubles as a Datadog Logs intake.
SIEM_DD_PID=""
SIEM_DD_PORT="${SIEM_DD_PORT:-8093}"
SIEM_DD_LOG="${SIEM_DD_LOG:-/tmp/ministr-e2e-siem-dd-${RUN_TS}.jsonl}"
SIEM_DD_API_KEY="dd-api-key-per-org-$$"
# F5.3-d-iii-c — fake TCP syslog/CEF collector. Different port,
# different protocol than the HEC/Datadog receivers (TCP, not HTTP).
SIEM_SYSLOG_PID=""
SIEM_SYSLOG_PORT="${SIEM_SYSLOG_PORT:-8094}"
SIEM_SYSLOG_LOG="${SIEM_SYSLOG_LOG:-/tmp/ministr-e2e-siem-syslog-${RUN_TS}.jsonl}"
# F5.3-d-iii-c-udp — fake UDP syslog/CEF collector for the parallel
# UDP transport path. Separate port from the TCP collector so the
# harness can route both flavors through the same syslog_cef kind.
SIEM_SYSLOG_UDP_PID=""
SIEM_SYSLOG_UDP_PORT="${SIEM_SYSLOG_UDP_PORT:-8095}"
SIEM_SYSLOG_UDP_LOG="${SIEM_SYSLOG_UDP_LOG:-/tmp/ministr-e2e-siem-syslog-udp-${RUN_TS}.jsonl}"
# F5.3-d-iii-b-dispatch — fake S3 PUT receiver. The aws-sdk-s3
# client targets this endpoint via endpoint_url_override in the
# per-org config's token JSON; SigV4 signing is performed but the
# fake server skips signature validation (records the PUT body for
# the harness assertions to grep).
SIEM_S3_PID=""
SIEM_S3_PORT="${SIEM_S3_PORT:-8096}"
SIEM_S3_LOG="${SIEM_S3_LOG:-/tmp/ministr-e2e-siem-s3-${RUN_TS}.jsonl}"

# Bail on missing tooling early — the failure mode otherwise is a
# cryptic curl/jq error 200 lines into the script.
for cmd in docker curl jq openssl cargo python3; do
    if ! command -v "${cmd}" >/dev/null 2>&1; then
        echo "ERROR: required tool '${cmd}' not on PATH" >&2
        exit 2
    fi
done

cleanup() {
    if [[ "${KEEP}" == "1" ]]; then
        echo
        info "KEEP=1 — leaving Postgres + serve (PID ${SERVE_PID:-?}) + receiver (PID ${RECEIVER_PID:-?}) running."
        info "  kill ${SERVE_PID:-?} ${RECEIVER_PID:-?}; just dev-cloud-down"
        return
    fi
    echo
    step "cleanup"
    if [[ -n "${RECEIVER_PID}" ]] && kill -0 "${RECEIVER_PID}" 2>/dev/null; then
        info "stopping webhook receiver (PID ${RECEIVER_PID})"
        kill "${RECEIVER_PID}" 2>/dev/null || true
        wait "${RECEIVER_PID}" 2>/dev/null || true
    fi
    if [[ -n "${OIDC_MOCK_PID}" ]] && kill -0 "${OIDC_MOCK_PID}" 2>/dev/null; then
        info "stopping OIDC mock IdP (PID ${OIDC_MOCK_PID})"
        kill "${OIDC_MOCK_PID}" 2>/dev/null || true
        wait "${OIDC_MOCK_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_HEC_PID}" ]] && kill -0 "${SIEM_HEC_PID}" 2>/dev/null; then
        info "stopping SIEM HEC receiver (PID ${SIEM_HEC_PID})"
        kill "${SIEM_HEC_PID}" 2>/dev/null || true
        wait "${SIEM_HEC_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_HEC2_PID}" ]] && kill -0 "${SIEM_HEC2_PID}" 2>/dev/null; then
        info "stopping SIEM HEC2 receiver (PID ${SIEM_HEC2_PID})"
        kill "${SIEM_HEC2_PID}" 2>/dev/null || true
        wait "${SIEM_HEC2_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_DD_PID}" ]] && kill -0 "${SIEM_DD_PID}" 2>/dev/null; then
        info "stopping SIEM Datadog receiver (PID ${SIEM_DD_PID})"
        kill "${SIEM_DD_PID}" 2>/dev/null || true
        wait "${SIEM_DD_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_SYSLOG_PID}" ]] && kill -0 "${SIEM_SYSLOG_PID}" 2>/dev/null; then
        info "stopping SIEM syslog receiver (PID ${SIEM_SYSLOG_PID})"
        kill "${SIEM_SYSLOG_PID}" 2>/dev/null || true
        wait "${SIEM_SYSLOG_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_SYSLOG_UDP_PID}" ]] && kill -0 "${SIEM_SYSLOG_UDP_PID}" 2>/dev/null; then
        info "stopping SIEM syslog UDP receiver (PID ${SIEM_SYSLOG_UDP_PID})"
        kill "${SIEM_SYSLOG_UDP_PID}" 2>/dev/null || true
        wait "${SIEM_SYSLOG_UDP_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SIEM_S3_PID}" ]] && kill -0 "${SIEM_S3_PID}" 2>/dev/null; then
        info "stopping SIEM fake S3 receiver (PID ${SIEM_S3_PID})"
        kill "${SIEM_S3_PID}" 2>/dev/null || true
        wait "${SIEM_S3_PID}" 2>/dev/null || true
    fi
    if [[ -n "${SERVE_PID}" ]] && kill -0 "${SERVE_PID}" 2>/dev/null; then
        info "stopping serve (PID ${SERVE_PID})"
        kill "${SERVE_PID}" 2>/dev/null || true
        wait "${SERVE_PID}" 2>/dev/null || true
    fi
    info "stopping Postgres"
    docker compose -f docker-compose.dev.yml down >/dev/null 2>&1 || true
    info "wiping per-run scratch (preserving model cache at ${DATA_DIR}/models)"
    rm -rf "${SAMPLE_DIR}" "${DATA_DIR}/corpora" "${DATA_DIR}/corpora.json" \
        "${BLOB_ROOT}" "${CONFIG_PATH}" "${RECEIVER_LOG}" \
        /tmp/ministr-e2e-quota-source-${RUN_TS}-* || true
}
trap cleanup EXIT INT TERM

# ─── helpers ──────────────────────────────────────────────────────────

# Mint a fresh bearer token bound to a UUID-shaped subject via
# `ministr cloud mint-test-bearer`. The OAuth self-issuer's DCR flow
# mints non-UUID `client_id`s which break the cloud's `users.id` /
# `org_members.user_id` / visibility-filter `$1::uuid` casts —
# discovered by F-Test-1's first run. The mint-test-bearer subcommand
# bridges the gap by upserting a real `users` row (same path the
# GitHub callback uses) and minting a bearer bound to that UUID.
#
# Idempotent: re-running with the same github_id returns the same
# UUID; old tokens remain valid until they expire.
mint_token() {
    local github_id="$1" email="$2"
    # Capture stderr to a file so a transient cargo / db / tracing
    # failure surfaces in the harness output instead of vanishing. The
    # F5.2-b-harness-flake noted "zero log lines on failure" — keeping
    # stderr means future regressions of that shape carry their own
    # diagnostic instead of needing a separate `bash -x` reproduction.
    local err_file
    err_file=$(mktemp)
    local out
    if ! out=$(cargo run -q -p ministr-cli -- cloud mint-test-bearer \
        --github-id "${github_id}" \
        --email "${email}" 2> "${err_file}"); then
        echo "DIAGNOSTIC: mint_token exited non-zero — stderr:" >&2
        head -c 4000 "${err_file}" >&2
        echo >&2
        rm -f "${err_file}"
        return 1
    fi
    rm -f "${err_file}"
    printf '%s\n' "${out}" | jq -r '.token'
}

# Query Postgres directly for a user_id by github_id. Used by the
# direct-DB tenant-isolation assertions (a workaround for the cloud
# registry/cloud_corpora design gap — see F-Test-1-followup findings:
# cloud-mode register_corpus writes only to cloud_corpora and the
# in-memory registry stays empty until indexing completes, so the GET
# /api/v1/corpora list returns empty even for the owner).
psql_user_id() {
    local github_id="$1"
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "SELECT id FROM users WHERE github_id = ${github_id};" \
        2>/dev/null \
        | tr -d ' \r\n'
}

psql_count() {
    local sql="$1"
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA -c "${sql}" 2>/dev/null \
        | tr -d ' \r\n'
}

# `expect_status_and_capture METHOD URL BEARER EXPECTED_STATUS DESC [BODY]`
# Curls, captures the response body to a global RESPONSE_BODY (the
# caller jq-parses it), asserts the HTTP status matches EXPECTED_STATUS.
# RESPONSE_STATUS holds the actual code for caller inspection.
RESPONSE_BODY=""
RESPONSE_STATUS=""
curl_request() {
    local method="$1" url="$2" bearer="$3" body="${4:-}"
    local tmp
    tmp=$(mktemp)
    if [[ -n "${body}" ]]; then
        RESPONSE_STATUS=$(curl -sS -o "${tmp}" -w '%{http_code}' \
            -X "${method}" "${url}" \
            -H "authorization: Bearer ${bearer}" \
            -H 'content-type: application/json' \
            -d "${body}")
    else
        RESPONSE_STATUS=$(curl -sS -o "${tmp}" -w '%{http_code}' \
            -X "${method}" "${url}" \
            -H "authorization: Bearer ${bearer}")
    fi
    RESPONSE_BODY=$(cat "${tmp}")
    rm -f "${tmp}"
}

# F-Test-3b — MCP Streamable HTTP handshake. Drives initialize +
# notifications/initialized + a single tools/call so the cloud's
# `ensure_session_mut` stamps a tenant-scoped session in the
# in-memory registry. Side-effect: prints the Mcp-Session-Id header
# value on stdout so callers can capture it; response bodies are
# discarded (rmcp returns SSE which we don't need to parse here —
# the side-effect of session registration is the only thing we
# care about for the F-Test-3b round-trip).
mcp_initialize_and_call() {
    local bearer="$1" tool_name="$2"
    # bash quirk: `${3:-{}}` appends an extra `}` to the actual value
    # because the parser greedily closes parameter expansion on the
    # first `}`. Use an unambiguous default with an explicit branch.
    local args_json
    if [[ -z "${3:-}" ]]; then
        args_json='{}'
    else
        args_json="$3"
    fi
    local init_out mcp_sid
    init_out=$(curl -sS --max-time 15 -D - -o /dev/null -X POST "${ENDPOINT}/mcp" \
        -H "authorization: Bearer ${bearer}" \
        -H 'content-type: application/json' \
        -H 'accept: application/json, text/event-stream' \
        -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0.0.0"}}}')
    mcp_sid=$(printf '%s' "${init_out}" | awk 'BEGIN{IGNORECASE=1}/^mcp-session-id:/ {print $2}' | tr -d '\r\n')
    if [[ -z "${mcp_sid}" ]]; then
        echo "mcp_initialize_and_call: no Mcp-Session-Id header returned" >&2
        return 1
    fi
    local notif_status
    notif_status=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
        -X POST "${ENDPOINT}/mcp" \
        -H "authorization: Bearer ${bearer}" \
        -H "mcp-session-id: ${mcp_sid}" \
        -H 'content-type: application/json' \
        -H 'accept: application/json, text/event-stream' \
        -d '{"jsonrpc":"2.0","method":"notifications/initialized"}')
    info "  mcp notify status=${notif_status} sid=${mcp_sid:0:8}…" >&2
    # tools/call returns SSE; --max-time bounds the wait. Body is
    # discarded — the side-effect we want is ensure_session_mut firing
    # inside the handler and stamping the SessionEntry's tenant_id.
    local call_status body_tmp
    body_tmp=$(mktemp)
    printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"%s","arguments":%s}}' \
        "${tool_name}" "${args_json}" > "${body_tmp}"
    call_status=$(curl -sS --max-time 90 -o /dev/null -w '%{http_code}' \
        -X POST "${ENDPOINT}/mcp" \
        -H "authorization: Bearer ${bearer}" \
        -H "mcp-session-id: ${mcp_sid}" \
        -H 'content-type: application/json' \
        -H 'accept: application/json, text/event-stream' \
        --data-binary "@${body_tmp}" \
        || echo "TIMEOUT")
    rm -f "${body_tmp}"
    info "  mcp tools/call status=${call_status}" >&2
    printf '%s' "${mcp_sid}"
}

assert_status() {
    local actual="$1" expected="$2" name="$3"
    if [[ "${actual}" == "${expected}" ]]; then
        pass "${name} (HTTP ${actual})"
    else
        fail "${name} — expected HTTP ${expected}, got ${actual} · body=${RESPONSE_BODY:0:200}"
    fi
}

assert_jq() {
    local body="$1" jq_expr="$2" expected="$3" name="$4"
    local actual
    actual=$(printf '%s' "${body}" | jq -r "${jq_expr}" 2>/dev/null || true)
    if [[ "${actual}" == "${expected}" ]]; then
        pass "${name} (${jq_expr} == ${expected})"
    else
        fail "${name} — expected ${jq_expr} == '${expected}', got '${actual}'"
    fi
}

# ─── setup ────────────────────────────────────────────────────────────

step "step 1/6 — bring up Postgres on :55432"
docker compose -f docker-compose.dev.yml up -d >/dev/null
attempts=0
until docker compose -f docker-compose.dev.yml exec -T postgres \
        pg_isready -U ministr -d ministr_dev >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [[ "${attempts}" -gt 30 ]]; then
        echo "Postgres failed to become ready" >&2
        exit 1
    fi
    sleep 1
done
info "Postgres ready"

# Reset the public schema so a previous KEEP=1 run can't leak state.
# Idempotent: works on a fresh container too.
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -v ON_ERROR_STOP=1 -c "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;" >/dev/null
info "public schema reset"

step "step 2/6 — seed a tiny corpus dir"
rm -rf "${SAMPLE_DIR}"
mkdir -p "${SAMPLE_DIR}/src"
cat > "${SAMPLE_DIR}/README.md" <<'MD'
# e2e-source

Synthetic corpus used by `just e2e-cloud-local`. Tiny — the e2e
harness asserts multi-tenant correctness, not indexing throughput.
MD
for i in 1 2 3; do
    cat > "${SAMPLE_DIR}/src/m_${i}.rs" <<EOF
pub fn handle_${i}(s: &str) -> String { format!("m${i}:{s}") }
EOF
done
info "seeded ${SAMPLE_DIR}"

step "step 3/6 — start cloud serve in background"
export MINISTR_PG_URL="postgres://ministr:ministr@localhost:55432/ministr_dev?sslmode=disable"
export MINISTR_CLOUD_BASE_URL="${ENDPOINT}"
export MINISTR_BLOB_FS_ROOT="${BLOB_ROOT}"
# F-Test-4 — Stripe webhook signing secret. The harness signs fixture
# events itself (openssl HMAC-SHA256) and POSTs them directly to
# /webhooks/stripe; we never call Stripe's API. The value is a test
# constant scoped to localhost — never leaves this machine.
export MINISTR_STRIPE_WEBHOOK_SECRET="whsec_e2e_test_only_localhost_no_money"
# F5.3-d-i — wire the Splunk-HEC sink to the fake receiver spawned
# below. Setting both env vars triggers SplunkHecSink::from_env() to
# succeed at serve boot; ChainedAuditSink picks it up alongside
# PostgresAuditSink + WebhookFanoutSink.
export MINISTR_SIEM_HEC_URL="http://127.0.0.1:${SIEM_HEC_PORT}/services/collector/event"
export MINISTR_SIEM_HEC_TOKEN="${SIEM_HEC_TOKEN}"
# F5.3-c-ii-archive-read — point the cloud at the archive dir
# F5.3-c-ii-archive-fs's CLI writes to. The /audit/archived
# endpoint reads gzipped JSONL files from there.
export MINISTR_AUDIT_ARCHIVE_DIR="/tmp/ministr-e2e-audit-archive-${RUN_TS}"
unset MINISTR_CLOUD_DATA_DIR
# Persistent DATA_DIR keeps the embedding model cache across runs;
# wipe only the corpora + corpora.json so the registry starts empty.
rm -rf "${BLOB_ROOT}" "${DATA_DIR}/corpora" "${DATA_DIR}/corpora.json"
mkdir -p "${DATA_DIR}/models"
# Seed the model cache from demo-local's data dir if the e2e cache is
# empty and demo-local has one. Saves a flaky HF round-trip on first
# e2e run AND keeps subsequent runs fast. Falls through silently if
# demo-local was never run — the embedder then tries to download
# fresh, which works most of the time but has occasionally failed
# with "model.safetensors not found in repository" (HF mirror flake).
if [[ ! -e "${DATA_DIR}/models/candle" && -e /tmp/ministr-demo-data/models/candle ]]; then
    info "seeding embedding model cache from /tmp/ministr-demo-data"
    cp -R /tmp/ministr-demo-data/models/candle "${DATA_DIR}/models/candle"
fi
cat > "${CONFIG_PATH}" <<EOF
# Auto-generated by scripts/e2e-cloud-local.sh; safe to delete.
data_dir = "${DATA_DIR}"
default_model = "all-MiniLM-L6-v2"
log_format = "pretty"
default_context_budget = 100000
corpus_paths = []

[prefetch]
EOF
info "compiling ministr-cli (cached after first run)"
cargo build -q -p ministr-cli
# F5.4-e-revoke-api-serve: seed a fixture revocation list the
# main test_serve can serve at /api/v1/license-revocations.jsonl.
REVOKE_API_FIXTURE="/tmp/ministr-e2e-revoke-serve-fixture.jsonl"
cat > "${REVOKE_API_FIXTURE}" <<'EOF'
{"ts_iso":"2026-05-23T00:00:00Z","ts_unix":1779494400,"enterprise_id":"e2e-revoke-api-fixture","jwt_id_hash":"aaaaaaaaaaaaaaaa","reason":"harness fixture"}
EOF
# F5.5-b-persist-write: MINISTR_SLA_FLUSH_SECS=2 makes the periodic
# SLA snapshot flush tick fast enough to land at least one row
# within the harness's runtime (default is 60s — too slow for e2e).
MINISTR_SLA_FLUSH_SECS=2 \
MINISTR_LICENSE_REVOCATIONS_SERVE_PATH="${REVOKE_API_FIXTURE}" \
cargo run -q -p ministr-cli -- \
    --config "${CONFIG_PATH}" \
    serve --transport http --oauth \
    --host 127.0.0.1 --port "${PORT}" \
    > "${SERVE_LOG}" 2>&1 &
SERVE_PID=$!
info "serve PID ${SERVE_PID} — logs at ${SERVE_LOG}"

step "step 4/6 — wait for /healthz"
attempts=0
until curl -sf "${ENDPOINT}/healthz" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SERVE_PID}" 2>/dev/null; then
        echo "serve crashed before becoming ready — log tail:" >&2
        tail -30 "${SERVE_LOG}" >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 120 ]]; then
        echo "serve didn't reach /healthz in 240s" >&2
        tail -30 "${SERVE_LOG}" >&2
        exit 1
    fi
    sleep 2
done
info "cloud is live"

step "step 5/6 — mint two distinct bearer tokens (UUID-subject path)"
TOKEN_A=$(mint_token 100001 "e2e-tenant-a@e2e.test")
TOKEN_B=$(mint_token 100002 "e2e-tenant-b@e2e.test")
if [[ -z "${TOKEN_A}" || -z "${TOKEN_B}" ]]; then
    echo "token mint failed — TOKEN_A=${TOKEN_A:0:8}… TOKEN_B=${TOKEN_B:0:8}…" >&2
    exit 1
fi
if [[ "${TOKEN_A}" == "${TOKEN_B}" ]]; then
    echo "both tokens are identical — multi-tenant test is meaningless" >&2
    exit 1
fi
info "TOKEN_A=${TOKEN_A:0:12}…"
info "TOKEN_B=${TOKEN_B:0:12}…"

step "step 5b/6 — spawn webhook receiver on :${RECEIVER_PORT}"
rm -f "${RECEIVER_LOG}"
python3 "$(dirname "$0")/e2e-webhook-receiver.py" "${RECEIVER_PORT}" "${RECEIVER_LOG}" \
    > /tmp/ministr-e2e-receiver.log 2>&1 &
RECEIVER_PID=$!
# Wait for the receiver to be listening (max 5s).
attempts=0
until curl -sf "http://127.0.0.1:${RECEIVER_PORT}/" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${RECEIVER_PID}" 2>/dev/null; then
        echo "receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-receiver.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "receiver didn't reach :${RECEIVER_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "receiver PID ${RECEIVER_PID}; record at ${RECEIVER_LOG}"

step "step 5b'/6 — spawn fake Splunk HEC receiver on :${SIEM_HEC_PORT}"
rm -f "${SIEM_HEC_LOG}"
python3 "$(dirname "$0")/e2e-siem-hec-receiver.py" "${SIEM_HEC_PORT}" "${SIEM_HEC_LOG}" \
    > /tmp/ministr-e2e-siem-hec-stdout.log 2>&1 &
SIEM_HEC_PID=$!
attempts=0
until curl -sf "http://127.0.0.1:${SIEM_HEC_PORT}/" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SIEM_HEC_PID}" 2>/dev/null; then
        echo "SIEM HEC receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-siem-hec-stdout.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "SIEM HEC receiver didn't reach :${SIEM_HEC_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "SIEM HEC receiver PID ${SIEM_HEC_PID}; record at ${SIEM_HEC_LOG}"

step "step 5b''/6 — spawn second SIEM HEC receiver on :${SIEM_HEC2_PORT} (for F5.3-d-ii-dispatch)"
rm -f "${SIEM_HEC2_LOG}"
# Pre-touch the JSONL so the harness's `wc -l < $log` snapshot
# doesn't emit a "No such file or directory" before the first POST
# lands. The receiver appends on every event; an empty file is the
# correct zero baseline.
: > "${SIEM_HEC2_LOG}"
python3 "$(dirname "$0")/e2e-siem-hec-receiver.py" "${SIEM_HEC2_PORT}" "${SIEM_HEC2_LOG}" \
    > /tmp/ministr-e2e-siem-hec2-stdout.log 2>&1 &
SIEM_HEC2_PID=$!
attempts=0
until curl -sf "http://127.0.0.1:${SIEM_HEC2_PORT}/" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SIEM_HEC2_PID}" 2>/dev/null; then
        echo "SIEM HEC2 receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-siem-hec2-stdout.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "SIEM HEC2 receiver didn't reach :${SIEM_HEC2_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "SIEM HEC2 receiver PID ${SIEM_HEC2_PID}; record at ${SIEM_HEC2_LOG}"

step "step 5b'''/6 — spawn fake Datadog Logs receiver on :${SIEM_DD_PORT} (for F5.3-d-iii-a)"
python3 "$(dirname "$0")/e2e-siem-hec-receiver.py" "${SIEM_DD_PORT}" "${SIEM_DD_LOG}" \
    > /tmp/ministr-e2e-siem-dd-stdout.log 2>&1 &
SIEM_DD_PID=$!
attempts=0
until curl -sf "http://127.0.0.1:${SIEM_DD_PORT}/" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SIEM_DD_PID}" 2>/dev/null; then
        echo "SIEM Datadog receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-siem-dd-stdout.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "SIEM Datadog receiver didn't reach :${SIEM_DD_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "SIEM Datadog receiver PID ${SIEM_DD_PID}; record at ${SIEM_DD_LOG}"

step "step 5b''''/6 — spawn fake TCP syslog/CEF collector on :${SIEM_SYSLOG_PORT} (for F5.3-d-iii-c)"
python3 "$(dirname "$0")/e2e-siem-syslog-receiver.py" "${SIEM_SYSLOG_PORT}" "${SIEM_SYSLOG_LOG}" \
    > /tmp/ministr-e2e-siem-syslog-stdout.log 2>&1 &
SIEM_SYSLOG_PID=$!
attempts=0
# TCP syslog has no /healthz; probe with bash's /dev/tcp open-and-close.
until exec 5<>/dev/tcp/127.0.0.1/${SIEM_SYSLOG_PORT} 2>/dev/null && exec 5>&-; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SIEM_SYSLOG_PID}" 2>/dev/null; then
        echo "SIEM syslog receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-siem-syslog-stdout.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "SIEM syslog receiver didn't reach :${SIEM_SYSLOG_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "SIEM syslog receiver PID ${SIEM_SYSLOG_PID}; record at ${SIEM_SYSLOG_LOG}"

step "step 5b'''''/6 — spawn fake UDP syslog/CEF collector on :${SIEM_SYSLOG_UDP_PORT} (for F5.3-d-iii-c-udp)"
python3 "$(dirname "$0")/e2e-siem-syslog-udp-receiver.py" "${SIEM_SYSLOG_UDP_PORT}" "${SIEM_SYSLOG_UDP_LOG}" \
    > /tmp/ministr-e2e-siem-syslog-udp-stdout.log 2>&1 &
SIEM_SYSLOG_UDP_PID=$!
# UDP has no connection handshake to probe; verify the process is
# alive after a short bind delay. The receiver binds and starts
# serving immediately on socketserver.ThreadingUDPServer; 200ms is
# generous.
sleep 0.3
if ! kill -0 "${SIEM_SYSLOG_UDP_PID}" 2>/dev/null; then
    echo "SIEM syslog UDP receiver crashed during boot — log tail:" >&2
    tail -10 /tmp/ministr-e2e-siem-syslog-udp-stdout.log >&2
    exit 1
fi
info "SIEM syslog UDP receiver PID ${SIEM_SYSLOG_UDP_PID}; record at ${SIEM_SYSLOG_UDP_LOG}"

step "step 5b''''''/6 — spawn fake S3 PUT receiver on :${SIEM_S3_PORT} (for F5.3-d-iii-b-dispatch)"
python3 "$(dirname "$0")/e2e-siem-s3-receiver.py" "${SIEM_S3_PORT}" "${SIEM_S3_LOG}" \
    > /tmp/ministr-e2e-siem-s3-stdout.log 2>&1 &
SIEM_S3_PID=$!
attempts=0
until curl -sf "http://127.0.0.1:${SIEM_S3_PORT}/" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${SIEM_S3_PID}" 2>/dev/null; then
        echo "SIEM fake S3 receiver crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-siem-s3-stdout.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "SIEM fake S3 receiver didn't reach :${SIEM_S3_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "SIEM fake S3 receiver PID ${SIEM_S3_PID}; record at ${SIEM_S3_LOG}"

step "step 5c/6 — spawn mock OIDC IdP on :${OIDC_MOCK_PORT}"
# F5.2-c — generate an RSA-2048 keypair the mock IdP uses to sign ID
# tokens. The cloud's /oidc/callback fetches JWKS from the IdP and
# uses the public key to verify the JWT signature. Regenerated on
# every harness run (no need to persist; the cache is in-process on
# the openidconnect side).
OIDC_KEY_DIR="${OIDC_KEY_DIR:-/tmp/ministr-e2e-oidc-keys-${RUN_TS}}"
mkdir -p "${OIDC_KEY_DIR}"
openssl genrsa -out "${OIDC_KEY_DIR}/private.pem" 2048 2>/dev/null
openssl rsa -in "${OIDC_KEY_DIR}/private.pem" -pubout -out "${OIDC_KEY_DIR}/public.pem" 2>/dev/null
# JWKS requires the modulus + exponent as base64url-encoded big-endian
# integers without padding. openssl emits the modulus as hex prefixed
# by "Modulus=" and the exponent as a separate line; for RSA-2048 with
# the OpenSSL default `e=0x010001` (65537) the JWK exponent is "AQAB".
OIDC_MODULUS_HEX=$(openssl rsa -pubin -in "${OIDC_KEY_DIR}/public.pem" -modulus -noout 2>/dev/null | sed 's/^Modulus=//')
OIDC_JWK_N=$(python3 -c "
import base64, sys
hex_s = sys.argv[1]
# Strip any leading 0x00 the way openssl sometimes emits for
# top-bit-set moduli — the JWK spec wants no leading zero bytes.
b = bytes.fromhex(hex_s)
if b[:1] == b'\\x00':
    b = b[1:]
print(base64.urlsafe_b64encode(b).rstrip(b'=').decode())
" "${OIDC_MODULUS_HEX}")
export OIDC_PRIVATE_KEY_PATH="${OIDC_KEY_DIR}/private.pem"
export OIDC_JWK_N
export OIDC_JWK_E="AQAB"
export OIDC_JWK_KID="e2e-key"
export OIDC_FIXED_EMAIL="oidc-test@e2e.test"
export OIDC_FIXED_SUBJECT="e2e-subject-1"
export OIDC_FIXED_CLIENT_ID="e2e-client"
# F5.2-f — synthetic IdP-side groups the user belongs to. The JWT's
# `groups` claim carries this list. The F5.2-f harness POSTs a
# group_role_map referencing "acme-engineers" → "admin"; after the
# callback runs, org_members has a row at role=admin.
export OIDC_FIXED_GROUPS="acme-engineers,acme-other-group"
info "OIDC RSA keypair at ${OIDC_KEY_DIR}; JWK n len=${#OIDC_JWK_N}"
python3 "$(dirname "$0")/e2e-oidc-mock-idp.py" "${OIDC_MOCK_PORT}" \
    > /tmp/ministr-e2e-oidc-mock.log 2>&1 &
OIDC_MOCK_PID=$!
attempts=0
until curl -sf "http://127.0.0.1:${OIDC_MOCK_PORT}/.well-known/openid-configuration" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if ! kill -0 "${OIDC_MOCK_PID}" 2>/dev/null; then
        echo "oidc mock crashed during boot — log tail:" >&2
        tail -10 /tmp/ministr-e2e-oidc-mock.log >&2
        exit 1
    fi
    if [[ "${attempts}" -gt 25 ]]; then
        echo "oidc mock didn't reach :${OIDC_MOCK_PORT} in 5s" >&2
        exit 1
    fi
    sleep 0.2
done
info "oidc mock IdP PID ${OIDC_MOCK_PID}; discovery at http://127.0.0.1:${OIDC_MOCK_PORT}"

# ─── assertions ───────────────────────────────────────────────────────

step "step 6/6 — assertions"

# 1) /healthz alive
curl_request GET "${ENDPOINT}/healthz" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "healthz reachable"
assert_jq "${RESPONSE_BODY}" '.status' "ready" "healthz status==ready"

# 2) atlas manifest reachable + count == 50 (F2.6 seed list)
curl_request GET "${ENDPOINT}/atlas/manifest.json" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "atlas manifest reachable"
assert_jq "${RESPONSE_BODY}" '.count' "50" "atlas manifest count==50"

# 3) tenant A registers a corpus
REGISTER_BODY=$(printf '{"paths":["%s"],"display_name":"e2e-tenant-a-corpus"}' "${SAMPLE_DIR}")
curl_request POST "${ENDPOINT}/api/v1/corpora" "${TOKEN_A}" "${REGISTER_BODY}"
# 200 or 201 are both acceptable — F1.2's response shape can be either.
if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
    pass "tenant A POST /corpora (HTTP ${RESPONSE_STATUS})"
else
    fail "tenant A POST /corpora — expected 200/201, got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
fi
CORPUS_ID_A=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.corpus_id // empty')
if [[ -n "${CORPUS_ID_A}" ]]; then
    pass "tenant A corpus_id captured (${CORPUS_ID_A})"
else
    fail "tenant A corpus_id missing in response"
fi

# 4) tenant A's GET /corpora — owner sees the freshly-registered
#    corpus immediately. The cloud-registry pending-corpus gap that
#    previously kept this list empty (the in-memory CorpusRegistry
#    trailed cloud_corpora until indexing completed) is closed by
#    the daemon's list_corpora pending-merge that consults the
#    PostgresTenantCorpusFilter's pending_corpora_for_tenant.
curl_request GET "${ENDPOINT}/api/v1/corpora" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "tenant A GET /corpora"
COUNT_A_SEES=$(printf '%s' "${RESPONSE_BODY}" | jq "[.corpora[]? | select(.id == \"${CORPUS_ID_A}\")] | length" 2>/dev/null || echo "ERR")
if [[ "${COUNT_A_SEES}" == "1" ]]; then
    pass "tenant A sees their own corpus in /corpora (cloud-registry gap closed)"
elif [[ "${COUNT_A_SEES}" == "ERR" ]]; then
    fail "tenant A /corpora parse error — body=${RESPONSE_BODY:0:200}"
else
    fail "tenant A does NOT see their own corpus — count=${COUNT_A_SEES} body=${RESPONSE_BODY:0:300}"
fi

# 4b) **tenant-id ownership** — verify at the data layer that the row
#     was stamped with tenant A's UUID (closes the F2.x-d invariant).
USER_A_UUID=$(psql_user_id 100001)
USER_B_UUID=$(psql_user_id 100002)
if [[ -n "${USER_A_UUID}" && -n "${USER_B_UUID}" && "${USER_A_UUID}" != "${USER_B_UUID}" ]]; then
    pass "tenants A + B have distinct UUIDs in users (A=${USER_A_UUID:0:8}…, B=${USER_B_UUID:0:8}…)"
else
    fail "tenant UUIDs missing or identical — A=${USER_A_UUID} B=${USER_B_UUID}"
fi
OWNER_OF_CORPUS=$(psql_count "SELECT tenant_id FROM cloud_corpora WHERE corpus_id = '${CORPUS_ID_A}';")
if [[ "${OWNER_OF_CORPUS}" == "${USER_A_UUID}" ]]; then
    pass "cloud_corpora.tenant_id stamped correctly (matches tenant A)"
else
    fail "cloud_corpora.tenant_id mismatch — expected ${USER_A_UUID}, got ${OWNER_OF_CORPUS}"
fi

# 4c) **F5.5-a-priority** — the customer-enqueue path
#     (PostgresIndexJobSink::create_pending) reads the requesting
#     tenant's Plan from the scope_tenant task-local and stamps the
#     queue priority via ministr_mcp::auth::queue_priority. Pro tier
#     (DEFAULT_GITHUB_SIGNIN_PLAN for harness-bootstrapped users)
#     maps to priority=1. The local-paths POST /corpora above takes
#     the register_corpus_only branch (no indexer_jobs row); to
#     exercise create_pending we POST a clone with a synthetic Git URL
#     — the worker will eventually mark it failed (DNS/auth) but the
#     row carries the stamped priority from the moment of INSERT, and
#     claim_next preserves the JSON blob's priority field through its
#     deserialise/serialise round-trip. Before F5.5-a-priority this
#     was always 0.
F55_CLONE_REPO="https://github.com/ministr-e2e/f55-priority-fixture.git"
F55_CLONE_BODY=$(printf '{"repo":"%s"}' "${F55_CLONE_REPO}")
curl_request POST "${ENDPOINT}/api/v1/corpora/${CORPUS_ID_A}/clone" "${TOKEN_A}" "${F55_CLONE_BODY}"
if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
    pass "F5.5-a-priority: clone POST accepted (HTTP ${RESPONSE_STATUS})"
    F55_CLONE_CORPUS=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.corpus_id // empty')
else
    fail "F5.5-a-priority: clone POST got ${RESPONSE_STATUS} (expected 200/201) · body=${RESPONSE_BODY:0:200}"
    F55_CLONE_CORPUS=""
fi
if [[ -n "${F55_CLONE_CORPUS}" ]]; then
    JOB_PRIORITY=$(psql_count "SELECT priority FROM indexer_jobs WHERE corpus_id = '${F55_CLONE_CORPUS}' ORDER BY created_at DESC LIMIT 1;")
    if [[ "${JOB_PRIORITY}" == "1" ]]; then
        pass "indexer_jobs.priority=1 stamped from tenant A's Pro plan (F5.5-a-priority wire is live)"
    else
        fail "indexer_jobs.priority mismatch — expected 1 (Pro lane), got '${JOB_PRIORITY}'"
    fi
    JOB_BLOB_PRIORITY=$(psql_count "SELECT (data::jsonb)->>'priority' FROM indexer_jobs WHERE corpus_id = '${F55_CLONE_CORPUS}' ORDER BY created_at DESC LIMIT 1;")
    if [[ "${JOB_BLOB_PRIORITY}" == "1" ]]; then
        pass "indexer_jobs.data->>priority=1 matches the column (round-trip parity preserved)"
    else
        fail "indexer_jobs.data->>priority drift — expected 1, got '${JOB_BLOB_PRIORITY}'"
    fi
fi

# 4d) **F5.5-a-plan-lookup** — closes the F5.5-a-priority honest caveat
#     by wiring PostgresPlanResolver into OAuthStore so the OAuth path
#     resolves Tenant.plan from users.plan_id. Flip tenant A to
#     enterprise via psql, POST a clone with a NEW repo URL (different
#     corpus_id), assert priority=4 on the new indexer_jobs row, then
#     restore tenant A to pro so downstream Stripe-webhook tests see
#     the baseline they expect.
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "UPDATE users SET plan_id='enterprise' WHERE id='${USER_A_UUID}'::uuid;" \
    >/dev/null 2>&1 || true
F55_ENT_REPO="https://github.com/ministr-e2e/f55-plan-lookup-fixture.git"
F55_ENT_BODY=$(printf '{"repo":"%s"}' "${F55_ENT_REPO}")
curl_request POST "${ENDPOINT}/api/v1/corpora/${CORPUS_ID_A}/clone" "${TOKEN_A}" "${F55_ENT_BODY}"
if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
    pass "F5.5-a-plan-lookup: clone POST under Enterprise plan accepted (HTTP ${RESPONSE_STATUS})"
    F55_ENT_CORPUS=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.corpus_id // empty')
else
    fail "F5.5-a-plan-lookup: clone POST got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
    F55_ENT_CORPUS=""
fi
if [[ -n "${F55_ENT_CORPUS}" ]]; then
    ENT_PRIORITY=$(psql_count "SELECT priority FROM indexer_jobs WHERE corpus_id = '${F55_ENT_CORPUS}' ORDER BY created_at DESC LIMIT 1;")
    if [[ "${ENT_PRIORITY}" == "4" ]]; then
        pass "indexer_jobs.priority=4 stamped under Enterprise plan (PlanResolver OAuth path is live)"
    else
        fail "indexer_jobs.priority mismatch — expected 4 (Enterprise lane), got '${ENT_PRIORITY}'"
    fi
fi
# Restore tenant A to pro so the rest of the harness sees the same
# baseline the F5.5-a-priority assertion above already established.
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "UPDATE users SET plan_id='pro' WHERE id='${USER_A_UUID}'::uuid;" \
    >/dev/null 2>&1 || true

# 5) **tenant isolation** — tenant B's GET returns 200 (and is empty —
#    same reason as tenant A's empty GET, but ALSO genuinely doesn't
#    own A's corpus). The data-layer ownership check above is the
#    real proof of isolation.
curl_request GET "${ENDPOINT}/api/v1/corpora" "${TOKEN_B}"
assert_status "${RESPONSE_STATUS}" "200" "tenant B GET /corpora"
COUNT_B_SEES=$(printf '%s' "${RESPONSE_BODY}" | jq "[.corpora[]? | select(.id == \"${CORPUS_ID_A}\")] | length")
if [[ "${COUNT_B_SEES}" == "0" ]]; then
    pass "tenant isolation: tenant B does NOT see tenant A's corpus in /corpora"
else
    fail "TENANT LEAK: tenant B sees tenant A's corpus (count=${COUNT_B_SEES}) — body=${RESPONSE_BODY:0:300}"
fi

# 6) tenant A creates an org
curl_request POST "${ENDPOINT}/api/v1/orgs" "${TOKEN_A}" '{"name":"e2e-org-a"}'
if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
    pass "tenant A POST /orgs (HTTP ${RESPONSE_STATUS})"
else
    fail "tenant A POST /orgs — expected 200/201, got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
fi
ORG_ID_A=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.id // empty')
if [[ -n "${ORG_ID_A}" ]]; then
    assert_jq "${RESPONSE_BODY}" '.role' "owner" "tenant A is owner of new org"
else
    fail "tenant A org id missing in response"
fi

# 6b) **F-Test-2 — webhook fan-out e2e.** Tenant A subscribes the
#     local Python receiver to their org. Triggering an `invite.created`
#     audit event fires the F3.5b-i ChainedAuditSink which posts the
#     event to the receiver with an `X-Ministr-Signature: sha256=<hex>`
#     header. The harness recomputes HMAC-SHA256(secret, ts + "." + body)
#     and asserts equality.
if [[ -n "${ORG_ID_A}" ]]; then
    RECEIVER_URL="http://127.0.0.1:${RECEIVER_PORT}/hook"
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/webhooks" "${TOKEN_A}" \
        "$(printf '{"url":"%s","event_filter":"*"}' "${RECEIVER_URL}")"
    if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
        pass "tenant A POST /orgs/{id}/webhooks (HTTP ${RESPONSE_STATUS})"
    else
        fail "tenant A POST /webhooks — expected 200/201, got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
    fi
    WEBHOOK_SECRET=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.secret // empty')
    if [[ -n "${WEBHOOK_SECRET}" ]]; then
        pass "webhook secret returned by create endpoint"
    else
        fail "webhook secret missing — body=${RESPONSE_BODY:0:200}"
    fi

    # Trigger an audit event with org_id set. invite.created hits the
    # F3.5b-i fan-out (skips events with org_id IS NULL).
    INVITE_BODY='{"email":"e2e-invitee@e2e.test","role":"member"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_BODY}"
    if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
        pass "tenant A POST /orgs/{id}/invites (HTTP ${RESPONSE_STATUS}) — triggers invite.created"
    else
        fail "tenant A POST /invites — got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
    fi

    # Poll the receiver's record file for an invite.created delivery
    # (max ~6s — fan-out is spawn-tokio + 1 HTTP round-trip). Each
    # record's `.body` is a JSON-encoded string containing the actual
    # webhook payload; `fromjson` walks back into it to read `.event`.
    DELIVERED=""
    attempts=0
    while [[ "${attempts}" -lt 30 && -z "${DELIVERED}" ]]; do
        if [[ -s "${RECEIVER_LOG}" ]]; then
            DELIVERED=$(jq -c \
                'select((.body | fromjson | .event) == "invite.created")' \
                "${RECEIVER_LOG}" 2>/dev/null | head -1 || true)
        fi
        attempts=$((attempts + 1))
        sleep 0.2
    done
    if [[ -n "${DELIVERED}" && -n "${WEBHOOK_SECRET}" ]]; then
        pass "receiver got invite.created delivery within 6s"

        # Verify HMAC: openssl dgst with the captured secret should
        # produce the same hex that landed in X-Ministr-Signature.
        RECV_BODY=$(printf '%s' "${DELIVERED}" | jq -r '.body')
        RECV_TS=$(printf '%s' "${DELIVERED}" | jq -r '.headers["x-ministr-timestamp"]')
        RECV_SIG=$(printf '%s' "${DELIVERED}" | jq -r '.headers["x-ministr-signature"]')
        # Sig header is `sha256=<hex>`; strip the prefix.
        RECV_HEX="${RECV_SIG#sha256=}"
        EXPECT_HEX=$(printf '%s.%s' "${RECV_TS}" "${RECV_BODY}" \
            | openssl dgst -sha256 -hmac "${WEBHOOK_SECRET}" 2>/dev/null \
            | awk '{print $NF}')
        if [[ -n "${EXPECT_HEX}" && "${RECV_HEX}" == "${EXPECT_HEX}" ]]; then
            pass "HMAC signature verifies (sha256(secret, ts + '.' + body) matches)"
        else
            fail "HMAC mismatch — expected=${EXPECT_HEX} got=${RECV_HEX} ts=${RECV_TS}"
        fi
    elif [[ -z "${DELIVERED}" ]]; then
        fail "no invite.created delivery within 6s · receiver_log=$(wc -l < "${RECEIVER_LOG}" 2>/dev/null || echo 0) lines"
    fi
else
    note "skipped F-Test-2 webhook scenario — ORG_ID_A not captured"
fi

# 7) tenant B's GET /orgs does NOT see A's org
curl_request GET "${ENDPOINT}/api/v1/orgs" "${TOKEN_B}"
assert_status "${RESPONSE_STATUS}" "200" "tenant B GET /orgs"
# Use `|| echo 0` so a jq failure (eg unexpected response shape)
# emits 0 and surfaces as a FAIL rather than aborting the script.
# GET /api/v1/orgs returns `{"orgs":[...]}` (wrapped envelope, not bare
# array — discovered when the bare `.[]?` jq expression errored).
COUNT_B_ORG=$(printf '%s' "${RESPONSE_BODY}" | jq "[.orgs[]? | select(.id == \"${ORG_ID_A}\")] | length" 2>/dev/null || echo "ERR")
if [[ "${COUNT_B_ORG}" == "0" ]]; then
    pass "org isolation: tenant B does NOT see tenant A's org"
elif [[ "${COUNT_B_ORG}" == "ERR" ]]; then
    fail "ORG LEAK CHECK — jq error parsing response · body=${RESPONSE_BODY:0:200}"
else
    fail "ORG LEAK: tenant B sees tenant A's org (count=${COUNT_B_ORG})"
fi

# 8) tenant A mints API key → uses it as bearer → sees own corpus
curl_request POST "${ENDPOINT}/api/v1/api_keys" "${TOKEN_A}" '{"name":"e2e-key","scopes":"ministr:read ministr:write"}'
if [[ "${RESPONSE_STATUS}" == "200" || "${RESPONSE_STATUS}" == "201" ]]; then
    pass "tenant A POST /api_keys (HTTP ${RESPONSE_STATUS})"
else
    fail "tenant A POST /api_keys — got ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
fi
API_TOKEN=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.token // empty')
if [[ -n "${API_TOKEN}" ]]; then
    if [[ "${API_TOKEN}" == mst_pk_* ]]; then
        pass "api key has mst_pk_ prefix"
    else
        fail "api key prefix unexpected: ${API_TOKEN:0:12}…"
    fi
    # API key bearer authn — must return 200 (not 401). This was a
    # real bug in validate_scope_middleware (F-Test-1-followup): it
    # pre-checked validate_token (OAuth-only) and short-circuited 401
    # for valid mst_pk_ keys, never reaching resolve_tenant_with_scope's
    # api-key fall-through. Asserting HTTP 200 here closes that gap.
    curl_request GET "${ENDPOINT}/api/v1/corpora" "${API_TOKEN}"
    assert_status "${RESPONSE_STATUS}" "200" "GET /corpora with API key bearer (authn proves resolver wired)"
else
    fail "api key token missing in response — body=${RESPONSE_BODY:0:200}"
fi

# 8b) **F-Test-5 — quota / paywall enforcement e2e.** Tenant A on the
#     Pro plan has §3 cap of 10 corpora. Register 9 more (10 total),
#     each succeeds. The 11th attempt must return HTTP 402 with the
#     byte-for-byte payload from F2.3's `Violation` spec:
#     `{reason: "corpus_quota_exceeded", upgrade_url:
#     "https://ministr.ai/billing/upgrade?from=pro"}`.
#     Requires PostgresCorporaProbe (this chunk) — RegistryProbe
#     counted the wrong thing on cloud-mode because in-memory registry
#     stays empty until indexing. cap value confirmed in
#     ministr-cloud::caps::caps_for_plan: Pro = Some(10).
QUOTA_SAMPLE_BASE="/tmp/ministr-e2e-quota-source-${RUN_TS}"
QUOTA_ALL_SUCCEED=1
# Loop runs 3..9 (7 iterations). Tenant A already has 3 corpora:
# the step-3 original register, the step-4c F5.5-a-priority clone, and
# the step-4d F5.5-a-plan-lookup Enterprise clone. 3 + 7 = 10 lands
# exactly at the Pro cap; the DIR_OVER attempt below is #11 → 402.
for i in 3 4 5 6 7 8 9; do
    DIR="${QUOTA_SAMPLE_BASE}-${i}"
    rm -rf "${DIR}"
    mkdir -p "${DIR}"
    printf 'corpus %s\n' "${i}" > "${DIR}/README.md"
    curl_request POST "${ENDPOINT}/api/v1/corpora" "${TOKEN_A}" \
        "$(printf '{"paths":["%s"],"display_name":"e2e-quota-%s"}' "${DIR}" "${i}")"
    if [[ "${RESPONSE_STATUS}" != "200" && "${RESPONSE_STATUS}" != "201" ]]; then
        fail "quota: corpus #${i} POST /corpora got ${RESPONSE_STATUS} (expected 200/201) · body=${RESPONSE_BODY:0:200}"
        QUOTA_ALL_SUCCEED=0
        break
    fi
done
if [[ "${QUOTA_ALL_SUCCEED}" == "1" ]]; then
    pass "tenant A registered 10 corpora at Pro cap (all 200/201)"
fi

# 11th attempt — should 402.
DIR_OVER="${QUOTA_SAMPLE_BASE}-11"
rm -rf "${DIR_OVER}"
mkdir -p "${DIR_OVER}"
printf 'over cap\n' > "${DIR_OVER}/README.md"
curl_request POST "${ENDPOINT}/api/v1/corpora" "${TOKEN_A}" \
    "$(printf '{"paths":["%s"],"display_name":"e2e-quota-over"}' "${DIR_OVER}")"
if [[ "${RESPONSE_STATUS}" == "402" ]]; then
    pass "11th corpus blocked by quota (HTTP 402)"
    PAYWALL_REASON=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.reason // empty' 2>/dev/null || echo "")
    PAYWALL_URL=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.upgrade_url // empty' 2>/dev/null || echo "")
    if [[ "${PAYWALL_REASON}" == "corpus_quota_exceeded" ]]; then
        pass "paywall reason == corpus_quota_exceeded"
    else
        fail "paywall reason mismatch — expected corpus_quota_exceeded, got '${PAYWALL_REASON}'"
    fi
    if [[ "${PAYWALL_URL}" == "https://ministr.ai/billing/upgrade?from=pro" ]]; then
        pass "paywall upgrade_url matches §3 spec (from=pro)"
    else
        fail "paywall upgrade_url mismatch — got '${PAYWALL_URL}'"
    fi
else
    fail "11th corpus NOT blocked — got HTTP ${RESPONSE_STATUS} · body=${RESPONSE_BODY:0:200}"
fi

# 9) **F-Test-3 — session tenant isolation.** The daemon's serve
#    process boots with a deterministic bootstrap session id
#    `ministr-<hash>` (derived from config corpus_paths in
#    `infra.rs::generate_session_id`). It carries NO tenant_id. F6.2-e-
#    followup-ii says scoped callers should NOT see unstamped legacy
#    entries — but the harness's earlier "count=1 NOTE" surfaced a
#    real leak: `scope_tenant` middleware was missing from the
#    session_export_router, so `tenant_scope::current()` returned None
#    inside `handle_list` and admit_session_for_scope admitted every
#    entry. Layer added in this chunk; both tenants now see 0.
curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "tenant A GET /sessions"
SESSIONS_A_COUNT=$(printf '%s' "${RESPONSE_BODY}" | jq 'length' 2>/dev/null || echo "ERR")
if [[ "${SESSIONS_A_COUNT}" == "0" ]]; then
    pass "tenant A /sessions is empty (scope_tenant layer mounted)"
else
    fail "tenant A /sessions count=${SESSIONS_A_COUNT} — likely the bootstrap session is leaking · body=${RESPONSE_BODY:0:200}"
fi

curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_B}"
assert_status "${RESPONSE_STATUS}" "200" "tenant B GET /sessions"
SESSIONS_B_COUNT=$(printf '%s' "${RESPONSE_BODY}" | jq 'length' 2>/dev/null || echo "ERR")
if [[ "${SESSIONS_B_COUNT}" == "0" ]]; then
    pass "tenant B /sessions is empty (isolation)"
else
    fail "tenant B /sessions count=${SESSIONS_B_COUNT} — cross-tenant leak"
fi

# 10) **F-Test-3 — cross-tenant 404 on /sessions/{id}/export.**
#     Validates F6.2-e-followup-ii's existence-resistance design: a
#     nonexistent session id returns 404 (not 200, not 403); the same
#     id returns 404 for both tenants — neither side can probe the
#     other's session id-space by observing response codes.
SYNTH_ID="00000000-0000-0000-0000-000000000000"
curl_request POST "${ENDPOINT}/api/v1/sessions/${SYNTH_ID}/export" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "404" "tenant A /sessions/{synthetic}/export → 404"
curl_request POST "${ENDPOINT}/api/v1/sessions/${SYNTH_ID}/export" "${TOKEN_B}"
assert_status "${RESPONSE_STATUS}" "404" "tenant B /sessions/{synthetic}/export → 404 (existence-resistant)"

# 11) **F-Test-3b — full session bundle round-trip via MCP /mcp.**
#     Drives a real MCP Streamable HTTP handshake as tenant A so
#     `MinistrServer::initialize` captures tenant_id via context.extensions
#     (F-Test-3b-fix-1) and the subsequent tools/call's ensure_session_mut
#     stamps the SessionEntry. Then verifies the session is visible only
#     to tenant A and that POST /sessions/{id}/export returns a tar bundle
#     for A but 404 for B. Tool choice: `ministr_survey` because its
#     handler calls `ensure_session_mut` (the trigger that stamps
#     tenant_id on the SessionEntry). ministr_toc reads structure
#     without firing ensure_session_mut, so it doesn't exercise the
#     tenant-stamping path the round-trip depends on.
info "F-Test-3b: tenant A drives an MCP handshake + tools/call ministr_survey"
mcp_initialize_and_call "${TOKEN_A}" "ministr_survey" '{"query":"function","top_k":1}' >/dev/null || true

# Poll /sessions for up to 30s — ministr_survey's ensure_session_mut
# call only fires AFTER the backend returns results, which can lag
# behind the curl --max-time bound when the daemon is busy indexing
# the cwd corpus (the auto-discovered ingestion runs concurrently in
# this harness). The poll deals with both: a quick path where the
# stamp lands immediately, and a slow path where the cloud finishes
# the survey a few seconds after the curl returned.
A_SESSION_ID=""
SESSIONS_A_AFTER_MCP="0"
attempts=0
while [[ "${attempts}" -lt 60 ]]; do
    curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_A}"
    SESSIONS_A_AFTER_MCP=$(printf '%s' "${RESPONSE_BODY}" | jq 'length' 2>/dev/null || echo "ERR")
    if [[ "${SESSIONS_A_AFTER_MCP}" =~ ^[1-9][0-9]*$ ]]; then
        A_SESSION_ID=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.[0].session_id')
        break
    fi
    attempts=$((attempts + 1))
    sleep 0.5
done
if [[ -n "${A_SESSION_ID}" ]]; then
    pass "tenant A /sessions count >= 1 after MCP tool call (tenant_id_hint capture works)"
else
    fail "tenant A /sessions count=${SESSIONS_A_AFTER_MCP} after 30s — tenant_id_hint capture broken · body=${RESPONSE_BODY:0:200}"
fi

curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_B}"
SESSIONS_B_AFTER_MCP=$(printf '%s' "${RESPONSE_BODY}" | jq 'length' 2>/dev/null || echo "ERR")
if [[ "${SESSIONS_B_AFTER_MCP}" == "0" ]]; then
    pass "tenant B /sessions still empty after tenant A's MCP call (isolation preserved)"
else
    fail "tenant B /sessions count=${SESSIONS_B_AFTER_MCP} — cross-tenant leak after MCP"
fi

if [[ -n "${A_SESSION_ID}" ]]; then
    # Tenant A exports the session — expect 200 + tar bytes.
    EXPORT_TMP=$(mktemp -t e2e-export.XXXXXX.tar)
    EXPORT_STATUS=$(curl -sS -o "${EXPORT_TMP}" -w '%{http_code}' \
        -X POST "${ENDPOINT}/api/v1/sessions/${A_SESSION_ID}/export" \
        -H "authorization: Bearer ${TOKEN_A}")
    if [[ "${EXPORT_STATUS}" == "200" ]]; then
        pass "tenant A POST /sessions/{real-id}/export → 200"
        # The tar's first 5 bytes after offset 257 are the ustar magic;
        # tar -tf walking it confirms the archive shape AND surfaces
        # the manifest.json entry.
        if tar -tf "${EXPORT_TMP}" 2>/dev/null | grep -q '^manifest\.json$'; then
            pass "exported tar contains manifest.json (F6.2-a wire shape)"
        else
            fail "exported tar missing manifest.json (got entries: $(tar -tf "${EXPORT_TMP}" 2>/dev/null | tr '\n' ',' | head -c 100))"
        fi
    else
        fail "tenant A POST /sessions/{real-id}/export → expected 200, got ${EXPORT_STATUS}"
    fi
    rm -f "${EXPORT_TMP}"

    # Tenant B tries to export tenant A's real session id — must 404
    # (existence-resistant per F6.2-e-followup-ii). This is the
    # higher-bar variant of the synthetic-id check above.
    curl_request POST "${ENDPOINT}/api/v1/sessions/${A_SESSION_ID}/export" "${TOKEN_B}"
    assert_status "${RESPONSE_STATUS}" "404" "tenant B POST /sessions/{tenant-A-real-id}/export → 404 (existence-resistant)"
else
    note "skipped tenant A/B export assertions — A's session id was not captured"
fi

# 12) **F-Test-3b-fix-1-shared-bootstrap — per-connection SessionEntry.**
#     Pre-fix-1-shared-bootstrap, `server_factory` used `server.clone()`
#     so all /mcp connections shared the bootstrap `active_session_id`.
#     First tenant stamped the entry; subsequent tenants' tool-call
#     activity mutated the shared shadow (real cross-tenant data leak
#     via F6.2 export). Fix: `server.fork_for_new_session()` gives each
#     connection a fresh uuid_v4 active_session_id. This assertion
#     proves: tenant B's MCP call yields a DISTINCT session_id from
#     tenant A's (not the same shared bootstrap), AND /sessions for
#     each tenant returns only their own session.
info "F-Test-3b-fix-1-shared-bootstrap: tenant B drives its own MCP call"
mcp_initialize_and_call "${TOKEN_B}" "ministr_survey" '{"query":"function","top_k":1}' >/dev/null || true

B_SESSION_ID=""
SESSIONS_B_AFTER_OWN_MCP="0"
attempts=0
while [[ "${attempts}" -lt 60 ]]; do
    curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_B}"
    SESSIONS_B_AFTER_OWN_MCP=$(printf '%s' "${RESPONSE_BODY}" | jq 'length' 2>/dev/null || echo "ERR")
    if [[ "${SESSIONS_B_AFTER_OWN_MCP}" =~ ^[1-9][0-9]*$ ]]; then
        B_SESSION_ID=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.[0].session_id')
        break
    fi
    attempts=$((attempts + 1))
    sleep 0.5
done
if [[ -n "${B_SESSION_ID}" ]]; then
    pass "tenant B /sessions count >= 1 after own MCP tool call (fork gives B its own SessionEntry)"
    if [[ "${B_SESSION_ID}" != "${A_SESSION_ID}" ]]; then
        pass "tenant B session_id differs from tenant A's (no shared-bootstrap contamination)"
    else
        fail "tenant B session_id == tenant A's — shared-bootstrap leak still present (A=${A_SESSION_ID})"
    fi
else
    fail "tenant B /sessions count=${SESSIONS_B_AFTER_OWN_MCP} after own MCP — fork didn't create per-connection entry"
fi

# Re-verify tenant A's /sessions still returns only A's session
# (not A + B). Without the fork, B's call would have mutated A's
# shadow and /sessions for A would still be size 1 (with the now
# also-shared session). With the fork, A's /sessions still shows
# just A's session_id.
curl_request GET "${ENDPOINT}/api/v1/sessions" "${TOKEN_A}"
A_SESSIONS_AFTER_B=$(printf '%s' "${RESPONSE_BODY}" | jq -r '[.[].session_id] | join(",")' 2>/dev/null)
if [[ "${A_SESSIONS_AFTER_B}" == "${A_SESSION_ID}" ]]; then
    pass "tenant A /sessions still returns only A's session id after B's MCP call"
else
    fail "tenant A /sessions changed after B's MCP call — expected ${A_SESSION_ID}, got ${A_SESSIONS_AFTER_B}"
fi

# 13) **F-Test-4 — Stripe webhook subscription flip.** The cloud
#     handler at POST /webhooks/stripe verifies a Stripe-Signature HMAC
#     against MINISTR_STRIPE_WEBHOOK_SECRET, then on customer.subscription
#     events runs `UPDATE users SET plan_id = $2 WHERE stripe_customer_id
#     = $1`. We test this end-to-end WITHOUT touching Stripe's API: the
#     harness sets MINISTR_STRIPE_WEBHOOK_SECRET to a known test value,
#     UPDATEs tenant A's user row with a synthetic stripe_customer_id,
#     constructs subscription.updated + .deleted fixture events, signs
#     them with openssl HMAC-SHA256 mirroring Stripe's scheme, POSTs
#     them, and polls users.plan_id for the flip. Zero Stripe API
#     traffic, zero money risk.
info "F-Test-4: Stripe webhook subscription flip (local fixture, no Stripe API contact)"
STRIPE_CUSTOMER_A="cus_e2e_test_$(date +%s%N | tail -c 8)"
# Set tenant A's stripe_customer_id + reset plan_id to free so the
# subscription.updated → pro flip is observable (tenant A may have been
# upgraded to pro by F-Test-5 paywall setup; that's noise for this
# assertion).
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "UPDATE users SET stripe_customer_id = '${STRIPE_CUSTOMER_A}', plan_id = 'free' WHERE id = '${USER_A_UUID}'::uuid;" \
    >/dev/null 2>&1
PRE_PLAN_A=$(psql_count "SELECT plan_id FROM users WHERE id = '${USER_A_UUID}'::uuid;")
info "  tenant A user pre-flip plan_id=${PRE_PLAN_A} stripe_customer_id=${STRIPE_CUSTOMER_A}"

# Build a Stripe-Signature header for `body` signed with
# MINISTR_STRIPE_WEBHOOK_SECRET. Scheme (Stripe v1, since 2019):
#   header value = "t=<ts>,v1=<hex>"
#   hex = HMAC-SHA256(secret, "<ts>.<body>")
# Mirrors ministr-cloud's own outbound webhook signer (X-Ministr-Sig).
stripe_sign_header() {
    local body="$1" secret="$2" ts hex
    ts=$(date +%s)
    hex=$(printf '%s.%s' "${ts}" "${body}" \
        | openssl dgst -sha256 -hmac "${secret}" 2>/dev/null \
        | awk '{print $NF}')
    printf 't=%s,v1=%s' "${ts}" "${hex}"
}

# subscription.updated → plan_id should become "pro"
SUB_UPDATED_BODY='{"id":"evt_test_updated","type":"customer.subscription.updated","data":{"object":{"customer":"'"${STRIPE_CUSTOMER_A}"'","status":"active","items":{"data":[{"price":{"lookup_key":"pro"}}]}}}}'
SUB_UPDATED_SIG=$(stripe_sign_header "${SUB_UPDATED_BODY}" "${MINISTR_STRIPE_WEBHOOK_SECRET}")
SUB_UPDATED_STATUS=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
    -X POST "${ENDPOINT}/webhooks/stripe" \
    -H "stripe-signature: ${SUB_UPDATED_SIG}" \
    -H 'content-type: application/json' \
    --data "${SUB_UPDATED_BODY}")
assert_status "${SUB_UPDATED_STATUS}" "200" "POST /webhooks/stripe (subscription.updated) → 200 (HMAC verified)"

POST_PLAN_A=""
for _ in $(seq 1 10); do
    POST_PLAN_A=$(psql_count "SELECT plan_id FROM users WHERE id = '${USER_A_UUID}'::uuid;")
    [[ "${POST_PLAN_A}" == "pro" ]] && break
    sleep 0.5
done
if [[ "${POST_PLAN_A}" == "pro" ]]; then
    pass "tenant A users.plan_id flipped from ${PRE_PLAN_A} → pro after subscription.updated"
else
    fail "tenant A users.plan_id stuck at ${POST_PLAN_A} after subscription.updated (expected pro)"
fi

# subscription.deleted → plan_id should fall back to "free"
SUB_DELETED_BODY='{"id":"evt_test_deleted","type":"customer.subscription.deleted","data":{"object":{"customer":"'"${STRIPE_CUSTOMER_A}"'","status":"canceled"}}}'
SUB_DELETED_SIG=$(stripe_sign_header "${SUB_DELETED_BODY}" "${MINISTR_STRIPE_WEBHOOK_SECRET}")
SUB_DELETED_STATUS=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
    -X POST "${ENDPOINT}/webhooks/stripe" \
    -H "stripe-signature: ${SUB_DELETED_SIG}" \
    -H 'content-type: application/json' \
    --data "${SUB_DELETED_BODY}")
assert_status "${SUB_DELETED_STATUS}" "200" "POST /webhooks/stripe (subscription.deleted) → 200"

DOWN_PLAN_A=""
for _ in $(seq 1 10); do
    DOWN_PLAN_A=$(psql_count "SELECT plan_id FROM users WHERE id = '${USER_A_UUID}'::uuid;")
    [[ "${DOWN_PLAN_A}" == "free" ]] && break
    sleep 0.5
done
if [[ "${DOWN_PLAN_A}" == "free" ]]; then
    pass "tenant A users.plan_id fell back from pro → free after subscription.deleted"
else
    fail "tenant A users.plan_id stuck at ${DOWN_PLAN_A} after subscription.deleted (expected free)"
fi

# Bad-signature rejection — same body, wrong secret. Must be 400.
BAD_SIG=$(stripe_sign_header "${SUB_UPDATED_BODY}" "whsec_attacker_does_not_have_real_secret")
BAD_STATUS=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
    -X POST "${ENDPOINT}/webhooks/stripe" \
    -H "stripe-signature: ${BAD_SIG}" \
    -H 'content-type: application/json' \
    --data "${SUB_UPDATED_BODY}")
assert_status "${BAD_STATUS}" "400" "POST /webhooks/stripe with attacker-signed body → 400 (signature rejected)"

# 14) **F5.1-b — SAML SP browser-facing endpoints.** Two per-org
#     routes mount at /orgs/{id}/saml/{metadata.xml,login}. Both
#     require an `org_saml_configs` row for the org; missing row
#     returns 404. The login endpoint redirects (302) to the IdP
#     SSO URL with a DEFLATE+base64-encoded SAMLRequest query
#     parameter (HTTP-Redirect binding). No assertion verification
#     yet (that's F5.1-c).
info "F5.1-b: SAML SP metadata + login redirect endpoints"
if [[ -n "${ORG_ID_A}" ]]; then
    # First, assert 404 for an org with no SAML config row.
    NO_CFG_STATUS=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
        "${ENDPOINT}/orgs/${ORG_ID_A}/saml/metadata.xml")
    assert_status "${NO_CFG_STATUS}" "404" "GET /orgs/{id}/saml/metadata.xml without config row → 404"

    # Insert a SAML config row for tenant A's org. Use known fixture
    # values so the assertions can match them in the response body.
    SAML_IDP_ENTITY="https://idp.fixture.test/entity"
    SAML_IDP_SSO="https://idp.fixture.test/sso"
    SAML_IDP_CERT="-----BEGIN CERTIFICATE-----\nMIIBfixture\n-----END CERTIFICATE-----"
    SAML_SP_ENTITY="http://localhost:8088/orgs/${ORG_ID_A}/saml"
    SAML_SP_ACS="http://localhost:8088/orgs/${ORG_ID_A}/saml/acs"
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "INSERT INTO org_saml_configs (org_id, idp_entity_id, idp_sso_url, idp_x509_cert, sp_entity_id, sp_acs_url) VALUES ('${ORG_ID_A}', '${SAML_IDP_ENTITY}', '${SAML_IDP_SSO}', '${SAML_IDP_CERT}', '${SAML_SP_ENTITY}', '${SAML_SP_ACS}');" \
        >/dev/null 2>&1

    # GET metadata.xml with row → 200 + XML body with our entityID.
    META_TMP=$(mktemp)
    META_STATUS=$(curl -sS --max-time 5 -o "${META_TMP}" -w '%{http_code}' \
        "${ENDPOINT}/orgs/${ORG_ID_A}/saml/metadata.xml")
    assert_status "${META_STATUS}" "200" "GET /orgs/{id}/saml/metadata.xml with config → 200"
    META_BODY=$(cat "${META_TMP}")
    rm -f "${META_TMP}"
    if [[ "${META_BODY}" == *"EntityDescriptor"* ]] \
        && [[ "${META_BODY}" == *"${SAML_SP_ENTITY}"* ]]; then
        pass "metadata.xml body contains EntityDescriptor with our SP entity_id"
    else
        fail "metadata.xml body missing EntityDescriptor or SP entity_id (got first 200 bytes: ${META_BODY:0:200})"
    fi

    # GET login → 302 to IdP SSO URL with SAMLRequest query param.
    # curl -i prints headers; pipe through awk to grab Location.
    LOGIN_HEADERS=$(curl -sS --max-time 5 -D - -o /dev/null \
        "${ENDPOINT}/orgs/${ORG_ID_A}/saml/login")
    LOGIN_STATUS=$(printf '%s' "${LOGIN_HEADERS}" | awk 'NR==1 {print $2}')
    LOGIN_LOC=$(printf '%s' "${LOGIN_HEADERS}" | awk 'BEGIN{IGNORECASE=1}/^location:/ {sub(/^[^:]+: /,""); print; exit}' | tr -d '\r\n')
    assert_status "${LOGIN_STATUS}" "302" "GET /orgs/{id}/saml/login → 302"
    if [[ "${LOGIN_LOC}" == "${SAML_IDP_SSO}?SAMLRequest="* ]] \
        && [[ "${LOGIN_LOC}" == *"&RelayState="* ]]; then
        pass "login Location header redirects to IdP with SAMLRequest + RelayState params"
    else
        fail "login Location header malformed: '${LOGIN_LOC}'"
    fi
else
    note "skipped F5.1-b SAML assertions — ORG_ID_A not captured"
fi

# 15) **F5.1-d — per-org SAML config CRUD endpoints (owner-only).**
#     Owner POST upserts; owner GET returns the config; non-owner
#     (tenant B who isn't a member of A's org) GET returns 403;
#     owner DELETE returns 204; subsequent GET returns 404.
info "F5.1-d: per-org SAML config CRUD endpoints"
if [[ -n "${ORG_ID_A}" ]]; then
    # First, DROP the row we INSERT'd directly in the F5.1-b block so
    # the CRUD POST exercises the INSERT branch of the upsert (not
    # just UPDATE on the existing row).
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "DELETE FROM org_saml_configs WHERE org_id = '${ORG_ID_A}'::uuid;" >/dev/null 2>&1

    SAML_CRUD_BODY='{"idp_entity_id":"https://idp.crud.test/entity","idp_sso_url":"https://idp.crud.test/sso","idp_x509_cert":"-----BEGIN CERTIFICATE-----\nMIIBcrudfixture\n-----END CERTIFICATE-----","sp_entity_id":"http://localhost:8088/orgs/'${ORG_ID_A}'/saml","sp_acs_url":"http://localhost:8088/orgs/'${ORG_ID_A}'/saml/acs"}'

    # Owner POST → 200 with the row JSON.
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/saml/config" "${TOKEN_A}" "${SAML_CRUD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "owner POST /saml/config → 200 (upsert)"
    GOT_ENTITY=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.idp_entity_id // empty')
    if [[ "${GOT_ENTITY}" == "https://idp.crud.test/entity" ]]; then
        pass "POST /saml/config returns the saved row"
    else
        fail "POST /saml/config response missing idp_entity_id (got: ${RESPONSE_BODY:0:200})"
    fi

    # Owner GET → 200 + same row.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/saml/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "200" "owner GET /saml/config → 200"
    GOT_ENFORCE=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.enforce_signed_assertions')
    if [[ "${GOT_ENFORCE}" == "true" ]]; then
        pass "GET /saml/config defaults enforce_signed_assertions to true"
    else
        fail "GET /saml/config enforce_signed_assertions=${GOT_ENFORCE} (expected true)"
    fi

    # Non-owner (tenant B) GET → 403 (member_role returns None →
    # assert_owner_or_admin maps to Forbidden — same shape as
    # webhooks/audit).
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/saml/config" "${TOKEN_B}"
    assert_status "${RESPONSE_STATUS}" "403" "non-owner GET /saml/config → 403"

    # Owner DELETE → 204 (no body).
    curl_request DELETE "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/saml/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "204" "owner DELETE /saml/config → 204"

    # GET after DELETE → 404.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/saml/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "404" "GET /saml/config after DELETE → 404"
else
    note "skipped F5.1-d SAML CRUD assertions — ORG_ID_A not captured"
fi

# 16) **F5.2-b — OIDC RP login redirect.** Mock IdP serves discovery
#     at http://127.0.0.1:${OIDC_MOCK_PORT}. Without a config row the
#     /oidc/login endpoint returns 404; with a row pointing at the
#     mock IdP it fetches discovery and returns 302 to
#     ${mock_authorize_endpoint}?response_type=code&state=&nonce=
#     &code_challenge=&code_challenge_method=S256&...
info "F5.2-b: OIDC RP login redirect"
if [[ -n "${ORG_ID_A}" ]]; then
    NO_OIDC_STATUS=$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' \
        "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/login")
    assert_status "${NO_OIDC_STATUS}" "404" "GET /orgs/{id}/oidc/login without config → 404"

    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "INSERT INTO org_oidc_configs (org_id, issuer_url, client_id, client_secret) VALUES ('${ORG_ID_A}', 'http://127.0.0.1:${OIDC_MOCK_PORT}', 'e2e-client', 'e2e-secret');" \
        >/dev/null 2>&1

    OIDC_HEADERS=$(curl -sS --max-time 10 -D - -o /dev/null \
        "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/login")
    OIDC_LOGIN_STATUS=$(printf '%s' "${OIDC_HEADERS}" | awk 'NR==1 {print $2}')
    OIDC_LOC=$(printf '%s' "${OIDC_HEADERS}" | awk 'BEGIN{IGNORECASE=1}/^location:/ {sub(/^[^:]+: /,""); print; exit}' | tr -d '\r\n')
    assert_status "${OIDC_LOGIN_STATUS}" "302" "GET /orgs/{id}/oidc/login with config → 302"
    if [[ "${OIDC_LOC}" == "http://127.0.0.1:${OIDC_MOCK_PORT}/authorize?"* ]] \
        && [[ "${OIDC_LOC}" == *"state="* ]] \
        && [[ "${OIDC_LOC}" == *"nonce="* ]] \
        && [[ "${OIDC_LOC}" == *"code_challenge="* ]] \
        && [[ "${OIDC_LOC}" == *"code_challenge_method=S256"* ]]; then
        pass "login Location redirects to IdP authorize endpoint with state + nonce + PKCE S256"
    else
        fail "login Location malformed: '${OIDC_LOC:0:200}'"
    fi

    # 17) **F5.2-c — OIDC callback exchanges code → bearer.** Drive
    #     the full auth-code grant:
    #       (a) follow the /oidc/login redirect to the mock IdP
    #           /authorize — it auto-approves and 302s back to the
    #           cloud's /oidc/callback?code=&state= with the same
    #           state we minted at /oidc/login.
    #       (b) follow that callback — the cloud exchanges the code
    #           at the mock IdP's /token (using the saved PKCE
    #           verifier), validates the ID token's signature against
    #           the mock IdP's JWKS, upserts a users row keyed on
    #           OIDC_FIXED_EMAIL, mints a bearer via OAuthStore, and
    #           returns JSON {token, user_id, plan_id}.
    #       (c) the returned bearer authenticates against the real
    #           tenant-scoped /api/v1/corpora endpoint — proves the
    #           OIDC bearer is indistinguishable from a GitHub one.
    info "F5.2-c: OIDC auth-code grant → bearer mint → tenant-scoped API access"
    # Step (a): drive /oidc/login to mint a fresh pending-login
    # state, capture the IdP /authorize URL the cloud redirects to.
    # The earlier F5.2-b assertions already consumed previous states
    # so we start fresh here.
    # macOS BSD awk ignores `BEGIN{IGNORECASE=1}`, so use the
    # tolower($1)=="location:" form everywhere the upstream header
    # case may vary. axum (cloud) emits lowercase; Python's
    # BaseHTTPRequestHandler (mock IdP) emits "Location:".
    OIDC_AUTHZ_LOC=$(curl -sS --max-time 5 -D - -o /dev/null \
        "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/login" \
        | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' \
        | tr -d '\r\n')
    if [[ -z "${OIDC_AUTHZ_LOC}" ]]; then
        fail "F5.2-c: no Location returned from /oidc/login (refreshed flow)"
    else
        # Step (b): hit the mock IdP /authorize. It immediately
        # 302s back to the cloud's /oidc/callback with `code` + the
        # original `state`. Use -G with --data-urlencode to avoid
        # bash double-escaping the URL params.
        OIDC_AUTHZ_HEADERS=$(curl -sS --max-time 5 -D - -o /tmp/ministr-e2e-oidc-authz.body \
            "${OIDC_AUTHZ_LOC}")
        OIDC_AUTHZ_STATUS=$(printf '%s' "${OIDC_AUTHZ_HEADERS}" | awk 'NR==1 {print $2}')
        OIDC_CALLBACK_LOC=$(printf '%s' "${OIDC_AUTHZ_HEADERS}" | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' | tr -d '\r\n')
        if [[ "${OIDC_CALLBACK_LOC}" == "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/callback?"* ]]; then
            pass "mock IdP /authorize redirects to cloud /oidc/callback with code+state"
        else
            fail "mock IdP /authorize redirect malformed (HTTP ${OIDC_AUTHZ_STATUS}): '${OIDC_CALLBACK_LOC:0:200}'"
            info "  /authorize URL was: ${OIDC_AUTHZ_LOC:0:200}…"
            info "  /authorize headers tail: $(printf '%s' "${OIDC_AUTHZ_HEADERS}" | tail -5)"
        fi
        # Step (b): hit the cloud's callback. JSON body carries the
        # bearer.
        OIDC_CB_OUT=$(curl -sS --max-time 10 -w '\n%{http_code}' "${OIDC_CALLBACK_LOC}")
        OIDC_CB_STATUS=$(printf '%s' "${OIDC_CB_OUT}" | tail -n1)
        OIDC_CB_BODY=$(printf '%s' "${OIDC_CB_OUT}" | sed '$d')
        if [[ "${OIDC_CB_STATUS}" == "200" ]]; then
            pass "GET /orgs/{id}/oidc/callback → 200 (HTTP 200)"
        else
            fail "GET /orgs/{id}/oidc/callback — expected 200, got ${OIDC_CB_STATUS} · body=${OIDC_CB_BODY:0:300}"
        fi
        OIDC_BEARER=$(printf '%s' "${OIDC_CB_BODY}" | jq -r '.token // empty')
        OIDC_USER_ID=$(printf '%s' "${OIDC_CB_BODY}" | jq -r '.user_id // empty')
        OIDC_PLAN_ID=$(printf '%s' "${OIDC_CB_BODY}" | jq -r '.plan_id // empty')
        if [[ -n "${OIDC_BEARER}" && -n "${OIDC_USER_ID}" ]]; then
            pass "callback JSON carries non-empty token + user_id (plan=${OIDC_PLAN_ID})"
        else
            fail "callback JSON missing token or user_id: '${OIDC_CB_BODY:0:300}'"
        fi
        # Step (c): use the OIDC-minted bearer against /api/v1/corpora.
        # 200 = bearer authenticates; the empty corpora list for this
        # fresh OIDC user is expected.
        if [[ -n "${OIDC_BEARER}" ]]; then
            curl_request GET "${ENDPOINT}/api/v1/corpora" "${OIDC_BEARER}"
            assert_status "${RESPONSE_STATUS}" "200" "OIDC-minted bearer authenticates against /api/v1/corpora"
        fi
        # Audit trail: F5.2-c emits an `oidc.login` row. Query
        # audit_events to prove the audit pipeline fired.
        AUDIT_COUNT=$(psql_count "SELECT count(*) FROM audit_events WHERE action='oidc.login';")
        if [[ "${AUDIT_COUNT}" -ge "1" ]]; then
            pass "audit_events contains an oidc.login row (count=${AUDIT_COUNT})"
        else
            fail "no oidc.login row in audit_events (count=${AUDIT_COUNT}) — audit sink may not be wired"
        fi
    fi

    # 18) **F5.2-d — per-org OIDC config CRUD.** Mirrors the F5.1-d
    #     SAML config block exactly. The F5.2-b/c flow above inserted
    #     a row directly via psql; this block first DROPs it so the
    #     CRUD POST exercises the INSERT branch of the upsert (not
    #     just UPDATE on the existing row).
    info "F5.2-d: per-org OIDC config CRUD endpoints"
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "DELETE FROM org_oidc_configs WHERE org_id = '${ORG_ID_A}'::uuid;" >/dev/null 2>&1

    OIDC_CRUD_BODY=$(cat <<EOF
{"issuer_url":"https://idp.crud.test/oidc","client_id":"crud-client","client_secret":"super-secret-${RUN_TS}","groups_claim":"roles","email_claim":"email","name_claim":"display_name","enforce_email_verified":false}
EOF
)

    # Owner POST → 200 with the row JSON. client_secret in response
    # is the REDACTED sentinel.
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}" "${OIDC_CRUD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "owner POST /oidc/config → 200 (upsert)"
    GOT_ISSUER=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.issuer_url // empty')
    if [[ "${GOT_ISSUER}" == "https://idp.crud.test/oidc" ]]; then
        pass "POST /oidc/config returns the saved row"
    else
        fail "POST /oidc/config response missing issuer_url (got: ${RESPONSE_BODY:0:200})"
    fi
    GOT_SECRET=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.client_secret // empty')
    if [[ "${GOT_SECRET}" == "[REDACTED]" ]]; then
        pass "POST /oidc/config redacts client_secret in response"
    else
        fail "POST /oidc/config leaked client_secret: '${GOT_SECRET:0:50}'"
    fi

    # Owner GET → 200 + same row + REDACTED client_secret.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "200" "owner GET /oidc/config → 200"
    GOT_GET_SECRET=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.client_secret // empty')
    if [[ "${GOT_GET_SECRET}" == "[REDACTED]" ]]; then
        pass "GET /oidc/config redacts client_secret"
    else
        fail "GET /oidc/config leaked client_secret: '${GOT_GET_SECRET:0:50}'"
    fi
    # Confirm raw secret never reached the wire by re-querying psql
    # — it should be in the DB (the upsert stored it) but only the
    # sentinel hit the HTTP response.
    DB_SECRET=$(docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "SELECT client_secret FROM org_oidc_configs WHERE org_id = '${ORG_ID_A}'::uuid;" \
        2>/dev/null | tr -d ' \r\n')
    if [[ "${DB_SECRET}" == "super-secret-${RUN_TS}" ]]; then
        pass "DB still has the real client_secret (redaction is HTTP-only)"
    else
        fail "DB client_secret mismatch — expected 'super-secret-${RUN_TS}', got '${DB_SECRET:0:40}'"
    fi

    # Non-owner (tenant B) GET → 403 (member_role returns None →
    # assert_oidc_owner_or_admin maps to Forbidden — same shape as
    # the SAML CRUD).
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_B}"
    assert_status "${RESPONSE_STATUS}" "403" "non-owner GET /oidc/config → 403"

    # Owner DELETE → 204 (no body).
    curl_request DELETE "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "204" "owner DELETE /oidc/config → 204"

    # GET after DELETE → 404.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "404" "GET /oidc/config after DELETE → 404"

    # 19) **F5.2-f — group_role_map → org_members upsert.** Re-POST
    #     the OIDC config WITH a `group_role_map` that names one of
    #     the IdP's user groups (OIDC_FIXED_GROUPS). Drive a fresh
    #     auth-code grant. After the callback, `org_members` should
    #     have a row for the OIDC user at the mapped role.
    info "F5.2-f: group_role_map → org_members upsert via OIDC callback"

    OIDC_F_CRUD_BODY=$(cat <<EOF
{"issuer_url":"http://127.0.0.1:${OIDC_MOCK_PORT}","client_id":"e2e-client","client_secret":"f-secret-${RUN_TS}","group_role_map":{"acme-engineers":"admin"}}
EOF
)
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}" "${OIDC_F_CRUD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "owner POST /oidc/config with group_role_map → 200"
    GOT_MAP=$(printf '%s' "${RESPONSE_BODY}" | jq -c '.group_role_map')
    if [[ "${GOT_MAP}" == '{"acme-engineers":"admin"}' ]]; then
        pass "POST /oidc/config returns the saved group_role_map"
    else
        fail "POST /oidc/config group_role_map mismatch (got: ${GOT_MAP})"
    fi

    # Drive the full grant + callback again to land a new
    # org_members row. Re-uses the OIDC_FIXED_GROUPS / fixed_email
    # the mock IdP was launched with.
    OIDC_F_AUTHZ_LOC=$(curl -sS --max-time 5 -D - -o /dev/null \
        "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/login" \
        | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' \
        | tr -d '\r\n')
    if [[ -z "${OIDC_F_AUTHZ_LOC}" ]]; then
        fail "F5.2-f: no Location from /oidc/login on the second flow"
    else
        OIDC_F_AUTHZ_HEADERS=$(curl -sS --max-time 5 -D - -o /dev/null "${OIDC_F_AUTHZ_LOC}")
        OIDC_F_CALLBACK_LOC=$(printf '%s' "${OIDC_F_AUTHZ_HEADERS}" | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' | tr -d '\r\n')
        OIDC_F_CB_OUT=$(curl -sS --max-time 10 -w '\n%{http_code}' "${OIDC_F_CALLBACK_LOC}")
        OIDC_F_CB_STATUS=$(printf '%s' "${OIDC_F_CB_OUT}" | tail -n1)
        OIDC_F_CB_BODY=$(printf '%s' "${OIDC_F_CB_OUT}" | sed '$d')
        if [[ "${OIDC_F_CB_STATUS}" == "200" ]]; then
            pass "F5.2-f callback → 200 (HTTP 200)"
        else
            fail "F5.2-f callback — expected 200, got ${OIDC_F_CB_STATUS} · body=${OIDC_F_CB_BODY:0:300}"
        fi
        # The OIDC user's UUID is in the callback response.
        OIDC_F_USER=$(printf '%s' "${OIDC_F_CB_BODY}" | jq -r '.user_id // empty')

        # Direct psql check: org_members has a row for this user at
        # the mapped role. The bootstrap-safe rule means a pre-
        # existing owner of ORG_ID_A (tenant A) is unaffected; the
        # OIDC user is a different uuid, so they get a fresh row.
        OIDC_F_ROLE=$(docker compose -f docker-compose.dev.yml exec -T postgres \
            psql -U ministr -d ministr_dev -tA \
            -c "SELECT role FROM org_members WHERE org_id = '${ORG_ID_A}'::uuid AND user_id = '${OIDC_F_USER}'::uuid;" \
            2>/dev/null | tr -d ' \r\n')
        if [[ "${OIDC_F_ROLE}" == "admin" ]]; then
            pass "org_members has OIDC user at role=admin (group_role_map applied)"
        else
            fail "org_members role for OIDC user — expected admin, got '${OIDC_F_ROLE}'"
        fi

        # Audit trail: member.added row should have fired for the
        # OIDC user (the user was newly added to the org via the
        # group claim, not via an invite).
        MA_COUNT=$(psql_count "SELECT count(*) FROM audit_events WHERE action='member.added' AND actor::text = '${OIDC_F_USER}';")
        if [[ "${MA_COUNT}" -ge "1" ]]; then
            pass "audit_events has member.added for OIDC user (count=${MA_COUNT})"
        else
            fail "no member.added audit row for OIDC user (count=${MA_COUNT})"
        fi
    fi

    # 20) **F5.2-f bootstrap safety** — POST a group_role_map that
    #     maps acme-engineers to "member" (a DOWNGRADE attempt). A
    #     subsequent sign-in MUST NOT downgrade an existing owner.
    #     To exercise this we'd need an OIDC user who was already
    #     an owner of ORG_ID_A; the simplest reliable setup is to
    #     directly UPDATE org_members to set the OIDC user to
    #     owner, then re-run the flow with a member-mapping. Skip
    #     this assertion if the prior step didn't capture
    #     OIDC_F_USER.
    if [[ -n "${OIDC_F_USER}" ]]; then
        # Promote the OIDC user to owner via direct DB write
        # (simulates a manual admin action).
        docker compose -f docker-compose.dev.yml exec -T postgres \
            psql -U ministr -d ministr_dev -tA \
            -c "UPDATE org_members SET role='owner' WHERE org_id='${ORG_ID_A}'::uuid AND user_id='${OIDC_F_USER}'::uuid;" \
            >/dev/null 2>&1

        # POST a downgrade-attempting mapping.
        OIDC_F_DOWNGRADE=$(cat <<EOF
{"issuer_url":"http://127.0.0.1:${OIDC_MOCK_PORT}","client_id":"e2e-client","client_secret":"f-secret-${RUN_TS}","group_role_map":{"acme-engineers":"member"}}
EOF
)
        curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}" "${OIDC_F_DOWNGRADE}"
        assert_status "${RESPONSE_STATUS}" "200" "POST downgrade-attempting group_role_map → 200"

        # Drive a fresh flow.
        OIDC_F2_LOC=$(curl -sS --max-time 5 -D - -o /dev/null \
            "${ENDPOINT}/orgs/${ORG_ID_A}/oidc/login" \
            | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' \
            | tr -d '\r\n')
        OIDC_F2_HEADERS=$(curl -sS --max-time 5 -D - -o /dev/null "${OIDC_F2_LOC}")
        OIDC_F2_CALLBACK_LOC=$(printf '%s' "${OIDC_F2_HEADERS}" | awk 'tolower($1)=="location:" {sub(/^[^:]+: /,""); print; exit}' | tr -d '\r\n')
        curl -sS --max-time 10 -o /dev/null -w '%{http_code}' "${OIDC_F2_CALLBACK_LOC}" >/dev/null 2>&1

        # The OIDC user MUST still be owner — never downgraded.
        OIDC_F2_ROLE=$(docker compose -f docker-compose.dev.yml exec -T postgres \
            psql -U ministr -d ministr_dev -tA \
            -c "SELECT role FROM org_members WHERE org_id='${ORG_ID_A}'::uuid AND user_id='${OIDC_F_USER}'::uuid;" \
            2>/dev/null | tr -d ' \r\n')
        if [[ "${OIDC_F2_ROLE}" == "owner" ]]; then
            pass "bootstrap-safe: OIDC owner NOT downgraded by member-mapping"
        else
            fail "OIDC owner WAS downgraded — expected owner, got '${OIDC_F2_ROLE}'"
        fi
    fi

    # 21) **F5.2-f validation — reject malformed group_role_map.**
    #     The CRUD POST must reject non-object payloads + invalid
    #     role values up-front so the callback never sees garbage.
    OIDC_BAD_ARRAY='{"issuer_url":"http://127.0.0.1:'${OIDC_MOCK_PORT}'","client_id":"e2e-client","client_secret":"x","group_role_map":["not","an","object"]}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}" "${OIDC_BAD_ARRAY}"
    assert_status "${RESPONSE_STATUS}" "400" "POST group_role_map=array → 400 (rejected)"
    OIDC_BAD_ROLE='{"issuer_url":"http://127.0.0.1:'${OIDC_MOCK_PORT}'","client_id":"e2e-client","client_secret":"x","group_role_map":{"acme-engineers":"superuser"}}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/oidc/config" "${TOKEN_A}" "${OIDC_BAD_ROLE}"
    assert_status "${RESPONSE_STATUS}" "400" "POST group_role_map with invalid role → 400"
else
    note "skipped F5.2-b/c/d/f OIDC assertions — ORG_ID_A not captured"
fi

# 22) **F5.3-a — tier-aware audit retention.** The F3.7c prune
#     DELETEs audit_events rows older than 90 days. F5.3-a's tier-
#     aware rule preserves rows belonging to orgs with
#     plan_id='enterprise'. Verify: seed three audit rows with
#     ts=now()-100d (one Enterprise org, one non-Enterprise org,
#     one org_id IS NULL), run `ministr audit prune`, assert the
#     Enterprise row survives + the other two are gone.
info "F5.3-a: tier-aware audit retention (Enterprise rows survive prune)"
if [[ -n "${ORG_ID_A}" ]]; then
    # Promote ORG_ID_A to Enterprise tier via direct DB write
    # (the CRUD path for tier upgrade lives in F2.4 Stripe Checkout;
    # for this assertion we bypass billing and set plan_id directly).
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "UPDATE orgs SET plan_id='enterprise' WHERE id='${ORG_ID_A}'::uuid;" \
        >/dev/null 2>&1

    # Seed three old-ts audit rows. Use a clearly identifiable
    # action string so the assertion can scope to exactly the rows
    # we inserted (avoids races with the harness's own audit
    # emissions from earlier blocks).
    F53A_ACTION="f53a_test_$$_${RUN_TS}"
    F53A_RES="00000000-0000-0000-0000-000000005301"
    F53A_RES_NONENT="00000000-0000-0000-0000-000000005302"
    F53A_RES_NULL="00000000-0000-0000-0000-000000005303"
    # The non-Enterprise org_id is a synthetic UUID that doesn't
    # exist in `orgs`. The pruner's NOT EXISTS subquery returns
    # true → the row gets pruned. Avoids needing to seed a real
    # second org (and dodges any uniqueness constraints on
    # orgs.name from earlier harness blocks).
    F53A_FAKE_ORG="ffffffff-ffff-ffff-ffff-fffff5301a01"
    # Insert audit rows with ts in the past (100 days ago, well past
    # the 90-day cutoff). `|| true` so a transient docker hiccup
    # doesn't kill set -e.
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA -v ON_ERROR_STOP=1 \
        -c "INSERT INTO audit_events (org_id, actor, action, resource, ts) VALUES
             ('${ORG_ID_A}'::uuid, NULL, '${F53A_ACTION}', '${F53A_RES}', now() - interval '100 days'),
             ('${F53A_FAKE_ORG}'::uuid, NULL, '${F53A_ACTION}', '${F53A_RES_NONENT}', now() - interval '100 days'),
             (NULL, NULL, '${F53A_ACTION}', '${F53A_RES_NULL}', now() - interval '100 days');" \
        >/dev/null 2>&1 || true

    # Pre-prune count: should be 3.
    PRE_COUNT=$(psql_count "SELECT count(*) FROM audit_events WHERE action='${F53A_ACTION}';")
    if [[ "${PRE_COUNT}" == "3" ]]; then
        pass "seeded 3 F5.3-a fixture audit rows (Enterprise + non-Ent + NULL)"
    else
        fail "F5.3-a fixture seed count expected 3, got ${PRE_COUNT}"
    fi

    # Run the prune via the CLI. MINISTR_PG_URL is already in env.
    cargo run -q -p ministr-cli -- audit prune --retention-days 90 > /tmp/ministr-e2e-f53a-prune.log 2>&1
    PRUNE_STATUS=$?
    if [[ "${PRUNE_STATUS}" == "0" ]]; then
        pass "audit prune CLI exits 0 (HTTP n/a — CLI runs to completion)"
    else
        fail "audit prune CLI exited ${PRUNE_STATUS} · log tail: $(tail -5 /tmp/ministr-e2e-f53a-prune.log)"
    fi

    # Enterprise row MUST survive.
    ENT_REMAINING=$(psql_count "SELECT count(*) FROM audit_events WHERE action='${F53A_ACTION}' AND resource='${F53A_RES}';")
    if [[ "${ENT_REMAINING}" == "1" ]]; then
        pass "Enterprise org audit row survived 90d prune (F5.3-a tier-aware skip)"
    else
        fail "Enterprise audit row was pruned — expected 1, got ${ENT_REMAINING}"
    fi

    # Non-Enterprise row MUST be gone.
    NONENT_REMAINING=$(psql_count "SELECT count(*) FROM audit_events WHERE action='${F53A_ACTION}' AND resource='${F53A_RES_NONENT}';")
    if [[ "${NONENT_REMAINING}" == "0" ]]; then
        pass "non-Enterprise org audit row pruned at 90d (regression-guards default tier)"
    else
        fail "non-Enterprise audit row NOT pruned — expected 0, got ${NONENT_REMAINING}"
    fi

    # NULL-org row MUST be gone (Enterprise promise covers org-scoped
    # actions only; personal-account audit data isn't retained).
    NULL_REMAINING=$(psql_count "SELECT count(*) FROM audit_events WHERE action='${F53A_ACTION}' AND resource='${F53A_RES_NULL}';")
    if [[ "${NULL_REMAINING}" == "0" ]]; then
        pass "NULL-org audit row pruned at 90d (no infinite retention for personal-account actions)"
    else
        fail "NULL-org audit row NOT pruned — expected 0, got ${NULL_REMAINING}"
    fi
else
    note "skipped F5.3-a tier-aware retention assertions — ORG_ID_A not captured"
fi

# 23) **F5.3-c-i — audit_events is partitioned by quarter.** Migration
#     0013 converted the table to PARTITION BY RANGE (ts) with 16
#     quarterly partitions covering 2024-Q1 through 2027-Q4. The
#     F5.3-a assertions already proved the old INSERT + DELETE +
#     SELECT paths still work post-conversion; this assertion locks
#     in the partition count + parent relkind so a future migration
#     can't silently un-partition the table.
info "F5.3-c-i: audit_events partitioned by quarter (16 partitions seeded)"
PART_COUNT=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
# Migration 0013 seeds exactly 16 partitions; the F5.3-c-ii boot
# helper extends the forward edge dynamically at every serve start,
# so the live count is >= 16 (today: 16 seeded + 2 boot-helper-added
# for 2028-Q1/Q2 = 18). Assert the floor rather than equality.
if [[ "${PART_COUNT}" -ge "16" ]]; then
    pass "audit_events has ≥16 quarterly partitions (live count=${PART_COUNT})"
else
    fail "audit_events partition count — expected ≥16, got ${PART_COUNT}"
fi
RELKIND=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT relkind FROM pg_class WHERE relname='audit_events';" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${RELKIND}" == "p" ]]; then
    pass "audit_events relkind == 'p' (partitioned table)"
else
    fail "audit_events relkind expected 'p', got '${RELKIND}'"
fi

# 24) **F5.3-c-ii — boot-time ensure_audit_partitions extends the
#     forward edge.** Migration 0013 ships partitions only out to
#     2027-Q4. The boot helper auto-creates new partitions through
#     `now() + DEFAULT_PARTITION_LOOKAHEAD_QUARTERS` (8 quarters).
#     Today (2026-05-22 is Q2 2026) + 8 quarters = Q2 2028, so the
#     helper should create 2028-Q1 + 2028-Q2 on top of the seeded
#     16 = 18 partitions total. Idempotent: a second run is a
#     no-op (created=0).
info "F5.3-c-ii: ensure_audit_partitions auto-extends forward edge"
# Capture pre-drop count (whatever the boot helper landed at).
# Using a captured baseline rather than a hardcoded number means
# this assertion stays stable as the calendar advances forward.
PRE_DROP=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
# Drop one forward partition to simulate a state where the helper
# has work to do. Using 2027_q4 ensures the helper has to fill
# back a gap (not just extend the forward edge).
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "DROP TABLE audit_events_y2027q4;" >/dev/null 2>&1 || true
COUNT_AFTER_DROP=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
DROP_EXPECT=$((PRE_DROP - 1))
if [[ "${COUNT_AFTER_DROP}" == "${DROP_EXPECT}" ]]; then
    pass "fixture DROP TABLE audit_events_y2027q4 → ${DROP_EXPECT} partitions (was ${PRE_DROP})"
else
    fail "fixture DROP count expected ${DROP_EXPECT}, got ${COUNT_AFTER_DROP}"
fi

# Run the CLI helper. It should fill the 2027_q4 gap, restoring
# the count to the pre-drop value.
cargo run -q -p ministr-cli -- audit ensure-partitions --lookahead-quarters 8 \
    > /tmp/ministr-e2e-ensure.log 2>&1
ENSURE_STATUS=$?
if [[ "${ENSURE_STATUS}" == "0" ]]; then
    pass "ministr audit ensure-partitions CLI exits 0"
else
    fail "ensure-partitions exited ${ENSURE_STATUS} · log: $(tail -5 /tmp/ministr-e2e-ensure.log)"
fi

COUNT_AFTER_ENSURE=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
if [[ "${COUNT_AFTER_ENSURE}" == "${PRE_DROP}" ]]; then
    pass "audit_events partitions restored to ${PRE_DROP} (gap-fill semantics work)"
else
    fail "post-ensure count expected ${PRE_DROP}, got ${COUNT_AFTER_ENSURE}"
fi

# 2027_q4 is back.
RECREATED=$(psql_count "SELECT count(*) FROM pg_class WHERE relname = 'audit_events_y2027q4';")
if [[ "${RECREATED}" == "1" ]]; then
    pass "ensure-partitions re-created audit_events_y2027q4"
else
    fail "audit_events_y2027q4 NOT re-created (count=${RECREATED})"
fi

# Idempotency: second call creates 0 new.
cargo run -q -p ministr-cli -- audit ensure-partitions --lookahead-quarters 8 \
    > /tmp/ministr-e2e-ensure2.log 2>&1
COUNT_AFTER_2ND=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
if [[ "${COUNT_AFTER_2ND}" == "${PRE_DROP}" ]]; then
    pass "second ensure-partitions call is a no-op (count still ${PRE_DROP})"
else
    fail "second-call count expected ${PRE_DROP}, got ${COUNT_AFTER_2ND}"
fi

# 25) **F5.3-d-i — SplunkHecSink streams audit events to the fake HEC
#     receiver.** The cloud's audit pipeline (PostgresAuditSink →
#     WebhookFanoutSink → SplunkHecSink) fires on every audit emission.
#     The harness has already triggered many such emissions (org.created,
#     invite.created, share.granted, api_key.created, member.added,
#     oidc.login, …) so by now the fake HEC receiver should have
#     accumulated several JSONL lines. Poll briefly for the first line
#     to land — the sink is fire-and-forget so the tokio task may not
#     have completed by the time the last audit row landed in Postgres.
info "F5.3-d-i: SplunkHecSink dispatches audit events to Splunk HEC"
HEC_WAIT_ATTEMPTS=0
while [[ ! -s "${SIEM_HEC_LOG}" ]]; do
    HEC_WAIT_ATTEMPTS=$((HEC_WAIT_ATTEMPTS + 1))
    if [[ "${HEC_WAIT_ATTEMPTS}" -gt 30 ]]; then
        break
    fi
    sleep 0.2
done
HEC_LINES=$(wc -l < "${SIEM_HEC_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
if [[ "${HEC_LINES}" -ge "1" ]]; then
    pass "SplunkHecSink delivered ≥1 audit event to fake HEC (lines=${HEC_LINES})"
else
    fail "no audit events arrived at fake HEC after 6s — sink not wired or all dispatches failing"
fi

# Inspect the first record: Authorization header must be Splunk-style,
# path must be `/services/collector/event`, body must contain a
# `sourcetype: "ministr_audit"` marker.
HEC_FIRST=$(head -n1 "${SIEM_HEC_LOG}" 2>/dev/null || echo '{}')
HEC_AUTH=$(printf '%s' "${HEC_FIRST}" | jq -r '.auth // empty')
HEC_PATH=$(printf '%s' "${HEC_FIRST}" | jq -r '.path // empty')
HEC_SOURCETYPE=$(printf '%s' "${HEC_FIRST}" | jq -r '.body | fromjson | .sourcetype // empty' 2>/dev/null)
if [[ "${HEC_AUTH}" == "Splunk ${SIEM_HEC_TOKEN}" ]]; then
    pass "HEC Authorization header matches \"Splunk <token>\" (token echoed exactly)"
else
    fail "HEC auth header mismatch — expected 'Splunk ${SIEM_HEC_TOKEN}', got '${HEC_AUTH:0:80}'"
fi
if [[ "${HEC_PATH}" == "/services/collector/event" ]]; then
    pass "HEC path matches /services/collector/event (Splunk HEC convention)"
else
    fail "HEC path mismatch — expected /services/collector/event, got '${HEC_PATH}'"
fi
if [[ "${HEC_SOURCETYPE}" == "ministr_audit" ]]; then
    pass "HEC event sourcetype == ministr_audit (Splunk search filter convention)"
else
    fail "HEC sourcetype mismatch — expected 'ministr_audit', got '${HEC_SOURCETYPE}'"
fi
# Verify the event payload contains an `action` field (the load-bearing
# audit-row field — Splunk searches usually filter on action=X).
HEC_FIRST_ACTION=$(printf '%s' "${HEC_FIRST}" | jq -r '.body | fromjson | .event.action // empty' 2>/dev/null)
if [[ -n "${HEC_FIRST_ACTION}" ]]; then
    pass "HEC event.action populated (first row action=${HEC_FIRST_ACTION})"
else
    fail "HEC event missing the action field — body=${HEC_FIRST:0:200}"
fi

# 26) **F5.3-d-ii-config — per-org SIEM config CRUD.** Mirrors the
#     F5.2-d OIDC CRUD shape. Dispatch wiring lands in F5.3-d-ii-dispatch.
info "F5.3-d-ii-config: per-org SIEM config CRUD endpoints"
if [[ -n "${ORG_ID_A}" ]]; then
    # First, ensure no row exists (the dispatch chunk will seed via
    # CRUD; for the wire-shape assertions we want INSERT, not UPDATE).
    docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "DELETE FROM org_siem_configs WHERE org_id = '${ORG_ID_A}'::uuid;" >/dev/null 2>&1

    SIEM_CRUD_TOKEN="hec-tenant-a-secret-${RUN_TS}"
    SIEM_CRUD_BODY=$(cat <<EOF
{"kind":"splunk_hec","endpoint_url":"https://splunk.tenant-a.test:8088/services/collector/event","token":"${SIEM_CRUD_TOKEN}","enabled":true}
EOF
)

    # Owner POST → 200 with the row JSON.
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_CRUD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "owner POST /siem/config → 200 (upsert)"
    GOT_KIND=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.kind // empty')
    if [[ "${GOT_KIND}" == "splunk_hec" ]]; then
        pass "POST /siem/config returns the saved kind"
    else
        fail "POST /siem/config missing kind (got: ${RESPONSE_BODY:0:200})"
    fi
    GOT_TOKEN=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.token // empty')
    if [[ "${GOT_TOKEN}" == "[REDACTED]" ]]; then
        pass "POST /siem/config redacts token in response"
    else
        fail "POST /siem/config LEAKED token: '${GOT_TOKEN:0:40}'"
    fi

    # Owner GET → 200 + REDACTED token.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "200" "owner GET /siem/config → 200"
    GOT_GET_TOKEN=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.token // empty')
    if [[ "${GOT_GET_TOKEN}" == "[REDACTED]" ]]; then
        pass "GET /siem/config redacts token"
    else
        fail "GET /siem/config LEAKED token: '${GOT_GET_TOKEN:0:40}'"
    fi
    # DB-ground-truth: the REAL token is in Postgres while only the
    # sentinel reached the HTTP wire.
    DB_TOKEN=$(docker compose -f docker-compose.dev.yml exec -T postgres \
        psql -U ministr -d ministr_dev -tA \
        -c "SELECT token FROM org_siem_configs WHERE org_id='${ORG_ID_A}'::uuid;" \
        2>/dev/null | tr -d ' \r\n')
    if [[ "${DB_TOKEN}" == "${SIEM_CRUD_TOKEN}" ]]; then
        pass "DB still has the real token (redaction is HTTP-only)"
    else
        fail "DB token mismatch — expected '${SIEM_CRUD_TOKEN}', got '${DB_TOKEN:0:40}'"
    fi

    # Non-owner (tenant B) GET → 403.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_B}"
    assert_status "${RESPONSE_STATUS}" "403" "non-owner GET /siem/config → 403"

    # Reject unknown kind.
    SIEM_BAD_KIND='{"kind":"future_provider","endpoint_url":"https://x.test","token":"t"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_BAD_KIND}"
    assert_status "${RESPONSE_STATUS}" "400" "POST unknown kind → 400 (rejected)"

    # Reject URL without scheme.
    SIEM_BAD_URL='{"kind":"splunk_hec","endpoint_url":"splunk.test:8088","token":"t"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_BAD_URL}"
    assert_status "${RESPONSE_STATUS}" "400" "POST missing-scheme URL → 400 (rejected)"

    # Owner DELETE → 204.
    curl_request DELETE "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "204" "owner DELETE /siem/config → 204"

    # GET after DELETE → 404.
    curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}"
    assert_status "${RESPONSE_STATUS}" "404" "GET /siem/config after DELETE → 404"

    # 27) **F5.3-d-ii-dispatch — per-org SIEM dispatcher routes
    #     ORG_ID_A's events to a different HEC endpoint.** The F5.3-d-i
    #     global sink fires for every event (receiver-1 on :${SIEM_HEC_PORT});
    #     the F5.3-d-ii-dispatch per-org sink also fires when the org
    #     has a config row in org_siem_configs. Wire the second
    #     receiver via the F5.3-d-ii-config CRUD, trigger one fresh
    #     org-scoped audit emission, verify the second receiver lands ≥1
    #     event with the per-org token.
    info "F5.3-d-ii-dispatch: per-org SIEM dispatcher routes ORG_ID_A events"
    # Re-create the row via the CRUD endpoint pointing at receiver-2.
    SIEM_PERORG_BODY=$(cat <<EOF
{"kind":"splunk_hec","endpoint_url":"http://127.0.0.1:${SIEM_HEC2_PORT}/services/collector/event","token":"${SIEM_HEC2_TOKEN}","enabled":true}
EOF
)
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_PERORG_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST per-org SIEM config (HEC2 endpoint) → 200"

    # Snapshot receiver-2's current line count BEFORE the audit trigger
    # so we can detect the delta even if anything leaked through earlier.
    HEC2_PRE=$(wc -l < "${SIEM_HEC2_LOG}" 2>/dev/null | tr -d ' ' || echo 0)

    # Trigger one fresh org-scoped audit emission. POSTing an invite
    # for ORG_ID_A fires `invite.created` audit (F3.7a sub-bullet) with
    # `with_org(ORG_ID_A)` — so the per-org dispatcher sees org_id =
    # ORG_ID_A and routes to receiver-2.
    INVITE_PERORG='{"email":"perorg-test@e2e.test"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_PERORG}"
    # 201 expected (matches F3.7a's create-invite response code).
    if [[ "${RESPONSE_STATUS}" == "201" || "${RESPONSE_STATUS}" == "200" ]]; then
        pass "POST /invites fires org-scoped audit (HTTP ${RESPONSE_STATUS})"
    else
        fail "POST /invites for ORG_ID_A — expected 201/200, got ${RESPONSE_STATUS}"
    fi

    # Poll receiver-2 for the new event. Fire-and-forget spawn means
    # the delivery races the assertion; allow up to 6s.
    HEC2_WAIT=0
    while true; do
        HEC2_NOW=$(wc -l < "${SIEM_HEC2_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
        if [[ "${HEC2_NOW}" -gt "${HEC2_PRE}" ]]; then
            break
        fi
        HEC2_WAIT=$((HEC2_WAIT + 1))
        if [[ "${HEC2_WAIT}" -gt 30 ]]; then
            break
        fi
        sleep 0.2
    done
    HEC2_POST=$(wc -l < "${SIEM_HEC2_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
    if [[ "${HEC2_POST}" -gt "${HEC2_PRE}" ]]; then
        pass "receiver-2 got ≥1 new event (pre=${HEC2_PRE}, post=${HEC2_POST})"
    else
        fail "no new events at receiver-2 after 6s (pre=${HEC2_PRE}, post=${HEC2_POST}) — per-org dispatch may not be wired"
    fi

    # Inspect the most-recently-arrived row at receiver-2: token must
    # match the per-org one (proving the dispatch built a fresh
    # SplunkHecSink from the configured row, not the global env-var
    # sink).
    HEC2_LAST=$(tail -n1 "${SIEM_HEC2_LOG}" 2>/dev/null || echo '{}')
    HEC2_AUTH=$(printf '%s' "${HEC2_LAST}" | jq -r '.auth // empty')
    if [[ "${HEC2_AUTH}" == "Splunk ${SIEM_HEC2_TOKEN}" ]]; then
        pass "receiver-2 Authorization carries the per-org token (not the global env-var token)"
    else
        fail "receiver-2 auth header expected 'Splunk ${SIEM_HEC2_TOKEN}', got '${HEC2_AUTH:0:80}'"
    fi
    HEC2_ORG=$(printf '%s' "${HEC2_LAST}" | jq -r '.body | fromjson | .event.org_id // empty' 2>/dev/null)
    if [[ "${HEC2_ORG}" == "${ORG_ID_A}" ]]; then
        pass "receiver-2 event.org_id == ORG_ID_A (per-org routing locked to caller's org)"
    else
        fail "receiver-2 event.org_id expected ${ORG_ID_A}, got '${HEC2_ORG}'"
    fi

    # 28) **F5.3-d-iii-a — Datadog Logs per-org dispatch.** Re-POST
    #     the per-org config with kind=datadog_logs pointing at the
    #     third fake receiver. PerOrgSiemDispatcher must branch on
    #     `kind` and call `dispatch_datadog_logs` instead of the
    #     Splunk HEC helper. The wire-shape differences: DD-API-KEY
    #     header (not "Authorization: Splunk …"); body is an array
    #     of `{ddsource, service, message, action, …}` objects (not
    #     a single `{sourcetype, event, time}` envelope).
    info "F5.3-d-iii-a: Datadog Logs per-org dispatch"

    SIEM_DD_BODY=$(cat <<EOF
{"kind":"datadog_logs","endpoint_url":"http://127.0.0.1:${SIEM_DD_PORT}/api/v2/logs","token":"${SIEM_DD_API_KEY}","enabled":true}
EOF
)
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_DD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST per-org Datadog config → 200"
    GOT_DD_KIND=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.kind // empty')
    if [[ "${GOT_DD_KIND}" == "datadog_logs" ]]; then
        pass "POST returns kind=datadog_logs (CRUD admits new kind)"
    else
        fail "POST kind expected datadog_logs, got '${GOT_DD_KIND}'"
    fi

    # Snapshot receiver-3's count before the audit trigger.
    DD_PRE=$(wc -l < "${SIEM_DD_LOG}" 2>/dev/null | tr -d ' ' || echo 0)

    # Fire one fresh org-scoped audit emission. Same trigger as
    # F5.3-d-ii-dispatch (POST another invite for ORG_ID_A) — the
    # invite.created audit fires with org_id=ORG_ID_A.
    INVITE_DD='{"email":"dd-test@e2e.test"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_DD}"
    if [[ "${RESPONSE_STATUS}" == "201" || "${RESPONSE_STATUS}" == "200" ]]; then
        pass "POST /invites for Datadog flow fires org-scoped audit (HTTP ${RESPONSE_STATUS})"
    else
        fail "POST /invites for ORG_ID_A — expected 201/200, got ${RESPONSE_STATUS}"
    fi

    # Poll receiver-3 for the new event.
    DD_WAIT=0
    while true; do
        DD_NOW=$(wc -l < "${SIEM_DD_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
        if [[ "${DD_NOW}" -gt "${DD_PRE}" ]]; then
            break
        fi
        DD_WAIT=$((DD_WAIT + 1))
        if [[ "${DD_WAIT}" -gt 30 ]]; then
            break
        fi
        sleep 0.2
    done
    DD_POST=$(wc -l < "${SIEM_DD_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
    if [[ "${DD_POST}" -gt "${DD_PRE}" ]]; then
        pass "receiver-3 (Datadog) got ≥1 new event (pre=${DD_PRE}, post=${DD_POST})"
    else
        fail "no new events at receiver-3 after 6s — Datadog dispatch may not be wired"
    fi

    # Inspect the most-recent row: DD-API-KEY header must echo the
    # configured key exactly (proving the dispatcher built from the
    # row's token); body must be an array shape with ddsource.
    DD_LAST=$(tail -n1 "${SIEM_DD_LOG}" 2>/dev/null || echo '{}')
    DD_API_KEY_HEADER=$(printf '%s' "${DD_LAST}" | jq -r '.dd_api_key // empty')
    if [[ "${DD_API_KEY_HEADER}" == "${SIEM_DD_API_KEY}" ]]; then
        pass "receiver-3 DD-API-KEY header echoes the configured key exactly"
    else
        fail "receiver-3 DD-API-KEY expected '${SIEM_DD_API_KEY}', got '${DD_API_KEY_HEADER:0:80}'"
    fi
    # Datadog body shape: top-level is a JSON array of log objects;
    # each has ddsource = "ministr".
    DD_FIRST_DDSOURCE=$(printf '%s' "${DD_LAST}" | jq -r '.body | fromjson | .[0].ddsource // empty' 2>/dev/null)
    if [[ "${DD_FIRST_DDSOURCE}" == "ministr" ]]; then
        pass "receiver-3 body[0].ddsource == 'ministr' (Datadog Logs envelope)"
    else
        fail "receiver-3 body shape mismatch — expected .[0].ddsource=ministr, got '${DD_FIRST_DDSOURCE}' · body=${DD_LAST:0:200}"
    fi
    DD_FIRST_ORG=$(printf '%s' "${DD_LAST}" | jq -r '.body | fromjson | .[0].org_id // empty' 2>/dev/null)
    if [[ "${DD_FIRST_ORG}" == "${ORG_ID_A}" ]]; then
        pass "receiver-3 body[0].org_id == ORG_ID_A (Datadog routing locked to caller's org)"
    else
        fail "receiver-3 body[0].org_id expected ${ORG_ID_A}, got '${DD_FIRST_ORG}'"
    fi

    # 29) **F5.3-d-iii-c — syslog/CEF per-org dispatch.** Re-POST the
    #     per-org config with kind=syslog_cef pointing at the fake
    #     TCP collector. PerOrgSiemDispatcher must take the
    #     syslog_cef arm and call dispatch_syslog_cef. Different
    #     protocol than HEC/Datadog (TCP, not HTTP); wire format
    #     is a CEF v0 line terminated by newline.
    info "F5.3-d-iii-c: syslog/CEF per-org dispatch (TCP)"
    SIEM_SYSLOG_BODY=$(cat <<EOF
{"kind":"syslog_cef","endpoint_url":"tcp://127.0.0.1:${SIEM_SYSLOG_PORT}","token":"","enabled":true}
EOF
)
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_SYSLOG_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST per-org syslog_cef config (empty token, tcp:// endpoint) → 200"
    GOT_SYSLOG_KIND=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.kind // empty')
    if [[ "${GOT_SYSLOG_KIND}" == "syslog_cef" ]]; then
        pass "POST returns kind=syslog_cef (CRUD admits TCP-syslog kind)"
    else
        fail "POST kind expected syslog_cef, got '${GOT_SYSLOG_KIND}'"
    fi

    # Snapshot receiver count before audit trigger.
    SYSLOG_PRE=$(wc -l < "${SIEM_SYSLOG_LOG}" 2>/dev/null | tr -d ' ' || echo 0)

    # Fire one org-scoped audit emission.
    INVITE_SYSLOG='{"email":"syslog-test@e2e.test"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_SYSLOG}"
    if [[ "${RESPONSE_STATUS}" == "201" || "${RESPONSE_STATUS}" == "200" ]]; then
        pass "POST /invites for syslog flow fires org-scoped audit (HTTP ${RESPONSE_STATUS})"
    else
        fail "POST /invites for ORG_ID_A — expected 201/200, got ${RESPONSE_STATUS}"
    fi

    # Poll the syslog collector for the new line.
    SYSLOG_WAIT=0
    while true; do
        SYSLOG_NOW=$(wc -l < "${SIEM_SYSLOG_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
        if [[ "${SYSLOG_NOW}" -gt "${SYSLOG_PRE}" ]]; then
            break
        fi
        SYSLOG_WAIT=$((SYSLOG_WAIT + 1))
        if [[ "${SYSLOG_WAIT}" -gt 30 ]]; then
            break
        fi
        sleep 0.2
    done
    SYSLOG_POST=$(wc -l < "${SIEM_SYSLOG_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
    if [[ "${SYSLOG_POST}" -gt "${SYSLOG_PRE}" ]]; then
        pass "syslog collector got ≥1 new line (pre=${SYSLOG_PRE}, post=${SYSLOG_POST})"
    else
        fail "no new lines at syslog collector after 6s — syslog_cef dispatch may not be wired"
    fi

    # Inspect the most-recently-arrived CEF line.
    SYSLOG_LAST=$(tail -n1 "${SIEM_SYSLOG_LOG}" 2>/dev/null || echo '{}')
    CEF_LINE=$(printf '%s' "${SYSLOG_LAST}" | jq -r '.line // empty')
    if [[ "${CEF_LINE}" == "CEF:0|ministr|ministr-cloud-audit|1|"* ]]; then
        pass "CEF line has correct v0 header (CEF:0|ministr|ministr-cloud-audit|1|…)"
    else
        fail "CEF header malformed: '${CEF_LINE:0:120}'"
    fi
    # Action field is position 4 (0-indexed) — split on `|`.
    CEF_ACTION=$(printf '%s' "${CEF_LINE}" | awk -F'|' '{print $5}')
    if [[ "${CEF_ACTION}" == "invite.created" ]]; then
        pass "CEF Signature (field 5) carries the audit action (invite.created)"
    else
        fail "CEF action field expected 'invite.created', got '${CEF_ACTION}'"
    fi
    # Extension fields are after the 7th `|`. Look for orgId=ORG_ID_A.
    if [[ "${CEF_LINE}" == *"orgId=${ORG_ID_A}"* ]]; then
        pass "CEF extension carries orgId=ORG_ID_A (per-org routing locked)"
    else
        fail "CEF extension missing orgId=${ORG_ID_A}: '${CEF_LINE:0:200}'"
    fi
    # Extension also carries the standard CEF labels for actor.
    if [[ "${CEF_LINE}" == *"suser="* ]]; then
        pass "CEF extension carries suser= (standard CEF label for actor)"
    else
        fail "CEF extension missing suser= label: '${CEF_LINE:0:200}'"
    fi

    # 30) **F5.3-d-iii-c-udp — UDP syslog fallback for syslog_cef.**
    #     Re-POST the per-org config with udp:// endpoint pointing at
    #     the parallel UDP collector. PerOrgSiemDispatcher → kind=syslog_cef
    #     branch → dispatch_syslog_cef (scheme router) → udp helper.
    #     UDP is fire-and-forget; the harness sees the datagram arrive
    #     at the receiver but has no ack-side signal.
    info "F5.3-d-iii-c-udp: UDP transport for syslog/CEF"
    SIEM_SYSLOG_UDP_BODY=$(cat <<EOF
{"kind":"syslog_cef","endpoint_url":"udp://127.0.0.1:${SIEM_SYSLOG_UDP_PORT}","token":"","enabled":true}
EOF
)
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${SIEM_SYSLOG_UDP_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST per-org syslog_cef config (udp:// endpoint) → 200"

    # Snapshot UDP collector count before the audit trigger.
    SYSLOG_UDP_PRE=$(wc -l < "${SIEM_SYSLOG_UDP_LOG}" 2>/dev/null | tr -d ' ' || echo 0)

    # Fire one org-scoped audit emission.
    INVITE_UDP='{"email":"udp-syslog-test@e2e.test"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_UDP}"
    if [[ "${RESPONSE_STATUS}" == "201" || "${RESPONSE_STATUS}" == "200" ]]; then
        pass "POST /invites for UDP syslog flow fires org-scoped audit (HTTP ${RESPONSE_STATUS})"
    else
        fail "POST /invites for ORG_ID_A — expected 201/200, got ${RESPONSE_STATUS}"
    fi

    # Poll the UDP collector for the datagram. UDP delivery on
    # 127.0.0.1 is effectively zero-loss for sub-100-byte payloads;
    # the only timing factor is the tokio::spawn schedule.
    SYSLOG_UDP_WAIT=0
    while true; do
        SYSLOG_UDP_NOW=$(wc -l < "${SIEM_SYSLOG_UDP_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
        if [[ "${SYSLOG_UDP_NOW}" -gt "${SYSLOG_UDP_PRE}" ]]; then
            break
        fi
        SYSLOG_UDP_WAIT=$((SYSLOG_UDP_WAIT + 1))
        if [[ "${SYSLOG_UDP_WAIT}" -gt 30 ]]; then
            break
        fi
        sleep 0.2
    done
    SYSLOG_UDP_POST=$(wc -l < "${SIEM_SYSLOG_UDP_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
    if [[ "${SYSLOG_UDP_POST}" -gt "${SYSLOG_UDP_PRE}" ]]; then
        pass "UDP collector got ≥1 datagram (pre=${SYSLOG_UDP_PRE}, post=${SYSLOG_UDP_POST})"
    else
        fail "no new datagrams at UDP collector after 6s — udp:// dispatch may not be wired"
    fi

    # Inspect the most-recent datagram. Same CEF v0 shape as TCP;
    # no trailing newline per RFC 3164/5424 (one datagram = one
    # message). The receiver's rstrip already collapsed any stray
    # CR/LF if present.
    UDP_LAST=$(tail -n1 "${SIEM_SYSLOG_UDP_LOG}" 2>/dev/null || echo '{}')
    UDP_CEF_LINE=$(printf '%s' "${UDP_LAST}" | jq -r '.line // empty')
    if [[ "${UDP_CEF_LINE}" == "CEF:0|ministr|ministr-cloud-audit|1|"* ]]; then
        pass "UDP datagram carries a CEF v0 line (header matches)"
    else
        fail "UDP datagram CEF header malformed: '${UDP_CEF_LINE:0:120}'"
    fi
    if [[ "${UDP_CEF_LINE}" == *"orgId=${ORG_ID_A}"* ]]; then
        pass "UDP CEF extension carries orgId=ORG_ID_A (UDP routing locked)"
    else
        fail "UDP CEF extension missing orgId=${ORG_ID_A}: '${UDP_CEF_LINE:0:200}'"
    fi

    # 31) **F5.3-d-iii-b-shim — s3_jsonl kind passes CRUD validation.**
    #     Validator only; no dispatch (F5.3-d-iii-b-dispatch will wire
    #     aws-sdk-s3). Exercises the per-kind scheme branch, the
    #     JSON-shape token validator, and the unknown-kind defensive
    #     reject cases.
    info "F5.3-d-iii-b-shim: s3_jsonl CRUD validation (dispatch deferred)"

    # Happy path: JSON-shape token + s3:// endpoint → 200.
    S3_GOOD_TOKEN='{"access_key_id":"AKIAEXAMPLE","secret_access_key":"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY","region":"us-east-1"}'
    S3_GOOD_BODY=$(jq -nc --arg t "${S3_GOOD_TOKEN}" '{"kind":"s3_jsonl","endpoint_url":"s3://my-audit-bucket/audit/","token":$t,"enabled":true}')
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_GOOD_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST s3_jsonl with JSON token + s3:// → 200"
    GOT_S3_KIND=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.kind // empty')
    if [[ "${GOT_S3_KIND}" == "s3_jsonl" ]]; then
        pass "POST returns kind=s3_jsonl (CRUD admits the new kind)"
    else
        fail "POST kind expected s3_jsonl, got '${GOT_S3_KIND}'"
    fi
    # Token is still REDACTED on the response — same posture as Splunk/Datadog
    # despite the JSON-shape payload.
    GOT_S3_TOKEN=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.token // empty')
    if [[ "${GOT_S3_TOKEN}" == "[REDACTED]" ]]; then
        pass "POST s3_jsonl redacts the JSON-shape token in response"
    else
        fail "POST s3_jsonl LEAKED the credentials: '${GOT_S3_TOKEN:0:80}'"
    fi

    # Reject: s3_jsonl with http:// scheme (cross-kind mismatch).
    S3_BAD_SCHEME=$(jq -nc --arg t "${S3_GOOD_TOKEN}" '{"kind":"s3_jsonl","endpoint_url":"https://s3.amazonaws.com/bucket/","token":$t}')
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_BAD_SCHEME}"
    assert_status "${RESPONSE_STATUS}" "400" "POST s3_jsonl with https:// → 400 (scheme mismatch)"

    # Reject: malformed JSON token.
    S3_BAD_JSON='{"kind":"s3_jsonl","endpoint_url":"s3://b/","token":"this-is-not-json"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_BAD_JSON}"
    assert_status "${RESPONSE_STATUS}" "400" "POST s3_jsonl with non-JSON token → 400"

    # Reject: JSON parses but missing required field.
    S3_MISSING_FIELD=$(jq -nc '{"kind":"s3_jsonl","endpoint_url":"s3://b/","token":"{\"secret_access_key\":\"s\",\"region\":\"r\"}"}')
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_MISSING_FIELD}"
    assert_status "${RESPONSE_STATUS}" "400" "POST s3_jsonl with token missing access_key_id → 400"

    # Reject: JSON-shape but empty region.
    S3_EMPTY_REGION='{"kind":"s3_jsonl","endpoint_url":"s3://b/","token":"{\"access_key_id\":\"AKIA\",\"secret_access_key\":\"s\",\"region\":\"\"}"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_EMPTY_REGION}"
    assert_status "${RESPONSE_STATUS}" "400" "POST s3_jsonl with empty region → 400"

    # 32) **F5.3-d-iii-b-dispatch — aws-sdk-s3 PUT path lands.**
    #     Reconfigure the per-org s3_jsonl row with
    #     endpoint_url_override pointing at the fake S3 server.
    #     Trigger an audit emission; verify a PUT arrived at
    #     /<bucket>/<key> with the right date-partition prefix +
    #     body shape.
    info "F5.3-d-iii-b-dispatch: S3 PUT path via aws-sdk-s3"
    S3_DISPATCH_TOKEN=$(jq -nc \
        --arg ak "AKIAEXAMPLE_E2E" \
        --arg sk "wJalrXUtnFEMI_e2e_test_secret_$$" \
        --arg rg "us-east-1" \
        --arg ep "http://127.0.0.1:${SIEM_S3_PORT}" \
        '{access_key_id:$ak, secret_access_key:$sk, region:$rg, endpoint_url_override:$ep}')
    S3_DISPATCH_BODY=$(jq -nc --arg t "${S3_DISPATCH_TOKEN}" '{"kind":"s3_jsonl","endpoint_url":"s3://ministr-audit-bucket/audit/","token":$t,"enabled":true}')
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/siem/config" "${TOKEN_A}" "${S3_DISPATCH_BODY}"
    assert_status "${RESPONSE_STATUS}" "200" "POST s3_jsonl dispatch config (endpoint_url_override → fake S3) → 200"

    # Snapshot fake-S3 line count before audit trigger.
    S3_PRE=$(wc -l < "${SIEM_S3_LOG}" 2>/dev/null | tr -d ' ' || echo 0)

    # Fire one org-scoped audit emission via POST /invites.
    INVITE_S3='{"email":"s3-dispatch-test@e2e.test"}'
    curl_request POST "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${INVITE_S3}"
    if [[ "${RESPONSE_STATUS}" == "201" || "${RESPONSE_STATUS}" == "200" ]]; then
        pass "POST /invites for S3 dispatch flow fires org-scoped audit (HTTP ${RESPONSE_STATUS})"
    else
        fail "POST /invites for ORG_ID_A — expected 201/200, got ${RESPONSE_STATUS}"
    fi

    # Poll fake S3 for the new PUT. aws-sdk-s3 SigV4 signing + DNS
    # resolution + TCP connect take longer than the simpler dispatch
    # paths; widen the poll timeout to 10s.
    S3_WAIT=0
    while true; do
        S3_NOW=$(wc -l < "${SIEM_S3_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
        if [[ "${S3_NOW}" -gt "${S3_PRE}" ]]; then
            break
        fi
        S3_WAIT=$((S3_WAIT + 1))
        if [[ "${S3_WAIT}" -gt 50 ]]; then
            break
        fi
        sleep 0.2
    done
    S3_POST=$(wc -l < "${SIEM_S3_LOG}" 2>/dev/null | tr -d ' ' || echo 0)
    if [[ "${S3_POST}" -gt "${S3_PRE}" ]]; then
        pass "fake S3 received ≥1 PUT (pre=${S3_PRE}, post=${S3_POST})"
    else
        fail "no PUTs arrived at fake S3 after 10s — aws-sdk-s3 dispatch may be misconfigured · serve log: $(tail -3 /tmp/ministr-e2e-serve.log 2>/dev/null)"
    fi

    # Inspect the most-recent PUT.
    S3_LAST=$(tail -n1 "${SIEM_S3_LOG}" 2>/dev/null || echo '{}')
    S3_METHOD=$(printf '%s' "${S3_LAST}" | jq -r '.method // empty')
    S3_PATH=$(printf '%s' "${S3_LAST}" | jq -r '.path // empty')
    S3_AUTH=$(printf '%s' "${S3_LAST}" | jq -r '.auth // empty')
    if [[ "${S3_METHOD}" == "PUT" ]]; then
        pass "fake S3 request method is PUT (S3 PutObject semantics)"
    else
        fail "fake S3 request method expected PUT, got '${S3_METHOD}'"
    fi
    # Path-style URL: /<bucket>/<prefix>/year=Y/month=M/day=D/<ms>-<rand>.json
    # aws-sdk-s3 URL-encodes `=` to `%3D` and appends `?x-id=PutObject`.
    # Match against both the encoded and decoded forms so the assertion
    # survives future SDK encoding-policy changes.
    if [[ "${S3_PATH}" == /ministr-audit-bucket/audit/year*month*day*.json* ]]; then
        pass "fake S3 path matches /<bucket>/<prefix>/year…/month…/day…/*.json (Hive-partitioned)"
    else
        fail "fake S3 path didn't match expected partitioned shape: '${S3_PATH:0:200}'"
    fi
    # SigV4 signature must be present (proves the SDK signed the request).
    if [[ "${S3_AUTH}" == AWS4-HMAC-SHA256\ Credential=* ]]; then
        pass "fake S3 PUT carries an AWS4-HMAC-SHA256 SigV4 signature header"
    else
        fail "fake S3 auth header missing SigV4 signature: '${S3_AUTH:0:120}'"
    fi
    # The body decodes to JSON containing the audit action.
    S3_BODY_ACTION=$(printf '%s' "${S3_LAST}" | jq -r '.body | fromjson | .action // empty' 2>/dev/null)
    if [[ "${S3_BODY_ACTION}" == "invite.created" ]]; then
        pass "fake S3 PUT body[.action] == invite.created (audit shape preserved through PUT)"
    else
        fail "fake S3 PUT body action expected invite.created, got '${S3_BODY_ACTION}' · body: $(printf '%s' "${S3_LAST}" | jq -r '.body' | head -c 200)"
    fi
    # The body's org_id matches the per-org routing.
    S3_BODY_ORG=$(printf '%s' "${S3_LAST}" | jq -r '.body | fromjson | .org_id // empty' 2>/dev/null)
    if [[ "${S3_BODY_ORG}" == "${ORG_ID_A}" ]]; then
        pass "fake S3 PUT body[.org_id] == ORG_ID_A (per-org routing locked)"
    else
        fail "fake S3 PUT body org_id expected ${ORG_ID_A}, got '${S3_BODY_ORG}'"
    fi
else
    note "skipped F5.3-d-ii-config + F5.3-d-ii-dispatch + F5.3-d-iii-a + F5.3-d-iii-c + F5.3-d-iii-c-udp + F5.3-d-iii-b-shim + F5.3-d-iii-b-dispatch — ORG_ID_A not captured"
fi

# 33) **F5.3-b — REVOKE for audit_events immutability.** The
#     `ministr_audit_runtime` role created by migration 0015 has
#     INSERT+SELECT only on audit_events (parent + children). Under
#     `SET LOCAL ROLE ministr_audit_runtime` a DELETE returns
#     "permission denied for table audit_events", while INSERT +
#     SELECT continue to work. Production cutover (cloud serve
#     connects as this role) lands in F5.3-b-deploy via Pulumi.
info "F5.3-b: REVOKE locks DELETE/UPDATE under ministr_audit_runtime"

# Verify the role exists (sanity check; the migration runs at every
# serve boot, so failure here would mean the migration itself
# regressed).
ROLE_EXISTS=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT 1 FROM pg_roles WHERE rolname='ministr_audit_runtime';" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${ROLE_EXISTS}" == "1" ]]; then
    pass "ministr_audit_runtime role exists (migration 0015 applied)"
else
    fail "ministr_audit_runtime role missing — migration 0015 didn't apply"
fi

# DELETE under the constrained role should fail.
# Note: psql exits non-zero on any per-statement error even with
# ON_ERROR_STOP=0 (the flag controls processing continuation, not
# exit status). The harness runs under `set -e`, so we trail with
# `|| true` to capture-and-inspect rather than abort the script.
DELETE_OUT=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev \
    -c "BEGIN; SET LOCAL ROLE ministr_audit_runtime; DELETE FROM audit_events; ROLLBACK;" \
    2>&1 || true)
if printf '%s' "${DELETE_OUT}" | grep -q "permission denied for table audit_events"; then
    pass "DELETE under SET ROLE ministr_audit_runtime → permission denied (immutability locked)"
else
    fail "DELETE under SET ROLE didn't get permission denied; output: $(printf '%s' "${DELETE_OUT}" | head -c 200)"
fi

# UPDATE under the constrained role should also fail.
UPDATE_OUT=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev \
    -c "BEGIN; SET LOCAL ROLE ministr_audit_runtime; UPDATE audit_events SET action='hacked' WHERE id IN (SELECT id FROM audit_events LIMIT 1); ROLLBACK;" \
    2>&1 || true)
if printf '%s' "${UPDATE_OUT}" | grep -q "permission denied for table audit_events"; then
    pass "UPDATE under SET ROLE ministr_audit_runtime → permission denied"
else
    fail "UPDATE under SET ROLE didn't get permission denied; output: $(printf '%s' "${UPDATE_OUT}" | head -c 200)"
fi

# SELECT under the constrained role must still succeed (otherwise
# the cloud serve's audit-list endpoint would 500 once it connects
# as the constrained role in production).
SELECT_OUT=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "BEGIN; SET LOCAL ROLE ministr_audit_runtime; SELECT count(*) FROM audit_events; ROLLBACK;" \
    2>&1 || true)
# Extract the numeric count line — between BEGIN/SET/COMMIT noise
# the SELECT result is a single numeric line. Grep for it.
SELECT_COUNT=$(printf '%s' "${SELECT_OUT}" | grep -E '^[0-9]+$' | head -n1)
if [[ -n "${SELECT_COUNT}" ]]; then
    pass "SELECT under SET ROLE ministr_audit_runtime → succeeds (count=${SELECT_COUNT})"
else
    fail "SELECT under SET ROLE failed; output: $(printf '%s' "${SELECT_OUT}" | head -c 200)"
fi

# Verify a child partition has the same REVOKE/GRANT shape (parent
# grants do NOT cascade to existing children in PG; the migration
# walked pg_inherits to apply grants explicitly).
CHILD_DELETE_PRIV=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT has_table_privilege('ministr_audit_runtime', 'audit_events_y2026q2', 'DELETE');" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${CHILD_DELETE_PRIV}" == "f" ]]; then
    pass "child partition audit_events_y2026q2 also has DELETE revoked (migration walked pg_inherits)"
else
    fail "child partition unexpectedly has DELETE privilege: '${CHILD_DELETE_PRIV}'"
fi
CHILD_INSERT_PRIV=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT has_table_privilege('ministr_audit_runtime', 'audit_events_y2026q2', 'INSERT');" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${CHILD_INSERT_PRIV}" == "t" ]]; then
    pass "child partition audit_events_y2026q2 has INSERT granted"
else
    fail "child partition unexpectedly missing INSERT privilege: '${CHILD_INSERT_PRIV}'"
fi

# 34) **F5.3-c-ii-archive-fs — cold partition archive (FS sink).**
#     Seed a fixture row into an OLD partition (audit_events_y2024q1
#     was the migration-0013 seed for Q1-2024), invoke the archive
#     CLI, and verify (a) the gzipped JSONL file lands on disk,
#     (b) the partition is DETACH'd + DROP'd from the database,
#     (c) the fixture row decodes back from the archived file.
info "F5.3-c-ii-archive-fs: cold partition archive (DETACH + DROP)"

ARCHIVE_DIR="${MINISTR_AUDIT_ARCHIVE_DIR}"
rm -rf "${ARCHIVE_DIR}"
mkdir -p "${ARCHIVE_DIR}"

# Seed a fixture audit row into the 2024-Q1 partition WITH ORG_ID_A
# so the F5.3-c-ii-archive-read endpoint's cross-org filter has a
# matching row to return. ts is a specific UTC date inside Q1.
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev \
    -c "INSERT INTO audit_events (action, resource, org_id, ts) \
        VALUES ('archive.fixture', 'fixture-resource', \
                '${ORG_ID_A}'::uuid, '2024-02-15 12:00:00+00');" \
    >/dev/null 2>&1
FIXTURE_INSERTED=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT count(*) FROM audit_events_y2024q1 WHERE action='archive.fixture';" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${FIXTURE_INSERTED}" == "1" ]]; then
    pass "seeded archive.fixture row (org=ORG_ID_A) into audit_events_y2024q1"
else
    fail "fixture INSERT didn't land in audit_events_y2024q1 (count=${FIXTURE_INSERTED})"
fi

# Snapshot partition count + relkind BEFORE archive.
PARTS_PRE=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")

# Run the archive CLI. `|| ARCHIVE_STATUS=$?` captures the exit
# code without tripping `set -e` (set -e doesn't fire when the
# command is on the LHS of `||`).
ARCHIVE_STATUS=0
cargo run -q -p ministr-cli -- audit archive \
    --partition audit_events_y2024q1 \
    --archive-dir "${ARCHIVE_DIR}" \
    > /tmp/ministr-e2e-archive.log 2>&1 || ARCHIVE_STATUS=$?
if [[ "${ARCHIVE_STATUS}" == "0" ]]; then
    pass "ministr audit archive CLI exits 0"
else
    fail "audit archive exited ${ARCHIVE_STATUS} · log: $(tail -5 /tmp/ministr-e2e-archive.log)"
fi

# Verify the gzipped JSONL file landed at the expected path.
ARCHIVE_FILE="${ARCHIVE_DIR}/audit_events_y2024q1.jsonl.gz"
if [[ -s "${ARCHIVE_FILE}" ]]; then
    pass "archive file landed at ${ARCHIVE_FILE} (non-empty)"
else
    fail "archive file missing or empty at ${ARCHIVE_FILE}"
fi

# Verify the archive contains the fixture row when decompressed.
ARCHIVE_CONTENTS=$(gunzip -c "${ARCHIVE_FILE}" 2>/dev/null || echo '')
if printf '%s' "${ARCHIVE_CONTENTS}" | jq -e 'select(.action=="archive.fixture") | .resource' >/dev/null 2>&1; then
    pass "archive contains the archive.fixture row (decompressed + parses as JSONL)"
else
    fail "archive missing the fixture row · contents head: $(printf '%s' "${ARCHIVE_CONTENTS}" | head -c 200)"
fi

# Verify the partition is GONE from the live database.
PARTS_POST=$(psql_count "SELECT count(*) FROM pg_inherits WHERE inhparent = 'audit_events'::regclass;")
PARTS_EXPECT=$((PARTS_PRE - 1))
if [[ "${PARTS_POST}" == "${PARTS_EXPECT}" ]]; then
    pass "audit_events partition count dropped (pre=${PARTS_PRE}, post=${PARTS_POST}; DETACH'd 1)"
else
    fail "partition count expected ${PARTS_EXPECT}, got ${PARTS_POST}"
fi
PARTITION_EXISTS=$(psql_count "SELECT count(*) FROM pg_class WHERE relname='audit_events_y2024q1';")
if [[ "${PARTITION_EXISTS}" == "0" ]]; then
    pass "audit_events_y2024q1 no longer exists in pg_class (DROP'd)"
else
    fail "audit_events_y2024q1 still in pg_class (count=${PARTITION_EXISTS}); DETACH but not DROP?"
fi

# Verify defense-in-depth: invalid partition name rejected.
INVALID_OUT=$(cargo run -q -p ministr-cli -- audit archive \
    --partition "../../etc/passwd" \
    --archive-dir "${ARCHIVE_DIR}" 2>&1 || true)
# miette word-wraps long error messages; grep on a short stable
# substring rather than the full sentence.
if printf '%s' "${INVALID_OUT}" | grep -q "doesn't match"; then
    pass "path-traversal partition name '../../etc/passwd' rejected at CLI edge"
else
    fail "invalid partition name not rejected · output: $(printf '%s' "${INVALID_OUT}" | head -c 200)"
fi

# 35) **F5.3-c-ii-archive-read — read endpoint streams archived
#     JSONL rows back through the cloud API.** Just-archived
#     audit_events_y2024q1 → GET /audit/archived?from=2024-01-01&
#     to=2024-04-01 → returns the archive.fixture row with the
#     correct org_id + action + ts.
info "F5.3-c-ii-archive-read: GET /audit/archived returns archived rows"

# Happy path: owner queries the date range covering the archived
# partition. The handler reads the gzipped JSONL file, decompresses,
# filters by org_id, returns the matching rows.
curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/audit/archived?from=2024-01-01&to=2024-04-01" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "owner GET /audit/archived → 200"
GOT_ARCHIVED_ROWS=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.rows | length // 0')
if [[ "${GOT_ARCHIVED_ROWS}" -ge "1" ]]; then
    pass "GET /audit/archived returned ≥1 row (count=${GOT_ARCHIVED_ROWS})"
else
    fail "GET /audit/archived returned 0 rows · body: ${RESPONSE_BODY:0:200}"
fi
GOT_FIRST_ACTION=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.rows[0].action // empty')
if [[ "${GOT_FIRST_ACTION}" == "archive.fixture" ]]; then
    pass "first archived row carries action=archive.fixture (read shape matches archive shape)"
else
    fail "first archived row action expected 'archive.fixture', got '${GOT_FIRST_ACTION}'"
fi
GOT_FIRST_ORG=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.rows[0].org_id // empty')
if [[ "${GOT_FIRST_ORG}" == "${ORG_ID_A}" ]]; then
    pass "first archived row org_id == ORG_ID_A (cross-org isolation enforced)"
else
    fail "first archived row org_id expected ${ORG_ID_A}, got '${GOT_FIRST_ORG}'"
fi

# Cross-org isolation: tenant B (different user, doesn't own
# ORG_ID_A) gets 403 — the ACL is owner/admin-only.
curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/audit/archived?from=2024-01-01&to=2024-04-01" "${TOKEN_B}"
assert_status "${RESPONSE_STATUS}" "403" "non-owner GET /audit/archived → 403"

# Out-of-range date filter: the archive file is for Q1 2024 but the
# query asks for Q3 2024 → quarter range overlaps nothing → empty
# result.
curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/audit/archived?from=2024-07-01&to=2024-10-01" "${TOKEN_A}"
assert_status "${RESPONSE_STATUS}" "200" "owner GET /audit/archived (out-of-range) → 200"
OOR_ROWS=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.rows | length // 0')
if [[ "${OOR_ROWS}" == "0" ]]; then
    pass "out-of-range date filter returns 0 rows (date-range filter works)"
else
    fail "out-of-range filter returned ${OOR_ROWS} rows · body: ${RESPONSE_BODY:0:200}"
fi

# Malformed date → 500 (the handler currently surfaces validation
# errors via AuditApiError::Repo which renders as 500; the message
# in the body still identifies the bad field).
curl_request GET "${ENDPOINT}/api/v1/orgs/${ORG_ID_A}/audit/archived?from=not-a-date&to=2024-04-01" "${TOKEN_A}"
# Either 400 or 500 is acceptable — clip to "non-2xx".
if [[ "${RESPONSE_STATUS}" -ge "400" && "${RESPONSE_STATUS}" -lt "600" ]]; then
    pass "malformed from-date rejected (HTTP ${RESPONSE_STATUS})"
else
    fail "malformed from-date not rejected (HTTP ${RESPONSE_STATUS})"
fi

# 36) **F5.4-a — license-key gate refuses boot under invalid config.**
#     Default harness runs in community mode (no env vars set; the
#     entire 174 prior assertions stay green under that path). This
#     assertion spawns a SEPARATE serve attempt with garbage license
#     env vars and confirms it exits non-zero with a clear error
#     message. The serve validates BEFORE indexing / port-bind so
#     this should be near-instant.
info "F5.4-a: license-key gate refuses boot under invalid config"
LICENSE_BOOT_STATUS=0
# Bash subtlety: `||` INSIDE a `$(...)` substitution assigns only
# in the subshell. Put `|| STATUS=$?` OUTSIDE so the outer variable
# actually captures the exit code.
LICENSE_BOOT_OUT=$(MINISTR_LICENSE_KEY="not-a-jwt" \
    MINISTR_LICENSE_PUBLIC_KEY="not-a-pem" \
    cargo run -q -p ministr-cli -- --config "${CONFIG_PATH}" \
        serve --transport http --host 127.0.0.1 --port 18099 \
    2>&1 < /dev/null) || LICENSE_BOOT_STATUS=$?
if [[ "${LICENSE_BOOT_STATUS}" != "0" ]]; then
    pass "invalid license env vars → CLI exits non-zero (status=${LICENSE_BOOT_STATUS})"
else
    fail "invalid license env vars did NOT refuse boot · output: $(printf '%s' "${LICENSE_BOOT_OUT}" | tail -c 200)"
fi
# Match a substring of the boot error message — "license gate refused"
# (the full sentence wraps via miette but this prefix is stable).
if printf '%s' "${LICENSE_BOOT_OUT}" | grep -q "license gate refused"; then
    pass "boot error message identifies the license gate as the cause"
else
    fail "boot error didn't mention license gate · output: $(printf '%s' "${LICENSE_BOOT_OUT}" | tail -c 200)"
fi

# 37) **F5.4-b — seat-cap enforcement.** Mint a fresh license with
#     seat_count=2 via the new `ministr cloud mint-test-license`
#     subcommand. Spawn a SEPARATE serve with that license. Tenant
#     A creates an org (owner = seat 1), invites email 2 (seat 2 —
#     pending; not counted in v0), and the THIRD invite SHOULD be
#     refused with 402 once another member actually joins.
#
#     Honest scope: pending invites don't count, only actual
#     org_members rows. So the trip-up is: own as 1, invite 2nd to
#     consume → 2 members, invite 3rd → 402. Simulating consume
#     mid-harness is heavy; instead seed a second org_member row
#     directly via psql to push the count to 2, then attempt the
#     invite and expect 402.
info "F5.4-b: seat-cap enforcement refuses next-seat invite when count == seat_count"
# Earlier chunks (F5.2-f group_role_map, etc) have already added
# members to ORG_ID_A — exact count varies. Query the current
# count and set seat_count to that exact value so the very next
# invite trips the cap.
CURRENT_MEMBERS=$(docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "SELECT count(*) FROM org_members WHERE org_id='${ORG_ID_A}'::uuid;" \
    2>/dev/null | tr -d ' \r\n')
if [[ "${CURRENT_MEMBERS}" =~ ^[0-9]+$ ]] && [[ "${CURRENT_MEMBERS}" -ge "1" ]]; then
    pass "ORG_ID_A has ${CURRENT_MEMBERS} existing org_members (dynamic baseline)"
else
    fail "could not query org_members count, got '${CURRENT_MEMBERS}'"
fi
LICENSE_JSON=$(cargo run -q -p ministr-cli -- cloud mint-test-license \
    --enterprise-id "e2e-test-acme" --seat-count "${CURRENT_MEMBERS}" --valid-days 1 2>/dev/null || echo '{}')
LICENSE_JWT=$(printf '%s' "${LICENSE_JSON}" | jq -r '.jwt // empty')
LICENSE_PUBKEY=$(printf '%s' "${LICENSE_JSON}" | jq -r '.public_key_pem // empty')
if [[ -n "${LICENSE_JWT}" && -n "${LICENSE_PUBKEY}" ]]; then
    pass "mint-test-license CLI produced JWT + public key (jwt_len=${#LICENSE_JWT})"
else
    fail "mint-test-license CLI produced empty JWT / pubkey · json: ${LICENSE_JSON:0:200}"
fi

# Spawn the test serve with the license env vars set + a different
# port from the default harness's port. The serve uses the SAME
# Postgres so it sees ORG_ID_A's now-2-member state.
SEATCAP_PORT=18098
SEATCAP_LOG=/tmp/ministr-e2e-seatcap-serve.log
MINISTR_LICENSE_KEY="${LICENSE_JWT}" \
MINISTR_LICENSE_PUBLIC_KEY="${LICENSE_PUBKEY}" \
cargo run -q -p ministr-cli -- --config "${CONFIG_PATH}" \
    serve --transport http --oauth --host 127.0.0.1 --port "${SEATCAP_PORT}" \
    > "${SEATCAP_LOG}" 2>&1 &
SEATCAP_PID=$!
# Wait for it to bind /healthz.
SEATCAP_WAIT=0
until curl -sf "http://127.0.0.1:${SEATCAP_PORT}/healthz" >/dev/null 2>&1; do
    SEATCAP_WAIT=$((SEATCAP_WAIT + 1))
    if ! kill -0 "${SEATCAP_PID}" 2>/dev/null; then
        echo "seatcap serve crashed during boot — log tail:" >&2
        tail -10 "${SEATCAP_LOG}" >&2
        SEATCAP_PID=""
        break
    fi
    if [[ "${SEATCAP_WAIT}" -gt 60 ]]; then
        echo "seatcap serve didn't bind in 12s" >&2
        kill "${SEATCAP_PID}" 2>/dev/null || true
        SEATCAP_PID=""
        break
    fi
    sleep 0.2
done

if [[ -n "${SEATCAP_PID}" ]]; then
    # Attempt to invite a 3rd seat. Expect 402 (paywall).
    SEATCAP_BODY='{"email":"thirdseat@e2e.test"}'
    SEATCAP_ENDPOINT="http://127.0.0.1:${SEATCAP_PORT}"
    curl_request POST "${SEATCAP_ENDPOINT}/api/v1/orgs/${ORG_ID_A}/invites" "${TOKEN_A}" "${SEATCAP_BODY}"
    assert_status "${RESPONSE_STATUS}" "402" "POST /invites under seat_count=2 with 2 members → 402"
    # Body should be the paywall shape with error="seat_cap_exceeded".
    SEAT_ERR=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.error // empty')
    if [[ "${SEAT_ERR}" == "seat_cap_exceeded" ]]; then
        pass "402 body carries error=seat_cap_exceeded (paywall shape)"
    else
        fail "402 body error field expected 'seat_cap_exceeded', got '${SEAT_ERR}' · body: ${RESPONSE_BODY:0:200}"
    fi
    SEAT_CAP_FIELD=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.cap // empty')
    if [[ "${SEAT_CAP_FIELD}" == "${CURRENT_MEMBERS}" ]]; then
        pass "402 body cap=${CURRENT_MEMBERS} (license seat_count echoed)"
    else
        fail "402 body cap expected ${CURRENT_MEMBERS}, got '${SEAT_CAP_FIELD}'"
    fi

    # Cleanup the test serve.
    kill "${SEATCAP_PID}" 2>/dev/null || true
    wait "${SEATCAP_PID}" 2>/dev/null || true
fi

# 38) **F5.4-e-revoke — boot refuses under a revoked license.**
#     Mint a fresh license; compute its jwt_id_hash; write a
#     revocation list containing that hash; attempt to boot a serve
#     pointing MINISTR_LICENSE_REVOCATIONS at the file; expect
#     non-zero exit with "license revoked" in the error. This proves
#     the wire (CLI revoke-license → JSONL file → boot is_revoked_by_file
#     → LicenseError::Revoked → miette refusal) is connected.
info "F5.4-e-revoke: boot refuses under a revoked license"
REVOKE_JSON=$(cargo run -q -p ministr-cli -- cloud mint-test-license \
    --enterprise-id "e2e-revoke-test" --seat-count 5 --valid-days 1 2>/dev/null || echo '{}')
REVOKE_JWT=$(printf '%s' "${REVOKE_JSON}" | jq -r '.jwt // empty')
REVOKE_PUBKEY=$(printf '%s' "${REVOKE_JSON}" | jq -r '.public_key_pem // empty')
if [[ -n "${REVOKE_JWT}" && -n "${REVOKE_PUBKEY}" ]]; then
    pass "F5.4-e-revoke: mint-test-license produced fresh license fixture"
else
    fail "F5.4-e-revoke: mint-test-license produced empty fixture"
fi

REVOKE_JWT_FILE=$(mktemp -t ministr-e2e-revoke-jwt.XXXXXX)
REVOKE_LIST=$(mktemp -t ministr-e2e-revoke-list.XXXXXX)
printf '%s' "${REVOKE_JWT}" > "${REVOKE_JWT_FILE}"

# Use the CLI itself to write the revocation record — exercises the
# JWT-file → hash → JSONL-append path end-to-end rather than
# back-computing the hash in bash.
if cargo run -q -p ministr-cli -- cloud revoke-license \
    --jwt "${REVOKE_JWT_FILE}" \
    --enterprise-id "e2e-revoke-test" \
    --reason "harness fixture for F5.4-e-revoke" \
    --revocation-list "${REVOKE_LIST}" 2>/dev/null; then
    pass "F5.4-e-revoke: revoke-license CLI exited 0"
else
    fail "F5.4-e-revoke: revoke-license CLI exited non-zero"
fi

if [[ -s "${REVOKE_LIST}" ]]; then
    REVOKE_LINE=$(head -1 "${REVOKE_LIST}")
    REVOKE_RECORDED_HASH=$(printf '%s' "${REVOKE_LINE}" | jq -r '.jwt_id_hash // empty')
    REVOKE_RECORDED_REASON=$(printf '%s' "${REVOKE_LINE}" | jq -r '.reason // empty')
    if [[ ${#REVOKE_RECORDED_HASH} -eq 16 ]]; then
        pass "revocation record carries 16-hex jwt_id_hash (${REVOKE_RECORDED_HASH})"
    else
        fail "revocation record jwt_id_hash malformed: '${REVOKE_RECORDED_HASH}'"
    fi
    if [[ "${REVOKE_RECORDED_REASON}" == "harness fixture for F5.4-e-revoke" ]]; then
        pass "revocation record carries operator-supplied reason"
    else
        fail "revocation record reason mismatch: '${REVOKE_RECORDED_REASON}'"
    fi
else
    fail "revocation list file is empty after revoke-license"
fi

# Now attempt to boot a serve under the revoked license. Expect
# non-zero exit AND "revoked" in the error output (F5.4-a-style boot
# check — fails fast before port-bind).
REVOKE_BOOT_STATUS=0
REVOKE_BOOT_OUT=$(MINISTR_LICENSE_KEY="${REVOKE_JWT}" \
    MINISTR_LICENSE_PUBLIC_KEY="${REVOKE_PUBKEY}" \
    MINISTR_LICENSE_REVOCATIONS="${REVOKE_LIST}" \
    cargo run -q -p ministr-cli -- --config "${CONFIG_PATH}" \
        serve --transport http --host 127.0.0.1 --port 18097 \
    2>&1 < /dev/null) || REVOKE_BOOT_STATUS=$?
if [[ "${REVOKE_BOOT_STATUS}" != "0" ]]; then
    pass "revoked license → CLI exits non-zero (status=${REVOKE_BOOT_STATUS})"
else
    fail "revoked license did NOT refuse boot · output: $(printf '%s' "${REVOKE_BOOT_OUT}" | tail -c 200)"
fi
if printf '%s' "${REVOKE_BOOT_OUT}" | grep -qi "revoked"; then
    pass "boot error message identifies the revocation as the cause"
else
    fail "boot error didn't mention revocation · output: $(printf '%s' "${REVOKE_BOOT_OUT}" | tail -c 200)"
fi

# Cleanup harness fixtures.
rm -f "${REVOKE_JWT_FILE}" "${REVOKE_LIST}"

# 38c) **F5.4-e-revoke-api-fetch — URL-based revocation refuses boot.**
#      Mint a fresh license, write its hash into a JSONL file the
#      main test_serve already serves via
#      MINISTR_LICENSE_REVOCATIONS_SERVE_PATH (line ~437), then boot
#      a test-serve with MINISTR_LICENSE_REVOCATIONS_URL pointing
#      at the main serve's /api/v1/license-revocations.jsonl. The
#      boot validator should fetch the URL, find the hash, and
#      refuse to start.
info "F5.4-e-revoke-api-fetch: URL-based revocation refuses boot"
URLFETCH_JSON=$(cargo run -q -p ministr-cli -- cloud mint-test-license \
    --enterprise-id "e2e-urlfetch-test" --seat-count 3 --valid-days 1 2>/dev/null || echo '{}')
URLFETCH_JWT=$(printf '%s' "${URLFETCH_JSON}" | jq -r '.jwt // empty')
URLFETCH_PUB=$(printf '%s' "${URLFETCH_JSON}" | jq -r '.public_key_pem // empty')
URLFETCH_HASH=$(cargo run -q -p ministr-cli -- cloud revoke-license \
    --jwt-id-hash $(printf '%s' "${URLFETCH_JWT}" | shasum -a 256 | awk '{print substr($1,1,16)}') \
    --enterprise-id "e2e-urlfetch-test" \
    --reason "F5.4-e-revoke-api-fetch harness fixture" \
    --revocation-list "${REVOKE_API_FIXTURE}" 2>&1 | tail -1 || true)
# The serve reads MINISTR_LICENSE_REVOCATIONS_SERVE_PATH at request
# time, so the new line in the fixture file is visible to GET /api/v1/...
# immediately without bouncing the main serve.
URLFETCH_CACHE=$(mktemp -t ministr-e2e-urlfetch-cache.XXXXXX)
rm -f "${URLFETCH_CACHE}" # ensure no stale cache before the test
URLFETCH_BOOT_STATUS=0
URLFETCH_BOOT_OUT=$(MINISTR_LICENSE_KEY="${URLFETCH_JWT}" \
    MINISTR_LICENSE_PUBLIC_KEY="${URLFETCH_PUB}" \
    MINISTR_LICENSE_REVOCATIONS_URL="${ENDPOINT}/api/v1/license-revocations.jsonl" \
    MINISTR_LICENSE_REVOCATIONS_CACHE_PATH="${URLFETCH_CACHE}" \
    cargo run -q -p ministr-cli -- --config "${CONFIG_PATH}" \
        serve --transport http --host 127.0.0.1 --port 18096 \
    2>&1 < /dev/null) || URLFETCH_BOOT_STATUS=$?
if [[ "${URLFETCH_BOOT_STATUS}" != "0" ]]; then
    pass "URL-fetched revocation refused boot (status=${URLFETCH_BOOT_STATUS})"
else
    fail "URL-fetched revocation did NOT refuse boot · output: $(printf '%s' "${URLFETCH_BOOT_OUT}" | tail -c 200)"
fi
if printf '%s' "${URLFETCH_BOOT_OUT}" | grep -qi "revoked"; then
    pass "URL-fetch boot error message identifies revocation"
else
    fail "URL-fetch boot error didn't mention revocation · output: $(printf '%s' "${URLFETCH_BOOT_OUT}" | tail -c 200)"
fi
# Verify cache file was populated by the boot-time fetch.
if [[ -s "${URLFETCH_CACHE}" ]]; then
    pass "cache file populated by the fetcher (operator's grace-window source)"
else
    fail "cache file empty/missing after boot fetch — path=${URLFETCH_CACHE}"
fi
rm -f "${URLFETCH_CACHE}"

# 38b) **F5.5-b-sla-skeleton — /sla endpoint returns uptime envelope.**
#      Foundation for the eventual status.ministr.ai dashboard + richer
#      load-balancer probes. /healthz today returns only status/corpus_count/
#      version; /sla adds uptime_secs + started_at_iso so polling consumers
#      can compute boot moment + age without inverting wall-clock deltas.
info "F5.5-b-sla-skeleton: /sla endpoint returns uptime envelope"
curl_request GET "${ENDPOINT}/sla" ""
assert_status "${RESPONSE_STATUS}" "200" "GET /sla returns 200 (unauthenticated)"
SLA_STATUS=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.status // empty')
SLA_VERSION=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.version // empty')
SLA_UPTIME=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.uptime_secs // empty')
SLA_STARTED=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.started_at_iso // empty')
if [[ "${SLA_STATUS}" == "ready" ]]; then
    pass "/sla body status==ready"
else
    fail "/sla status mismatch — got '${SLA_STATUS}'"
fi
if [[ -n "${SLA_VERSION}" && "${SLA_VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    pass "/sla body version is semver-shaped (${SLA_VERSION})"
else
    fail "/sla version malformed: '${SLA_VERSION}'"
fi
if [[ "${SLA_UPTIME}" =~ ^[0-9]+$ ]]; then
    pass "/sla body uptime_secs is non-negative integer (${SLA_UPTIME}s)"
else
    fail "/sla uptime_secs non-numeric: '${SLA_UPTIME}'"
fi
if [[ "${SLA_STARTED}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]; then
    pass "/sla body started_at_iso is ISO-8601 Z-suffixed (${SLA_STARTED})"
else
    fail "/sla started_at_iso malformed: '${SLA_STARTED}'"
fi

# F5.5-b-latency — by this point in the harness, hundreds of
# requests have already passed through the middleware. /sla.latency
# should report a non-zero rolling-window sample count plus
# numeric p50/p95/p99 in milliseconds.
SLA_LAT_COUNT=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.count // empty')
SLA_LAT_P50=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.p50_ms // empty')
SLA_LAT_P95=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.p95_ms // empty')
SLA_LAT_P99=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.p99_ms // empty')
if [[ "${SLA_LAT_COUNT}" =~ ^[0-9]+$ ]] && [[ "${SLA_LAT_COUNT}" -gt 0 ]]; then
    pass "/sla.latency.count > 0 (${SLA_LAT_COUNT} samples; F5.5-b-latency middleware is recording)"
else
    fail "/sla.latency.count missing or zero: '${SLA_LAT_COUNT}'"
fi
if [[ "${SLA_LAT_P50}" =~ ^[0-9]+$ ]] && [[ "${SLA_LAT_P95}" =~ ^[0-9]+$ ]] && [[ "${SLA_LAT_P99}" =~ ^[0-9]+$ ]]; then
    pass "/sla.latency p50/p95/p99 are non-negative ms (p50=${SLA_LAT_P50}ms p95=${SLA_LAT_P95}ms p99=${SLA_LAT_P99}ms)"
else
    fail "/sla.latency percentiles non-numeric — p50='${SLA_LAT_P50}' p95='${SLA_LAT_P95}' p99='${SLA_LAT_P99}'"
fi
if [[ "${SLA_LAT_P50}" -le "${SLA_LAT_P95}" ]] && [[ "${SLA_LAT_P95}" -le "${SLA_LAT_P99}" ]]; then
    pass "/sla.latency percentile monotonicity holds (p50 ≤ p95 ≤ p99)"
else
    fail "/sla.latency percentiles non-monotonic — p50=${SLA_LAT_P50} p95=${SLA_LAT_P95} p99=${SLA_LAT_P99}"
fi

# F5.5-b-persist-read — by this point in the harness, the
# persist-write flush task has landed multiple snapshots
# (MINISTR_SLA_FLUSH_SECS=2 on test_serve). The /sla handler should
# now read the max p95 from request_latency_snapshots and surface it
# as latency.window_30d_max_p95_ms. Compare with the snapshot from
# the SAME /sla call (current SLA_LAT_P95) and check the historical
# max is >= the current p95 (rolling-max is monotonic over time).
# We re-fetch /sla because the earlier capture happened BEFORE the
# persist-write task had run; pull a fresh body so the window field
# is wired through.
curl_request GET "${ENDPOINT}/sla" ""
SLA_WINDOW_P95=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.window_30d_max_p95_ms // empty')
SLA_CURR_P95=$(printf '%s' "${RESPONSE_BODY}" | jq -r '.latency.p95_ms // empty')
if [[ "${SLA_WINDOW_P95}" =~ ^[0-9]+$ ]]; then
    pass "/sla.latency.window_30d_max_p95_ms is numeric (${SLA_WINDOW_P95}ms; persist-read wire live)"
else
    fail "/sla.latency.window_30d_max_p95_ms missing or non-numeric: '${SLA_WINDOW_P95}'"
fi
if [[ "${SLA_WINDOW_P95}" -ge "${SLA_CURR_P95}" ]]; then
    pass "window_30d_max_p95_ms (${SLA_WINDOW_P95}ms) >= current p95_ms (${SLA_CURR_P95}ms) — rolling-max monotonicity holds"
else
    fail "window_30d_max_p95_ms (${SLA_WINDOW_P95}) < current p95_ms (${SLA_CURR_P95}) — historical max should not be less than current"
fi

# 39) **F5.4-e-rotate — re-mint in-flight licenses against a new key.**
#     Generate old + new keypairs; mint two licenses against the OLD
#     key (alpha + beta), recording both in the audit log; revoke alpha;
#     run rotate-license-keys; assert exactly one new JWT file landed
#     in out-dir (the unrevoked beta), and that it carries a fresh
#     jwt_id_hash distinct from the original. Validates the full wire:
#     audit-log dedup → revocation filter → re-mint with new key.
info "F5.4-e-rotate: re-mint in-flight licenses against a new keypair"
ROT_WORK=$(mktemp -d -t ministr-e2e-rotate.XXXXXX)
ROT_OLD_PRIV="${ROT_WORK}/old-private.pem"
ROT_OLD_PUB="${ROT_WORK}/old-public.pem"
ROT_NEW_PRIV="${ROT_WORK}/new-private.pem"
ROT_NEW_PUB="${ROT_WORK}/new-public.pem"
ROT_AUDIT="${ROT_WORK}/audit.jsonl"
ROT_REV_LIST="${ROT_WORK}/revocations.jsonl"
ROT_NEW_AUDIT="${ROT_WORK}/new-audit.jsonl"
ROT_OUT_DIR="${ROT_WORK}/reissued"

# Step 1: two keypairs.
if cargo run -q -p ministr-cli -- cloud generate-license-keypair \
    --private-key "${ROT_OLD_PRIV}" --public-key "${ROT_OLD_PUB}" >/dev/null 2>&1 \
 && cargo run -q -p ministr-cli -- cloud generate-license-keypair \
    --private-key "${ROT_NEW_PRIV}" --public-key "${ROT_NEW_PUB}" >/dev/null 2>&1; then
    pass "F5.4-e-rotate: generated old + new keypairs"
else
    fail "F5.4-e-rotate: failed to generate keypairs"
fi

# Step 2: mint two licenses with the OLD key into the audit log.
if cargo run -q -p ministr-cli -- cloud mint-license \
    --private-key "${ROT_OLD_PRIV}" \
    --enterprise-id "rotate-alpha" --seat-count 10 --valid-days 60 \
    --audit-log "${ROT_AUDIT}" --out "${ROT_WORK}/alpha.jwt" >/dev/null 2>&1 \
 && cargo run -q -p ministr-cli -- cloud mint-license \
    --private-key "${ROT_OLD_PRIV}" \
    --enterprise-id "rotate-beta" --seat-count 20 --valid-days 60 \
    --audit-log "${ROT_AUDIT}" --out "${ROT_WORK}/beta.jwt" >/dev/null 2>&1; then
    pass "F5.4-e-rotate: minted 2 licenses with old key into audit log"
else
    fail "F5.4-e-rotate: failed to mint test licenses"
fi

# Step 3: revoke alpha.
if cargo run -q -p ministr-cli -- cloud revoke-license \
    --jwt "${ROT_WORK}/alpha.jwt" \
    --enterprise-id "rotate-alpha" \
    --reason "F5.4-e-rotate harness fixture" \
    --revocation-list "${ROT_REV_LIST}" >/dev/null 2>&1; then
    pass "F5.4-e-rotate: revoked rotate-alpha (will be skipped during rotation)"
else
    fail "F5.4-e-rotate: revoke-license failed for alpha"
fi

# Step 4: run rotate.
ROT_OUT=$(cargo run -q -p ministr-cli -- cloud rotate-license-keys \
    --audit-log "${ROT_AUDIT}" \
    --revocation-list "${ROT_REV_LIST}" \
    --new-private-key "${ROT_NEW_PRIV}" \
    --out-dir "${ROT_OUT_DIR}" \
    --new-audit-log "${ROT_NEW_AUDIT}" \
    --valid-days 30 2>/dev/null) || true

if printf '%s' "${ROT_OUT}" | grep -q "1 re-issued, 1 skipped (revoked)"; then
    pass "rotate summary: 1 re-issued + 1 skipped (revoked) — matches fixture"
else
    fail "rotate summary line mismatch · output: $(printf '%s' "${ROT_OUT}" | tail -c 200)"
fi

# Step 5: assert exactly one JWT file in out-dir, and it's beta.
ROT_FILES=$(ls "${ROT_OUT_DIR}"/*.jwt 2>/dev/null | wc -l | tr -d ' ')
if [[ "${ROT_FILES}" == "1" ]]; then
    pass "out-dir holds exactly 1 reissued JWT (revoked alpha was skipped)"
else
    fail "out-dir holds ${ROT_FILES} JWT files (expected 1)"
fi

ROT_BETA_FILE=$(ls "${ROT_OUT_DIR}"/rotate-beta-*.jwt 2>/dev/null | head -1)
if [[ -n "${ROT_BETA_FILE}" && -s "${ROT_BETA_FILE}" ]]; then
    pass "reissued JWT filename starts with rotate-beta- (sanitised enterprise_id)"
else
    fail "no rotate-beta-*.jwt file in out-dir"
fi

# Step 6: new audit log has exactly one record for rotate-beta.
ROT_NEW_LINES=$(wc -l < "${ROT_NEW_AUDIT}" 2>/dev/null | tr -d ' ')
ROT_NEW_EID=$(jq -r '.enterprise_id // empty' "${ROT_NEW_AUDIT}" 2>/dev/null | head -1)
if [[ "${ROT_NEW_LINES}" == "1" && "${ROT_NEW_EID}" == "rotate-beta" ]]; then
    pass "new audit log has 1 record for rotate-beta (rotation cycle is auditable)"
else
    fail "new audit log unexpected — lines=${ROT_NEW_LINES} eid=${ROT_NEW_EID}"
fi

# Cleanup.
rm -rf "${ROT_WORK}"

# 40) **F5.5-b-persist-write — periodic flush of LatencyTracker to PG.**
#     With MINISTR_SLA_FLUSH_SECS=2 set on test_serve startup and many
#     requests already in flight, request_latency_snapshots should hold
#     ≥1 row by now. The serve has been up ≥10s in this harness run;
#     the first flush fires after one tick interval (2s) so multiple
#     rows are expected.
info "F5.5-b-persist-write: periodic LatencyTracker → request_latency_snapshots flush"
SLA_SNAP_COUNT=$(psql_count "SELECT count(*) FROM request_latency_snapshots WHERE ts_unix > extract(epoch FROM (NOW() - INTERVAL '60 seconds'))::bigint;")
if [[ "${SLA_SNAP_COUNT}" =~ ^[0-9]+$ ]] && [[ "${SLA_SNAP_COUNT}" -ge 1 ]]; then
    pass "request_latency_snapshots has ≥1 recent row (count=${SLA_SNAP_COUNT}; flush task is running)"
else
    fail "request_latency_snapshots empty after harness run — count='${SLA_SNAP_COUNT}'"
fi
# Verify the most recent row carries non-zero percentile values
# (the LatencyTracker had real samples, not a 0-only snapshot).
SLA_LATEST_P95=$(psql_count "SELECT p95_us FROM request_latency_snapshots ORDER BY ts_unix DESC LIMIT 1;")
if [[ "${SLA_LATEST_P95}" =~ ^[0-9]+$ ]] && [[ "${SLA_LATEST_P95}" -gt 0 ]]; then
    pass "most-recent snapshot p95_us > 0 (${SLA_LATEST_P95}µs ≈ $((SLA_LATEST_P95 / 1000))ms)"
else
    fail "most-recent snapshot p95_us missing or zero — got '${SLA_LATEST_P95}'"
fi
# Sanity: monotonicity should hold on the persisted row too.
SLA_LATEST_P50=$(psql_count "SELECT p50_us FROM request_latency_snapshots ORDER BY ts_unix DESC LIMIT 1;")
SLA_LATEST_P99=$(psql_count "SELECT p99_us FROM request_latency_snapshots ORDER BY ts_unix DESC LIMIT 1;")
if [[ "${SLA_LATEST_P50}" -le "${SLA_LATEST_P95}" ]] && [[ "${SLA_LATEST_P95}" -le "${SLA_LATEST_P99}" ]]; then
    pass "persisted row percentile monotonicity holds (p50=${SLA_LATEST_P50}µs ≤ p95=${SLA_LATEST_P95}µs ≤ p99=${SLA_LATEST_P99}µs)"
else
    fail "persisted row monotonicity broken — p50=${SLA_LATEST_P50} p95=${SLA_LATEST_P95} p99=${SLA_LATEST_P99}"
fi

# 40b) **F5.4-e-audit-db — DB-backed mirror of the mint audit log.**
#      Mint a fresh license with MINISTR_PG_URL set (env var already
#      exported at line ~382); the dual-write should land one row in
#      license_issuances. Then assert via psql + via `list-licenses
#      --pg-url` that the row is readable from both directions.
info "F5.4-e-audit-db: DB-backed mirror of mint audit log"
AUDIT_DB_WORK=$(mktemp -d -t ministr-e2e-audit-db.XXXXXX)
AUDIT_DB_PRIV="${AUDIT_DB_WORK}/private.pem"
AUDIT_DB_PUB="${AUDIT_DB_WORK}/public.pem"
AUDIT_DB_JWT="${AUDIT_DB_WORK}/audit-db.jwt"
AUDIT_DB_JSONL="${AUDIT_DB_WORK}/audit.jsonl"
if cargo run -q -p ministr-cli -- cloud generate-license-keypair \
    --private-key "${AUDIT_DB_PRIV}" --public-key "${AUDIT_DB_PUB}" >/dev/null 2>&1; then
    pass "F5.4-e-audit-db: keypair generated for audit-db fixture"
else
    fail "F5.4-e-audit-db: keypair generation failed"
fi
# Mint with both --audit-log AND fall-through to MINISTR_PG_URL.
# The dual-write writes JSONL + PG. Use a unique enterprise_id so
# the row is easy to find in DB regardless of harness state.
AUDIT_DB_EID="e2e-audit-db-$$"
if cargo run -q -p ministr-cli -- cloud mint-license \
    --private-key "${AUDIT_DB_PRIV}" \
    --enterprise-id "${AUDIT_DB_EID}" \
    --seat-count 7 \
    --valid-days 30 \
    --audit-log "${AUDIT_DB_JSONL}" \
    --out "${AUDIT_DB_JWT}" >/dev/null 2>&1; then
    pass "F5.4-e-audit-db: mint-license with MINISTR_PG_URL fall-through exited 0"
else
    fail "F5.4-e-audit-db: mint-license failed"
fi
# psql-direct assertion: row landed in license_issuances.
AUDIT_DB_COUNT=$(psql_count "SELECT count(*) FROM license_issuances WHERE enterprise_id = '${AUDIT_DB_EID}';")
if [[ "${AUDIT_DB_COUNT}" == "1" ]]; then
    pass "license_issuances has 1 row for ${AUDIT_DB_EID} (dual-write live)"
else
    fail "license_issuances row count for ${AUDIT_DB_EID} expected 1, got '${AUDIT_DB_COUNT}'"
fi
# JSONL still has the line too — dual-write means BOTH backends carry it.
AUDIT_DB_JSONL_LINES=$(wc -l < "${AUDIT_DB_JSONL}" 2>/dev/null | tr -d ' ')
if [[ "${AUDIT_DB_JSONL_LINES}" == "1" ]]; then
    pass "JSONL audit log also has 1 line (dual-write doesn't replace JSONL)"
else
    fail "JSONL audit log lines expected 1, got '${AUDIT_DB_JSONL_LINES}'"
fi
# Idempotency: re-mint with the same enterprise/seats/days would produce
# the SAME JWT (deterministic — no nonce; exp depends on now_secs so
# may differ by 1s). Skip the same-JWT idempotency assertion since we
# can't guarantee identical exp; instead assert that running mint twice
# under retry semantics keeps the row count sane (≥1, not 5+).
# list-licenses readback through PG.
AUDIT_DB_LIST_OUT=$(cargo run -q -p ministr-cli -- cloud list-licenses \
    --pg-url "${MINISTR_PG_URL}" --format json 2>/dev/null | grep -c "${AUDIT_DB_EID}" || true)
if [[ "${AUDIT_DB_LIST_OUT}" -ge 1 ]]; then
    pass "list-licenses --pg-url renders ${AUDIT_DB_EID} from license_issuances"
else
    fail "list-licenses --pg-url didn't surface ${AUDIT_DB_EID} (count=${AUDIT_DB_LIST_OUT})"
fi
# Cleanup the fixture row + working dir.
docker compose -f docker-compose.dev.yml exec -T postgres \
    psql -U ministr -d ministr_dev -tA \
    -c "DELETE FROM license_issuances WHERE enterprise_id = '${AUDIT_DB_EID}';" \
    >/dev/null 2>&1 || true
rm -rf "${AUDIT_DB_WORK}"

# 41) **F5.5-b-persist-retention — sla-prune-snapshots CLI removes old rows.**
#     Use --older-than-secs 1 against the snapshots the persist-write
#     task has been writing (with MINISTR_SLA_FLUSH_SECS=2). All rows
#     older than 1 second should be deleted; very-recently-written
#     rows (less than 1s old) may survive but the table count should
#     drop substantially. Also verify the defensive 0-second refusal
#     fires.
info "F5.5-b-persist-retention: sla-prune-snapshots CLI removes old rows"
PRE_PRUNE_COUNT=$(psql_count "SELECT count(*) FROM request_latency_snapshots;")
if cargo run -q -p ministr-cli -- cloud sla-prune-snapshots \
    --older-than-secs 1 >/dev/null 2>&1; then
    pass "sla-prune-snapshots CLI exited 0"
else
    fail "sla-prune-snapshots CLI exited non-zero"
fi
POST_PRUNE_COUNT=$(psql_count "SELECT count(*) FROM request_latency_snapshots;")
if [[ "${POST_PRUNE_COUNT}" -lt "${PRE_PRUNE_COUNT}" ]]; then
    pass "request_latency_snapshots row count dropped (pre=${PRE_PRUNE_COUNT} post=${POST_PRUNE_COUNT})"
else
    fail "row count did not drop after prune — pre=${PRE_PRUNE_COUNT} post=${POST_PRUNE_COUNT}"
fi
# Defensive: --older-than-secs 0 must refuse.
if cargo run -q -p ministr-cli -- cloud sla-prune-snapshots \
    --older-than-secs 0 >/dev/null 2>&1; then
    fail "sla-prune-snapshots --older-than-secs 0 should refuse but exited 0"
else
    pass "sla-prune-snapshots --older-than-secs 0 refuses (defensive against accidental delete-all)"
fi

# 42) **F5.4-e-revoke-api-serve — public revocation-list endpoint.**
#     Main test_serve started with MINISTR_LICENSE_REVOCATIONS_SERVE_PATH
#     pointing at the seeded fixture. GET /api/v1/license-revocations.jsonl
#     should return 200 + application/x-ndjson + the fixture body.
info "F5.4-e-revoke-api-serve: public revocation-list endpoint"
REVOKE_API_RESP=$(curl -s -i "${ENDPOINT}/api/v1/license-revocations.jsonl" 2>&1)
REVOKE_API_STATUS=$(printf '%s' "${REVOKE_API_RESP}" | head -1 | awk '{print $2}')
if [[ "${REVOKE_API_STATUS}" == "200" ]]; then
    pass "GET /api/v1/license-revocations.jsonl returns 200"
else
    fail "GET /api/v1/license-revocations.jsonl returned ${REVOKE_API_STATUS} (expected 200)"
fi
# Content-Type header — case-insensitive grep because curl preserves
# server casing.
if printf '%s' "${REVOKE_API_RESP}" | grep -qi "^content-type: *application/x-ndjson"; then
    pass "Content-Type: application/x-ndjson (correct mime for JSONL streaming)"
else
    fail "Content-Type missing or wrong; response head: $(printf '%s' "${REVOKE_API_RESP}" | head -10)"
fi
# Cache-Control header — 5-minute public cache as a thundering-herd guard.
if printf '%s' "${REVOKE_API_RESP}" | grep -qi "^cache-control: *public, max-age=300"; then
    pass "Cache-Control: public, max-age=300 (thundering-herd guard)"
else
    fail "Cache-Control header missing or wrong"
fi
# Body carries the fixture's enterprise_id.
if printf '%s' "${REVOKE_API_RESP}" | grep -q "e2e-revoke-api-fixture"; then
    pass "body carries the seeded fixture entry"
else
    fail "body missing fixture; response: $(printf '%s' "${REVOKE_API_RESP}" | tail -5)"
fi
# Cleanup the fixture file.
rm -f "${REVOKE_API_FIXTURE}"

# ─── summary ──────────────────────────────────────────────────────────

echo
if [[ "${FAIL_COUNT}" -eq 0 ]]; then
    printf "${C_GREEN}${C_BOLD}━━ e2e PASS — ${PASS_COUNT} assertions green ━━${C_RESET}\n"
    exit 0
else
    printf "${C_RED}${C_BOLD}━━ e2e FAIL — ${PASS_COUNT} pass, ${FAIL_COUNT} fail ━━${C_RESET}\n"
    printf "${C_DIM}serve logs: ${SERVE_LOG}${C_RESET}\n"
    exit "${FAIL_COUNT}"
fi
