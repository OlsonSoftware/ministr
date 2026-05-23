# F5.4-e-mint — issuing Enterprise license JWTs

ministr's on-prem distribution (F5.4) is license-gated: customers
configure `MINISTR_LICENSE_KEY` (the JWT) + `MINISTR_LICENSE_PUBLIC_KEY`
(the verification key) and the serve's F5.4-a boot gate validates
the JWT against the pubkey before starting. This doc covers the
ops-side flow: how YOU generate the signing keypair and mint
licenses for customers.

The F5.4-e-mint chunk ships the CLI primitives; F5.4-e-ui (deferred)
will add an admin UI on top; F5.4-e-audit + F5.4-e-revoke (deferred)
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
| `--seat-count` | F5.4-b's invite-cap | u32 |
| `--valid-days` | Days from now to `exp` | Must be > 0 (use `mint-test-license` for expired fixtures) |
| `--out` | Optional file path to write the JWT to | Defaults to stdout |

The minted JWT is opaque to the customer (RS256-signed; can't be
edited without re-signing). Distribute via your CRM / encrypted email
/ secure-share to the customer's ops contact alongside the public
key from the keypair-generation step.

## Customer-side setup

The customer pastes both values into their `MINISTR_LICENSE_KEY` +
`MINISTR_LICENSE_PUBLIC_KEY` env vars. Helm chart (`F5.4-c`,
`deploy/helm/ministr-enterprise/`) takes them via `values.yaml`:

```yaml
license:
  key: eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.PASTE_JWT_HERE...
  publicKey: |
    -----BEGIN PUBLIC KEY-----
    PASTE_PEM_HERE
    -----END PUBLIC KEY-----
```

Docker Compose (`F5.4-d`, `deploy/compose/`) takes them via `.env`.

The customer's serve boot logs an "Enterprise license validated"
line on success (per F5.4-a) — confirms they pasted the right
values. Wrong-signature or expired tokens refuse boot with a clear
miette error identifying the license gate as the cause.

## Renewal flow

There's no separate renewal command — issue a fresh JWT against
the same keypair with a new `--valid-days`. The customer
overwrites `MINISTR_LICENSE_KEY` and restarts their pods.

## Audit log (F5.4-e-audit)

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

## Honest gaps in this chunk

- **No revocation** — once a JWT is issued it's valid until `exp`
  even if the customer's contract terminates. F5.4-e-revoke will
  add a revocation table the serve checks on each boot (with a
  cached grace window for offline operation).

- **No admin UI** — F5.4-e-ui will surface this flow as a webapp
  with templated email distribution. Today it's CLI-only.

- **No key rotation tooling** — re-running `generate-license-keypair`
  with a different path produces a new keypair, but rotating
  customers off the old key requires re-minting their licenses and
  shipping the new public key. F5.4-e-rotate would automate the
  "mint a new key + reissue all in-flight licenses + email customers
  the new pubkey" cycle.

These gaps are tractable but not blocking today — operators can issue
licenses with the existing CLI; the gaps just mean ops processes
(record-keeping, revocation-on-termination) live in your CRM rather
than in ministr's tooling.
