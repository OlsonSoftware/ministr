# Demo — running the cloud locally

End-to-end recipe for standing up `ministr` cloud-mode on your laptop:
Postgres in Docker, the Rust cloud binary on `localhost:8080`, and
the Tauri desktop app pointed at it. Verifies that the F0–F2 stack
actually works against real vendor accounts (Stripe + GitHub) end to
end before you push to Azure.

> **Time budget**: ~30 minutes the first time (mostly Stripe + GitHub
> registration), ~30 seconds for subsequent restarts.

---

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Docker | 24+ | Local Postgres |
| Rust | 1.88+ | Workspace requires the 2024 edition |
| `just` | 1.x | Recipe runner |
| `pnpm` | 9+ | Desktop frontend |
| `stripe-cli` | latest | Forwards Stripe webhooks to localhost |

```sh
brew install docker just pnpm stripe/stripe-cli/stripe   # macOS
```

---

## 1. Bring up Postgres

```sh
just dev-cloud-up
```

This runs `docker compose -f docker-compose.dev.yml up -d`. Postgres
listens on `localhost:55432` (deliberately not 5432 so it doesn't
collide with a system Postgres).

Verify:

```sh
just dev-cloud-psql
\dt   # no tables yet — that's expected; the cloud auto-migrates on first serve
\q
```

---

## 2. Register a GitHub OAuth App

For LOCAL dev — production uses a **separate** App.

1. Go to <https://github.com/settings/applications/new>.
2. Fill in:
   - **Application name**: `ministr cloud (local)`
   - **Homepage URL**: `http://localhost:8080`
   - **Authorization callback URL**:
     `http://localhost:8080/auth/github/callback`
3. Click **Register application**.
4. Copy the **Client ID** (`Iv1.…`).
5. Click **Generate a new client secret** → copy it.

You now have `MINISTR_GITHUB_CLIENT_ID` + `MINISTR_GITHUB_CLIENT_SECRET`.

---

## 3. Set up Stripe (test mode)

You don't need a verified business — Stripe's test mode is free.

1. Sign in at <https://dashboard.stripe.com> (or create an account).
2. Toggle **Test mode** in the top bar.
3. **Developers → API keys** → copy the **Secret key** (`sk_test_…`).
4. **Products → + Add product**:
   - Name: `ministr Pro`
   - Price: `$20 USD` recurring monthly
   - Click **Save product**, then copy the **price ID** (`price_…`).
5. Repeat for `ministr Team` at `$30 USD` recurring monthly.
6. **Developers → Webhooks → Add destination → Local listener**.
   Choose all `customer.subscription.*` events. Don't save yet —
   `stripe-cli` will generate the signing secret in a moment.

Open a terminal:

```sh
stripe login
stripe listen --forward-to localhost:8080/webhooks/stripe
```

Copy the `whsec_…` line `stripe-cli` prints — that's your
`MINISTR_STRIPE_WEBHOOK_SECRET`. Leave this terminal running.

---

## 4. (Optional) Register a GitHub App for private-repo cloning

Skip this on first run — the local demo works without it. PAT-in-URL
clones (`https://USER:PAT@github.com/owner/repo.git`) work without the
App; the App just removes the need to hand the cloud a PAT.

1. <https://github.com/settings/apps/new>.
2. **GitHub App name**: `ministr cloud (local)`
3. **Homepage URL**: `http://localhost:8080`
4. **Webhook**: uncheck "Active" (no webhook needed for the F2.1 demo).
5. **Repository permissions**:
   - Contents: **Read-only**
   - Metadata: **Read-only**
6. Click **Create GitHub App**.
7. Copy the **App ID** (top of the settings page).
8. Click **Generate a private key** → downloads a `.pem` file.

You now have `MINISTR_GITHUB_APP_ID` + the PEM (full multi-line content
goes into `MINISTR_GITHUB_APP_PRIVATE_KEY`).

---

## 5. Configure your env

```sh
cp .env.dev.example .env.dev
$EDITOR .env.dev    # paste in everything from steps 2–4
```

Required for the minimum demo:

- `MINISTR_PG_URL` — leave as-is (points at the Docker Postgres)
- `MINISTR_CLOUD_BASE_URL` — leave as `http://localhost:8080`
- `MINISTR_GITHUB_CLIENT_ID` / `_SECRET` — from step 2

Optional but recommended for end-to-end testing:

- `MINISTR_STRIPE_SECRET_KEY` / `_WEBHOOK_SECRET` / `_PRICE_PRO` /
  `_PRICE_TEAM` — from step 3
- `MINISTR_GITHUB_APP_ID` / `_PRIVATE_KEY` — from step 4

The filesystem blob backend is the dev default — no extra env var
needed unless you want to point it at a specific directory:

```sh
export MINISTR_BLOB_FS_ROOT="$HOME/.ministr/cloud-dev/blobs"
```

---

## 6. Smoke-check before starting

```sh
source .env.dev
just dev-cloud-check
```

`cloud check` probes every wired integration and prints a tick/cross
table. Fix any rows that come back red before going further — the
cloud will refuse to mount the corresponding handler if its env vars
are malformed.

---

## 7. Start the cloud

```sh
source .env.dev
cargo run -p ministr-cli -- serve --transport http --oauth
```

You should see:

```
cloud postgres migrations applied
OAuth 2.1 authentication enabled
billing endpoint mounted — GET /api/v1/billing/usage
stripe checkout + portal mounted — POST /api/v1/billing/{checkout,portal}
github sign-in mounted — GET /auth/github/start, /auth/github/callback
atlas v0 mounted — GET /atlas/manifest.json + /atlas/{slug}/*
ministr HTTP server listening
```

Verify the basics:

```sh
curl http://localhost:8080/healthz
# {"status":"ready","corpus_count":0,"version":"0.6.0"}

curl http://localhost:8080/atlas/manifest.json | jq .count
# 50
```

---

## 8. Point the Tauri app at it

```sh
just dev    # or: cd ministr-app && pnpm tauri dev
```

In the app:

1. Open **Settings → Cloud**.
2. **Endpoint**: `http://localhost:8080`
3. Click **Save endpoint**.
4. Click **Sign in with GitHub**. Browser opens → GitHub consent →
   redirects back → the keychain now holds a bearer token.
5. The **Onboarding wizard** at the top should show step 1 complete.

---

## 9. Walk the happy path

| Step | What | Verify |
|---|---|---|
| Sign in | Onboarding step 1 | Plan badge appears (`Pro` by default) |
| Upgrade | Onboarding step 2 → click *Upgrade to Pro* | Browser opens Stripe Checkout; `stripe-cli` logs the webhook |
| Clone first repo | Onboarding step 4 → enter a public Git URL | Repo appears in the corpora list, indexing kicks off |
| Atlas manifest | Browser: `http://localhost:8080/atlas/manifest.json` | 50 entries returned |
| Atlas query | Browser: `http://localhost:8080/atlas/react/survey?query=hooks` | Returns 503 `atlas_not_indexed_yet` (expected for F2.6 v0) |

When you hit Stripe Checkout with the test card `4242 4242 4242 4242`
(any future exp + any 3-digit CVC), the webhook fires and your
`users.plan_id` flips to `pro`. Confirm:

```sh
just dev-cloud-psql
SELECT email, plan_id, stripe_customer_id FROM users;
```

---

## 10. Common surprises

- **Migrations not applied** — the cloud auto-migrates on `serve`
  startup. If you see "relation users does not exist" the migrations
  step failed; check the log for the `cloud postgres migrations
  applied` line.
- **GitHub OAuth callback mismatch** — the App's callback URL must
  be *exactly* `http://localhost:8080/auth/github/callback`. Missing
  the path or the port reproduces as a GitHub error page after
  consent.
- **Stripe webhooks silently ignored** — `stripe-cli` must keep
  running in its own terminal; otherwise `customer.subscription.*`
  events never reach the cloud.
- **Tauri keychain prompt every restart** — first launch on macOS
  asks for permission. Choose "Always Allow" so subsequent restarts
  don't reprompt.
- **Atlas query returns 503** — this is *expected* in F2.6 v0; the
  real indexer ships in F4.2. The manifest endpoint is the meaningful
  Atlas demo right now.

---

## 11. Tearing down

```sh
# Stop the cloud binary (Ctrl-C in its terminal).
# Stop stripe-cli (Ctrl-C in its terminal).

just dev-cloud-down       # preserves the Postgres volume
just dev-cloud-reset      # nukes the volume (start clean next time)
```

---

## 12. Going to production

Out of scope for this doc — see the `deploy/azure/` Pulumi stack.
TL;DR:

- Provision Azure Postgres Flex + Storage + Container Apps.
- Register **separate** GitHub OAuth App + GitHub App with
  `https://mcp.ministr.ai/...` callbacks.
- Switch Stripe to live mode + recreate the Pro/Team products there.
- Set `MINISTR_BLOB_STORE_KIND=azure` plus
  `MINISTR_BLOB_AZURE_ACCOUNT` + `MINISTR_BLOB_AZURE_CONTAINER`.
- DNS: `mcp.ministr.ai` → ACA ingress.
- Domain split: `ministr.ai` (docs-next on GitHub Pages) +
  `mcp.ministr.ai` (cloud binary) — see ROADMAP §3 and the existing
  `docs-next/public/CNAME` for the static-site side.

---

## 13. Demo against your Azure deployment

`just demo-remote` is the cloud analogue of `just demo-local` — it
points the same `ministr cloud demo` client at your live Azure
container and watches a real repo get cloned + indexed end-to-end.

### Run the demo (two commands)

```sh
just azure-init    # one-time: npm ci + pulumi stack init prod
just azure-demo    # provision (if needed) + push + roll + demo-remote
```

`azure-demo` handles both fresh-deploy AND subsequent runs:

- Fresh stack: runs `pulumi up` to provision everything (~5-7 min),
  then pushes the image, then `pulumi up` again to roll the revision,
  then runs `demo-remote`.
- Subsequent runs: skips the initial provisioning, just pushes the
  current code (tagged with the git sha), bumps the Pulumi `imageTag`,
  rolls the revision, and runs the demo.

Day-to-day after code changes, `just azure-demo` is the only command
you need.

### Other recipes

| Recipe | What it does |
|---|---|
| `just azure-init` | One-time: npm ci + `pulumi stack init prod` |
| `just azure-push` | Build + push image tagged with current git sha; bump pulumi config |
| `just azure-up` | `pulumi up` against the prod stack |
| `just azure-demo` | One-shot: provision (if needed) + push + roll + `demo-remote` |
| `just azure-status` | `pulumi stack output` + `/healthz` probe |
| `just azure-logs` | Tail live ACA container logs |
| `just azure-down` | Tear down the entire stack (asks for confirmation) |
| `just demo-remote` | Just the demo step — assumes the cloud is already deployed |

Swap the demo repo with `CLONE_URL=https://github.com/owner/repo.git just demo-remote`.

### Optional: custom domain

To use `mcp.ministr.ai` instead of the default ACA FQDN, set the
Pulumi config BEFORE the first apply:

```sh
pulumi -C deploy/azure config set customDomain mcp.ministr.ai
```

Then add a `CNAME` in DNS pointing your domain to the ACA managed-env
FQDN (printed as `appFqdn` after the first apply). The managed cert
provisions automatically within ~5 min once DNS resolves.

The script will:

1. Resolve the URL from `pulumi stack output publicBaseUrl` (or
   from `MINISTR_CLOUD_BASE_URL` if set).
2. Probe `/healthz`.
3. Mint a bearer token via the cloud's OAuth self-issuer
   (auto-consent — no IdP wired in MVP scope).
4. Hand off to `ministr cloud demo --clone-url …` which uses the
   auto-registered `/data/corpus` corpus as the clone parent, kicks
   off a server-side clone + index, and streams the SSE progress
   feed back into your terminal.

### Cost note

The MVP wiring (no Postgres, no Blob container, no Stripe, no IdP)
keeps you at the **~$25/mo baseline** of the existing Pulumi stack.
The cloud auto-disables every F1.3+ vendor route when its env vars
aren't set, so this is genuinely minimum-viable — no half-wired
billing or auth surfaces are reachable.

To opt into the next tier, flip `enablePostgres true` in Pulumi
config (adds ~$13/mo) and re-apply; the cloud picks up
`MINISTR_PG_URL` from the stack output and auto-mounts billing +
quota + rate-limit + Atlas + Stripe webhook + GitHub sign-in
routes (each independently env-gated — see `cmd_serve_http`).
