#!/usr/bin/env bash
# F2.7 — fully automated local cloud demo.
#
# Brings up Postgres, seeds a small sample corpus, starts the cloud
# binary in the background, mints a bearer token against the
# self-issuer, then runs `ministr cloud demo` against it so you can
# WATCH the indexing happen live in the terminal.
#
# Idempotent: re-running cleans up the previous run's serve + Postgres
# before starting fresh. Ctrl-C at any point also tears them down.
#
# Usage:
#   just demo-local            # default flow
#   PORT=9090 just demo-local  # use a different port
#   KEEP=1 just demo-local     # leave Postgres + serve running on exit
set -euo pipefail

PORT="${PORT:-8080}"
ENDPOINT="http://localhost:${PORT}"
SAMPLE_DIR="${SAMPLE_DIR:-/tmp/ministr-demo-source}"
SERVE_LOG="${SERVE_LOG:-/tmp/ministr-demo-serve.log}"
KEEP="${KEEP:-0}"

C_BOLD='\033[1m'
C_CYAN='\033[36m'
C_DIM='\033[2m'
C_GREEN='\033[32m'
C_RESET='\033[0m'

step() { printf "${C_BOLD}${C_CYAN}▶ %s${C_RESET}\n" "$*"; }
info() { printf "  ${C_DIM}·${C_RESET} %s\n" "$*"; }
done_step() { printf "  ${C_GREEN}✓${C_RESET} %s\n" "$*"; }

SERVE_PID=""

cleanup() {
    if [[ "${KEEP}" == "1" ]]; then
        echo
        info "KEEP=1 — leaving Postgres + serve (PID ${SERVE_PID}) running. Tear down manually with:"
        info "  kill ${SERVE_PID}; just dev-cloud-down"
        return
    fi
    echo
    step "cleanup"
    if [[ -n "${SERVE_PID}" ]] && kill -0 "${SERVE_PID}" 2>/dev/null; then
        info "stopping serve (PID ${SERVE_PID})"
        kill "${SERVE_PID}" 2>/dev/null || true
        wait "${SERVE_PID}" 2>/dev/null || true
    fi
    info "stopping Postgres"
    docker compose -f docker-compose.dev.yml down >/dev/null 2>&1 || true
    info "demo session ended"
}
trap cleanup EXIT INT TERM

step "step 1 / 7 — bringing up Postgres on :55432"
docker compose -f docker-compose.dev.yml up -d >/dev/null
# Wait until the container's healthcheck passes (max 30s).
attempts=0
until docker compose -f docker-compose.dev.yml exec -T postgres \
        pg_isready -U ministr -d ministr_dev >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [[ "${attempts}" -gt 30 ]]; then
        echo "Postgres failed to become ready after 30 attempts" >&2
        exit 1
    fi
    sleep 1
done
done_step "Postgres ready"

step "step 2 / 7 — seeding a sample corpus at ${SAMPLE_DIR}"
rm -rf "${SAMPLE_DIR}"
mkdir -p "${SAMPLE_DIR}/src/rs" "${SAMPLE_DIR}/src/py" "${SAMPLE_DIR}/src/ts"
cat > "${SAMPLE_DIR}/README.md" <<'MD'
# demo-source

Synthetic corpus used by `just demo-local` to exercise the cloud
indexing pipeline end-to-end. ~150 files across Rust, Python, and
TypeScript so the embedder has measurable work to do — the SSE
progress stream actually ticks instead of reporting "done in 0s".
MD

# Generate 50 files per language. The contents are valid syntax with
# distinct function names so the symbol table and the embedder both
# see real signal. Total ~150 files keeps the indexer busy a few
# seconds — long enough for the watcher to attach and see live ticks.
for i in $(seq 1 50); do
    cat > "${SAMPLE_DIR}/src/rs/module_${i}.rs" <<EOF
//! Demo Rust module ${i}. Tests the embedder + symbol table at scale.
pub fn handle_${i}(input: &str) -> String {
    format!("module_${i} handled: {input}")
}
pub fn process_${i}(value: u32) -> u32 {
    value.wrapping_mul(${i}).wrapping_add(7)
}
pub struct Widget${i} {
    pub id: u32,
    pub label: String,
}
impl Widget${i} {
    pub fn new(id: u32, label: impl Into<String>) -> Self {
        Self { id, label: label.into() }
    }
    pub fn describe(&self) -> String {
        format!("Widget${i}#{} ({})", self.id, self.label)
    }
}
EOF
    cat > "${SAMPLE_DIR}/src/py/module_${i}.py" <<EOF
"""Demo Python module ${i}."""
def handle_${i}(text: str) -> str:
    return f"module_${i} handled: {text}"

def process_${i}(value: int) -> int:
    return (value * ${i} + 7) & 0xFFFF_FFFF

class Widget${i}:
    def __init__(self, id_: int, label: str) -> None:
        self.id = id_
        self.label = label

    def describe(self) -> str:
        return f"Widget${i}#{self.id} ({self.label})"
EOF
    cat > "${SAMPLE_DIR}/src/ts/module_${i}.ts" <<EOF
/** Demo TypeScript module ${i}. */
export function handle_${i}(input: string): string {
    return \`module_${i} handled: \${input}\`;
}
export function process_${i}(value: number): number {
    return ((value * ${i}) + 7) >>> 0;
}
export class Widget${i} {
    constructor(public readonly id: number, public readonly label: string) {}
    describe(): string {
        return \`Widget${i}#\${this.id} (\${this.label})\`;
    }
}
EOF
done

FILE_COUNT=$(find "${SAMPLE_DIR}" -type f | wc -l | tr -d ' ')
done_step "${FILE_COUNT} files staged across rs / py / ts"

step "step 3 / 7 — starting \`ministr serve\` in background"
export MINISTR_PG_URL="postgres://ministr:ministr@localhost:55432/ministr_dev?sslmode=disable"
export MINISTR_CLOUD_BASE_URL="${ENDPOINT}"
export MINISTR_BLOB_FS_ROOT="/tmp/ministr-demo-blobs"
# Force in-memory OAuth state so this run is isolated from any prior
# `ministr serve` runs that may have left a SQLite OAuth DB behind.
unset MINISTR_CLOUD_DATA_DIR
rm -rf "${MINISTR_BLOB_FS_ROOT}"

# Build first so the background process starts immediately; pipes the
# build output through so the user sees what's happening.
info "compiling ministr-cli (cached after first run)"
cargo build -q -p ministr-cli

# Deliberately DON'T set MINISTR_CORPUS_PATHS — we want serve to
# start with zero corpora so the demo can register one AFTER it
# attaches to the progress stream, making the SSE actually tick
# instead of reporting "done in 0s".
cargo run -q -p ministr-cli -- \
    serve --transport http --oauth \
    --host 127.0.0.1 --port "${PORT}" \
    > "${SERVE_LOG}" 2>&1 &
SERVE_PID=$!
info "serve PID ${SERVE_PID} — logs at ${SERVE_LOG}"

step "step 4 / 7 — waiting for /healthz"
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
done_step "cloud is live"
curl -sS "${ENDPOINT}/healthz" | sed 's/^/    /'
echo

step "step 5 / 7 — minting a bearer token via the OAuth self-issuer"
REG=$(curl -sS -X POST "${ENDPOINT}/oauth/register" \
    -H 'content-type: application/json' \
    -d '{"redirect_uris":["http://127.0.0.1:0/cb"],"client_name":"demo-local","token_endpoint_auth_method":"none"}')
CID=$(printf '%s' "${REG}" | sed -n 's/.*"client_id":"\([^"]*\)".*/\1/p')
info "registered demo client client_id=${CID}"

VER=$(openssl rand -base64 64 | tr -d '=\n' | tr '+/' '-_' | head -c 64)
CHAL=$(printf '%s' "${VER}" | openssl dgst -binary -sha256 | base64 | tr -d '=\n' | tr '+/' '-_')
ST=$(openssl rand -hex 16)

LOC=$(curl -sS -o /dev/null -w '%{redirect_url}' \
    "${ENDPOINT}/oauth/authorize?response_type=code&client_id=${CID}&redirect_uri=http%3A%2F%2F127.0.0.1%3A0%2Fcb&code_challenge=${CHAL}&code_challenge_method=S256&state=${ST}&scope=ministr%3Aread%20ministr%3Awrite")
CODE=$(printf '%s' "${LOC}" | sed -n 's/.*[?&]code=\([^&]*\).*/\1/p')
info "consent auto-approved, auth code received"

TOKEN_JSON=$(curl -sS -X POST "${ENDPOINT}/oauth/token" \
    -H 'content-type: application/x-www-form-urlencoded' \
    --data-urlencode "grant_type=authorization_code" \
    --data-urlencode "code=${CODE}" \
    --data-urlencode "redirect_uri=http://127.0.0.1:0/cb" \
    --data-urlencode "client_id=${CID}" \
    --data-urlencode "code_verifier=${VER}")
TOKEN=$(printf '%s' "${TOKEN_JSON}" | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
if [[ -z "${TOKEN}" ]]; then
    echo "token exchange failed — server response:" >&2
    echo "${TOKEN_JSON}" >&2
    exit 1
fi
done_step "bearer token acquired"

step "step 6 / 7 — registering the sample dir as a corpus on the cloud"
# POST /api/v1/corpora — kicks off indexing on the server side.
# The serve background loop will start ingesting the sample
# immediately; we capture the assigned corpus_id so the demo CLI
# below knows what to watch.
REGISTER_BODY=$(printf '{"paths":["%s"],"display_name":"demo-source"}' "${SAMPLE_DIR}")
REGISTER_RESP=$(curl -sS -X POST "${ENDPOINT}/api/v1/corpora" \
    -H "authorization: Bearer ${TOKEN}" \
    -H "content-type: application/json" \
    -d "${REGISTER_BODY}")
CORPUS_ID=$(printf '%s' "${REGISTER_RESP}" | sed -n 's/.*"corpus_id":"\([^"]*\)".*/\1/p')
if [[ -z "${CORPUS_ID}" ]]; then
    echo "register_corpus did not return a corpus_id — body was:" >&2
    echo "${REGISTER_RESP}" >&2
    exit 1
fi
done_step "corpus_id=${CORPUS_ID} (indexing kicked off server-side)"

step "step 7 / 7 — \`ministr cloud demo\` attaching to the live progress stream"
echo
# --corpus skips the auto-pick logic and points the watcher at the
# corpus we just registered. The SSE stream now ticks in real time
# as the embedder works through the sample files.
cargo run -q -p ministr-cli -- cloud demo \
    --endpoint "${ENDPOINT}" \
    --token "${TOKEN}" \
    --corpus "${CORPUS_ID}"

echo
printf "${C_GREEN}${C_BOLD}━━ demo complete ━━${C_RESET}\n"
echo
info "serve logs: ${SERVE_LOG}"
info "blob backend root: ${MINISTR_BLOB_FS_ROOT}"
info "tear down: cleanup runs automatically below (unless KEEP=1)"
