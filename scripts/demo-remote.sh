#!/usr/bin/env bash
# Watch the deployed Azure cloud index a real repo live.
#
# Resolves the cloud URL from MINISTR_CLOUD_BASE_URL or from
# `pulumi -C deploy/azure stack output publicBaseUrl`. Mints a bearer
# token via the cloud's OAuth self-issuer (no IdP in MVP scope =
# auto-consent), then delegates to `ministr cloud demo --clone-url`
# which clones, indexes, and streams progress in your terminal.
#
# Usage:
#   just demo-remote                              # uses pulumi stack output
#   MINISTR_CLOUD_BASE_URL=https://… just demo-remote
#   CLONE_URL=https://github.com/owner/repo.git just demo-remote
set -euo pipefail

C_BOLD='\033[1m'
C_CYAN='\033[36m'
C_DIM='\033[2m'
C_GREEN='\033[32m'
C_RED='\033[31m'
C_RESET='\033[0m'

step() { printf "${C_BOLD}${C_CYAN}\xe2\x96\xb6 %s${C_RESET}\n" "$*"; }
info() { printf "  ${C_DIM}\xc2\xb7${C_RESET} %s\n" "$*"; }
done_step() { printf "  ${C_GREEN}\xe2\x9c\x93${C_RESET} %s\n" "$*"; }
fail() { printf "  ${C_RED}\xe2\x9c\x97${C_RESET} %s\n" "$*" >&2; exit 1; }

# --- step 1 / 5 — resolve cloud URL ----------------------------------
#
# Resolution order:
#   1. MINISTR_CLOUD_BASE_URL if it points to a remote host.
#   2. `pulumi -C deploy/azure stack output publicBaseUrl`.
#
# A localhost / 127.0.0.1 / [::1] value for MINISTR_CLOUD_BASE_URL is
# almost always stale from a `demo-local` session in the same shell or
# from `.env.azure`. demo-remote is the REMOTE analogue by definition —
# refusing to honour a local-loopback URL here prevents the script from
# silently doing the wrong thing. Pass an explicit remote URL via the
# env var (or `MINISTR_CLOUD_BASE_URL=… just demo-remote`) to override.
step "step 1 / 5 — resolving cloud URL"
ENDPOINT="${MINISTR_CLOUD_BASE_URL:-}"
is_loopback_url() {
    local url="$1"
    local host
    # Strip scheme, then anything after the first `/` or `:`. Quick &
    # cheap — handles the common forms (http://localhost:8080,
    # http://127.0.0.1, http://[::1]:8080).
    host="${url#*://}"
    host="${host%%/*}"
    host="${host%%:*}"
    host="${host#[}"
    host="${host%]}"
    case "${host}" in
        localhost|127.0.0.1|::1|0.0.0.0) return 0 ;;
        *) return 1 ;;
    esac
}
if [[ -n "${ENDPOINT}" ]] && is_loopback_url "${ENDPOINT}"; then
    info "ignoring stale MINISTR_CLOUD_BASE_URL=${ENDPOINT} (loopback — use just demo-local)"
    ENDPOINT=""
fi
if [[ -z "${ENDPOINT}" ]]; then
    if ! command -v pulumi >/dev/null 2>&1; then
        fail "no remote MINISTR_CLOUD_BASE_URL set and pulumi CLI not on PATH"
    fi
    # Source the Azure env helper if present (resolves AZURE_STORAGE_*
    # so the azblob:// state backend opens). Skipped silently when the
    # script doesn't exist — useful when MINISTR_CLOUD_BASE_URL is set
    # directly and pulumi state access isn't needed.
    if [[ -f ./scripts/azure-env.sh ]]; then
        eval "$(./scripts/azure-env.sh 2>/dev/null || true)"
    fi
    info "reading from pulumi stack output publicBaseUrl"
    ENDPOINT=$(pulumi -C deploy/azure stack output publicBaseUrl 2>/dev/null || true)
    if [[ -z "${ENDPOINT}" ]]; then
        fail "pulumi stack output publicBaseUrl was empty — has \`pulumi up\` run?"
    fi
fi
ENDPOINT="${ENDPOINT%/}"
done_step "endpoint=${ENDPOINT}"

# --- step 2 / 5 — health probe --------------------------------------
step "step 2 / 5 — health probe"
if ! curl -sSf "${ENDPOINT}/healthz" -o /tmp/ministr-remote-healthz; then
    fail "GET ${ENDPOINT}/healthz failed — is the container live?"
fi
cat /tmp/ministr-remote-healthz | sed 's/^/    /'
echo
done_step "cloud is reachable"

# --- step 3 / 5 — OAuth (DCR + PKCE + token) -------------------------
# Mints a token by hand instead of letting `cloud demo` open a browser
# loopback flow — keeps demo-remote scriptable / CI-friendly. The
# cloud's `/oauth/authorize` auto-consents in MVP scope (no IdP wired).
step "step 3 / 5 — minting bearer token via OAuth self-issuer"
REG=$(curl -sS -X POST "${ENDPOINT}/oauth/register" \
    -H 'content-type: application/json' \
    -d '{"redirect_uris":["http://127.0.0.1:0/cb"],"client_name":"demo-remote","token_endpoint_auth_method":"none"}')
CID=$(printf '%s' "${REG}" | sed -n 's/.*"client_id":"\([^"]*\)".*/\1/p')
[[ -n "${CID}" ]] || fail "register did not return client_id: ${REG}"
info "registered demo client client_id=${CID}"

VER=$(openssl rand -base64 64 | tr -d '=\n' | tr '+/' '-_' | head -c 64)
CHAL=$(printf '%s' "${VER}" | openssl dgst -binary -sha256 | base64 | tr -d '=\n' | tr '+/' '-_')
ST=$(openssl rand -hex 16)

LOC=$(curl -sS -o /dev/null -w '%{redirect_url}' \
    "${ENDPOINT}/oauth/authorize?response_type=code&client_id=${CID}&redirect_uri=http%3A%2F%2F127.0.0.1%3A0%2Fcb&code_challenge=${CHAL}&code_challenge_method=S256&state=${ST}&scope=ministr%3Aread%20ministr%3Awrite")
CODE=$(printf '%s' "${LOC}" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p')
[[ -n "${CODE}" ]] || fail "authorize did not yield a code (redirect_url=${LOC})"
info "consent auto-approved, auth code received"

TOKEN_JSON=$(curl -sS -X POST "${ENDPOINT}/oauth/token" \
    -H 'content-type: application/x-www-form-urlencoded' \
    --data-urlencode "grant_type=authorization_code" \
    --data-urlencode "code=${CODE}" \
    --data-urlencode "redirect_uri=http://127.0.0.1:0/cb" \
    --data-urlencode "client_id=${CID}" \
    --data-urlencode "code_verifier=${VER}")
TOKEN=$(printf '%s' "${TOKEN_JSON}" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
[[ -n "${TOKEN}" ]] || fail "token exchange failed: ${TOKEN_JSON}"
done_step "bearer token acquired"

# --- step 4 / 5 — register a parent corpus ---------------------------
# The boot-time auto-register of /data/corpus is system-scoped — not
# visible to the demo client's tenant — so `cloud demo` would see an
# empty list and refuse to clone. POST one explicitly under our tenant
# to give the clone endpoint a parent to attach to.
step "step 4 / 5 — registering a tenant-scoped parent corpus"
REG_RESP=$(curl -sS -X POST "${ENDPOINT}/api/v1/corpora" \
    -H "authorization: Bearer ${TOKEN}" \
    -H "content-type: application/json" \
    -d '{"paths":["/data/corpus"],"display_name":"demo-parent"}')
PARENT_ID=$(printf '%s' "${REG_RESP}" | sed -n 's/.*"corpus_id":"\([^"]*\)".*/\1/p')
[[ -n "${PARENT_ID}" ]] || fail "register did not return corpus_id: ${REG_RESP}"
done_step "parent corpus_id=${PARENT_ID}"

# --- step 5 / 5 — clone + watch via `ministr cloud demo` -------------
# Default repo is small enough to index in ~30s. Override with
# CLONE_URL=… for your own public repo.
CLONE_URL_VAL="${CLONE_URL:-https://github.com/dtolnay/anyhow.git}"
step "step 5 / 5 — cloning ${CLONE_URL_VAL} and streaming progress"
echo
# `--flag=value` syntax handles `-`-leading tokens (base64url alphabet).
cargo run -q -p ministr-cli -- cloud demo \
    "--endpoint=${ENDPOINT}" \
    "--token=${TOKEN}" \
    "--parent=${PARENT_ID}" \
    "--clone-url=${CLONE_URL_VAL}"

echo
printf "${C_GREEN}${C_BOLD}\xe2\x94\x81\xe2\x94\x81 demo-remote complete \xe2\x94\x81\xe2\x94\x81${C_RESET}\n"
echo
info "endpoint: ${ENDPOINT}"
info "your token is in shell history if you want to re-run \`ministr cloud watch\` directly"
