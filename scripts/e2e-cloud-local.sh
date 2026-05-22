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
    cargo run -q -p ministr-cli -- cloud mint-test-bearer \
        --github-id "${github_id}" \
        --email "${email}" 2>/dev/null \
        | jq -r '.token'
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
