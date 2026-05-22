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
for i in 2 3 4 5 6 7 8 9 10; do
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
else
    note "skipped F5.2-b/c/d OIDC assertions — ORG_ID_A not captured"
fi

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
