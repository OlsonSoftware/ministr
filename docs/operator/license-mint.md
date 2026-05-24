
# Issuing Enterprise license JWTs

ministr's on-prem distribution is license-gated: customers
configure `MINISTR_LICENSE_KEY` (the JWT) + `MINISTR_LICENSE_PUBLIC_KEY`
(the verification key) and the serve's boot gate validates
the JWT against the pubkey before starting. This doc covers the
ops-side flow: how YOU generate the signing keypair and mint
licenses for customers.

The CLI primitives are available today; an admin UI
is planned
add the audit log + revocation table.

## One-time setup: generate the signing keypair

Run this ONCE per ministr deployment. Stash the private key in
your secrets manager (Vault / KMS / 1Password / etc); the public
key ships to every Enterprise customer.

```bash
ministr cloud generate-license-keypair \
  --private-key /secure/ministr-license-private.pem \
  --public-key  /secure/ministr-license-public.pem
```

Defaults to RSA-2048 (NIST SP 800-131A minimum). Override via
`--bits 3072` or `--bits 4096` if your contract demands it; larger
sizes are slower without meaningful 2026 security uplift.

The command:
- Refuses to overwrite existing files (move the old keys aside
  first if you're rotating).
- chmods the private key to 0600 on POSIX. On Windows you must
  rely on directory ACLs (silent no-op).
- Writes the public key with default 0644 perms (it's not secret).

## Per-license issuance: mint a JWT

For each Enterprise customer at contract-signing time:

```bash
ministr cloud mint-license \
  --private-key /secure/ministr-license-private.pem \
  --enterprise-id "acme-corp" \
  --seat-count 50 \
  --valid-days 365 \
  --out /tmp/acme-corp-license.jwt
```

The flags:

| Flag | Purpose | Validation |
|------|---------|------------|
| `--private-key` | Path to the keypair's private side | File must exist |
| `--enterprise-id` | Identifies the customer in their boot log | Must be non-empty |
| `--seat-count` | seat-cap enforcement | u32 |
| `--valid-days` | Days from now to `exp` | Must be > 0 (use `mint-test-license` for expired fixtures) |
| `--out` | Optional file path to write the JWT to | Defaults to stdout |

The minted JWT is opaque to the customer (RS256-signed; can't be
edited without re-signing). Distribute via your CRM / encrypted email
/ secure-share to the customer's ops contact alongside the public
key from the keypair-generation step.

## Customer-side setup

The customer pastes both values into their `MINISTR_LICENSE_KEY` +
`MINISTR_LICENSE_PUBLIC_KEY` env vars. The Helm chart takes them
via `values.yaml`:

```yaml
license:
  key: eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.PASTE_JWT_HERE...
  publicKey: |
    -----BEGIN PUBLIC KEY-----
    PASTE_PEM_HERE
    -----END PUBLIC KEY-----
```

Docker Compose takes them via `.env`.

The customer's serve boot logs an "Enterprise license validated"
line on success — confirms they pasted the right
values. Wrong-signature or expired tokens refuse boot with a clear
miette error identifying the license gate as the cause.

## Renewal flow

There's no separate renewal command — issue a fresh JWT against
the same keypair with a new `--valid-days`. The customer
overwrites `MINISTR_LICENSE_KEY` and restarts their pods.

## Audit log

Pass `--audit-log PATH` to `mint-license` and a JSONL line is
appended per successful mint:

```bash
ministr cloud mint-license \
  --private-key /secure/ministr-license-private.pem \
  --enterprise-id "acme-corp" \
  --seat-count 50 --valid-days 365 \
  --audit-log /secure/ministr-license-issuances.jsonl \
  --out /tmp/acme-corp-license.jwt
```

Each line records: `ts_iso`, `ts_unix`, `enterprise_id`, `seat_count`,
`valid_days`, `exp`, and `jwt_id_hash` (first 16 hex chars of
`sha256(jwt)`). **The bearer material is NOT stored** — only its
hash, sufficient to disambiguate human-readable list output.

Append-only on POSIX (atomic for writes ≤ 4 KB; each line is well
under). Concurrent multi-host writes would interleave half-lines —
documented as a single-operator-host limitation. The audit-log
write happens BEFORE the JWT is printed/written, so a crash between
mint and audit-write doesn't leave an orphan issuance the operator
can't trace.

Read back via `list-licenses`:

```bash
# Table view (default), sorted most-recent first.
ministr cloud list-licenses --audit-log /secure/ministr-license-issuances.jsonl

# JSON view for piping into jq / further processing.
ministr cloud list-licenses --audit-log /secure/ministr-license-issuances.jsonl --format json
```

Malformed lines from a partial write are skipped with a `warn` log.

Stash the audit log alongside your license private key — both are
operationally-sensitive (the audit log reveals who-bought-what; the
private key signs JWTs). Customer's secrets manager handles
disk-level encryption.

## Revocation flow

When a customer contract terminates or a license key is compromised,
revoke the JWT so the customer's serve refuses to boot under it even
though `exp` may still be in the future.

```bash
ministr cloud revoke-license \
  --jwt /tmp/acme-corp-license.jwt \
  --enterprise-id "acme-corp" \
  --reason "contract terminated 2026-12-01" \
  --revocation-list /secure/ministr-license-revocations.jsonl
```

Or, if you no longer have the JWT file but the audit log has the
hash:

```bash
ministr cloud revoke-license \
  --jwt-id-hash abcdef0123456789 \
  --enterprise-id "acme-corp" \
  --reason "key compromise reported 2026-12-01" \
  --revocation-list /secure/ministr-license-revocations.jsonl
```

Each invocation appends one JSONL record carrying `ts_iso`,
`ts_unix`, `enterprise_id`, `jwt_id_hash`, and `reason`. Distribute
the updated revocation list to the customer via the same channel as
the license itself; the customer points
`MINISTR_LICENSE_REVOCATIONS=/path/to/revocations.jsonl` at the file
and restarts their pods. On boot, the serve refuses to start with:

```
license revoked at gate: hash=abcdef0123456789 reason=contract terminated 2026-12-01
```

Helm chart reads the path from `values.yaml`:

```yaml
license:
  key: …
  publicKey: …
  revocationsPath: /etc/ministr/revocations.jsonl
```

Docker Compose reads it from `.env` as
`MINISTR_LICENSE_REVOCATIONS`. The env var is **opt-in**: customers
who never receive a revocation list operate unchanged from the
The boot validator — only customers whose ops contact has been told
"point at this file" enable the enforcement path.

Hash-stability guarantee: `jwt_id_hash` is computed as the first 16
hex characters of `sha256(jwt)`, identical to the hash in the
the audit log. Revoke from either source (the JWT file or the
audit log) and you get the same record.

Stash the revocation list alongside your license private key + audit
log — all three are operationally sensitive (private key signs JWTs,
audit log reveals who-bought-what, revocation list reveals contract
churn). Customer's secrets manager handles disk-level encryption.

## Customer-side HTTP fetch

The complement to the server-side revocation endpoint.
When the customer sets `MINISTR_LICENSE_REVOCATIONS_URL`, the boot
validator fetches the operator's revocation list over HTTP, caches
it locally, falls back to the cache within a grace window if the
fetch fails, and refuses boot beyond grace.

Three env vars:

| Var | Default | Purpose |
|-----|---------|---------|
| `MINISTR_LICENSE_REVOCATIONS_URL` | unset | When set, takes precedence over `MINISTR_LICENSE_REVOCATIONS` file path. Operator-published URL. |
| `MINISTR_LICENSE_REVOCATIONS_CACHE_PATH` | `/tmp/ministr-revocations-cache.jsonl` | Where the fetcher writes the body on success; reads on fallback. |
| `MINISTR_LICENSE_REVOCATIONS_GRACE_SECS` | `86400` (24h) | Cache mtime cap. If fetch fails AND cache is older than this, refuse boot. |

Boot flow when URL is set:

1. **Fetch** with a 10s timeout. On 2xx, write body to
   `_CACHE_PATH`, consult the cache for the boot license's hash.
2. **On non-2xx / network error WITH fresh cache** (mtime ≤
   `_GRACE_SECS`): log a warning, use the cache. The serve boots
   under the slightly-stale list — better than refusing boot on a
   transient portal blip.
3. **On failure WITH stale or missing cache**: refuse boot with
   `LicenseError::RevocationFetchFailed`. Operator opted into
   network-fetched revocation; falling back to "no revocation
   check" would silently allow a revoked license to keep running.

Customer-side example (Helm values, Docker Compose `.env`, or bare
env):

```bash
export MINISTR_LICENSE_KEY="eyJhbGc..."
export MINISTR_LICENSE_PUBLIC_KEY="-----BEGIN PUBLIC KEY-----..."
export MINISTR_LICENSE_REVOCATIONS_URL="https://mcp.ministr.ai/api/v1/license-revocations.jsonl"
export MINISTR_LICENSE_REVOCATIONS_CACHE_PATH="/var/lib/ministr/revocations.jsonl"
export MINISTR_LICENSE_REVOCATIONS_GRACE_SECS="86400"  # default; 24h
ministr serve --transport http --port 8080
```

Mode coexistence: when both `_URL` and `_REVOCATIONS` (file) are
set, **URL wins** — the file fallback is for customers who never
opted into URL-based fetch. Operators transitioning from
file-based to network-fetched can simply add the URL env without
unsetting the old one.

### Background refresh

When `MINISTR_LICENSE_REVOCATIONS_URL` is set, the customer's serve
spawns a background tokio task that re-fetches the URL every
`MINISTR_LICENSE_REVOCATIONS_REFRESH_SECS` (default 3600 = 1 hour).
On each tick the task overwrites the cache file. This keeps the
cache warm so the NEXT pod restart's boot validator sees fresh
revocations even if the portal is briefly unreachable at restart
time.

### Mid-flight enforcement

The background refresh task ALSO re-checks the running license's
hash against the just-fetched cache. If the operator revokes the
license while the customer's serve is running, the task detects
it on the next refresh tick and exits the process. The
orchestrator (k8s / Docker / systemd) restarts the pod; the boot
validator refuses the now-revoked license; the pod stays down
(`CrashLoopBackOff` on k8s) until the operator unsets the license
or the customer pulls a new one.

Detection latency = ≤ `MINISTR_LICENSE_REVOCATIONS_REFRESH_SECS`
(default 1 hour). Operators wanting tighter latency lower the
env var — there's no other knob.

Exit code 1 with a clear log line:

```
ERROR running license has been REVOKED by the operator — exiting;
orchestrator must restart to pick up new license
   jwt_id_hash=abcdef0123456789
   reason=contract terminated 2026-12-01
```

Honest scope: `std::process::exit` is brutal — in-flight HTTP
requests get connection-reset rather than HTTP-503'd gracefully.
For graceful 503 + drain, a future release would wire
`axum::serve(...).with_graceful_shutdown(signal)` and a top-level
middleware reading a "license revoked" state flag. Today's
posture is the k8s-friendly one: pod dies → orchestrator notices
in CrashLoopBackOff → operator sees the log line. Operational
end-state is identical (no service under revoked license).

Refresh task errors log `warn` and the loop continues — a transient
portal blip doesn't crash the background task; the cache simply
stays at whatever it was last refreshed to until the portal recovers.

## Serving the revocation list via HTTP

Customers' on-prem serves can fetch the revocation list dynamically
instead of mounting the JSONL file directly. Two opt-ins:

1. **Operator side** — point the public-facing serve at the
   revocation list file:

   ```bash
   export MINISTR_LICENSE_REVOCATIONS_SERVE_PATH=/secure/ministr-license-revocations.jsonl
   ministr serve --transport http ...
   ```

   The serve now exposes `GET /api/v1/license-revocations.jsonl` as
   an **unauthenticated** public endpoint. The revocation list is
   non-secret — a `jwt_id_hash` reveals "this hash is revoked" but
   nothing about the bearer, customer, or original mint context.
   `Cache-Control: public, max-age=300` is set so polling consumers
   don't hammer.

   Read at request time, so updating the file (via
   `revoke-license --revocation-list PATH`) takes effect on the
   next HTTP request without bouncing the serve.

2. **Customer side** — once
   the customer-side fetch feature lands, customers will set
   `MINISTR_LICENSE_REVOCATIONS_URL=https://mcp.ministr.ai/api/v1/license-revocations.jsonl`
   and the boot validator will fetch + cache + grace-window-fall-back.
   Today, the customer-side fetcher hasn't shipped; customers still
   use the file-based `MINISTR_LICENSE_REVOCATIONS=/path` flow.

Response states:

| State | Status | Body |
|-------|--------|------|
| `MINISTR_LICENSE_REVOCATIONS_SERVE_PATH` unset | 404 | "operator hasn't opted in" |
| Set + file readable | 200 | JSONL body, `application/x-ndjson` |
| Set + file unreadable | 503 | "revocation list unreadable: …" |

Operator workflow:

```bash
# Add a revocation to the operator's list
ministr cloud revoke-license \
  --jwt /tmp/acme-corp-license.jwt \
  --enterprise-id "acme-corp" \
  --reason "contract terminated 2026-12-01" \
  --revocation-list /secure/ministr-license-revocations.jsonl

# Customer's serve polls the URL,
# pulls the updated list, and refuses to boot under the revoked hash
# on next restart. Until then, distribute the file by hand.
```

## Multi-operator setup

When several operators issue licenses from different machines, each
one's local JSONL audit log is invisible to the others. The DB-backed
mirror gives them a shared view via the existing cloud Postgres —
no new infrastructure to provision.

Two-step opt-in:

1. **One-time migration** — applied automatically when
   `MINISTR_PG_URL` is set on a serve that hasn't yet run migration
   `0017_license_issuances`. No manual step required on the operator
   side beyond the env var.

2. **Add the flag** to every `mint-license` invocation:

   ```bash
   ministr cloud mint-license \
     --private-key /secure/ministr-license-private.pem \
     --enterprise-id "acme-corp" \
     --seat-count 50 \
     --valid-days 365 \
     --audit-log /secure/ministr-license-issuances.jsonl \
     --pg-url "$MINISTR_PG_URL" \
     --out /tmp/acme-corp-license.jwt
   ```

   Or set `MINISTR_PG_URL` as an env var and the flag falls through
   automatically. Both backends get the same data — JSONL stays the
   file-local truth, PG becomes the multi-operator-visible mirror.

Read the unified history:

```bash
# Table view from DB across ALL operators
ministr cloud list-licenses --pg-url "$MINISTR_PG_URL"

# Or with the env var fall-through
ministr cloud list-licenses --pg-url "$MINISTR_PG_URL" --format json
```

The DB-backed flow uses an `INSERT ... ON CONFLICT DO NOTHING` on
`jwt_id_hash`, so re-running `mint-license` after a transient backend
blip is idempotent — the duplicate insert is silently absorbed. A
crash between the PG write and the JSONL write leaves the row in DB
without a corresponding JSONL line; the operator can spot this via
`list-licenses --pg-url`.

Storage cost: ~120 bytes per issuance × even 10K customers/year ≈
1.2 MB/year. Effectively free against the existing PG flex footprint.

Honest gap: the `list-licenses --pg-url` view doesn't dedupe rows
across JSONL + PG — if both are loaded into a single dashboard, the
DB-mirror's rows could double-count. The CLI surfaces one or the
other per invocation (mutually exclusive flags), so the operator
chooses which source to consume per query.

## Key rotation flow

Rotate your signing keypair when (a) you're on a scheduled rotation
cycle (annual is typical), or (b) the old private key is compromised
and you need to invalidate every license signed with it. The flow:

1. **Generate a fresh keypair** alongside the old one — keep the old
   private key around for the duration of the rotation cycle in case
   you need to re-issue against it for any reason.

   ```bash
   ministr cloud generate-license-keypair \
     --private-key /secure/ministr-license-private-2027.pem \
     --public-key  /secure/ministr-license-public-2027.pem
   ```

2. **Re-mint all in-flight licenses against the new key.** Reads the
   existing audit log, optionally consults the revocation list to
   skip revoked customers, drops naturally-expired records, and
   writes one fresh JWT per surviving enterprise into `--out-dir`.

   ```bash
   ministr cloud rotate-license-keys \
     --audit-log /secure/ministr-license-issuances.jsonl \
     --revocation-list /secure/ministr-license-revocations.jsonl \
     --new-private-key /secure/ministr-license-private-2027.pem \
     --out-dir /tmp/reissued-2027-Q1/ \
     --new-audit-log /secure/ministr-license-issuances-2027.jsonl \
     --valid-days 365
   ```

   Stdout prints a rotation summary:

   ```
   rotation summary — 2 re-issued, 1 skipped (revoked), 0 skipped (expired)

   enterprise_id             out_file
   ------------------------  ----------------------------------------
   acme-corp                 /tmp/reissued-2027-Q1/acme-corp-abc123.jwt
   beta-co                   /tmp/reissued-2027-Q1/beta-co-def456.jwt
   ```

   Filenames are `<enterprise_id>-<short_hash>.jwt` so the same
   enterprise can survive multiple rotations without colliding. The
   new audit log captures the re-issuance side so the rotation cycle
   is itself fully auditable.

3. **Distribute the new artefacts**:
   - Ship `ministr-license-public-2027.pem` to **every** customer (it
     replaces the public key in their `MINISTR_LICENSE_PUBLIC_KEY`
     env var).
   - Ship each per-customer JWT in `/tmp/reissued-2027-Q1/` to the
     respective customer's ops contact via your CRM / encrypted email
     (replaces the `MINISTR_LICENSE_KEY`).

4. **Customers paste both new values** and restart their pods. The
   The boot check accepts the fresh JWT against the new public
   key; the old key + JWT no longer pass validation against the new
   public key (mismatched signature). Customers who haven't pulled
   the new pubkey yet will see boot failures — communicate the
   rotation window clearly.

5. **Retire the old private key** once you're satisfied every
   customer has migrated. The old audit log + the old revocation list
   become historical records; archive them with your other compliance
   artefacts.

What this command **does NOT do**:

- Doesn't email customers automatically — distribution is still
  manual through your CRM (same as the original mint flow).
- Doesn't verify the old licenses' signatures — the audit log is
  the source of truth for what was issued, and we re-mint from its
  metadata, not by parsing the old JWTs.
- Doesn't preserve each license's original time-to-expiry —
  `--valid-days` applies uniformly to every re-issued JWT. If you
  need per-customer horizons, run multiple rotations with different
  audit-log subsets.

## Current limitations

- **No admin UI yet** — an admin webapp will surface this flow
  with templated email distribution. Today it's CLI-only.

These gaps are tractable but not blocking today — operators can issue,
revoke, and rotate licenses with the existing CLI; the gap just means
ops processes (record-keeping, email distribution) live in your CRM
rather than in ministr's tooling.
