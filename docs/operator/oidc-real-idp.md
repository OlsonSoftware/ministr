
# OIDC SSO — wiring a real Identity Provider

Operator-mode runbook for configuring ministr's cloud OIDC sign-in
against a production Identity Provider (`IdP`). Covers:

- [Keycloak](#keycloak) — self-hosted open source
- [Auth0](#auth0) — managed SaaS (Okta acquired Auth0 in 2021;
  the steps below also work for `developer.okta.com` apps with one
  field rename)
- [Google Workspace](#google-workspace) — Google Sign-In for orgs

The flow is the same for every `IdP`: register an OIDC application
in the `IdP`'s admin UI, copy three values into ministr's per-org
config, and your users get SSO. The provider-specific sections
below differ only in *where the values live* inside each admin
console.

## What ministr expects

| Field | Required | Notes |
| --- | --- | --- |
| `issuer_url` | yes | Must serve a valid OIDC discovery doc at `${issuer_url}/.well-known/openid-configuration`. The `issuer` value inside that document MUST byte-match `issuer_url` — OIDC Discovery 1.0 §4.3. |
| `client_id` | yes | Public identifier the `IdP` assigns to your OIDC application. |
| `client_secret` | yes | Confidential string the `IdP` issues alongside the `client_id`. ministr stores it in `org_oidc_configs.client_secret`; every HTTP read returns the sentinel `[REDACTED]`. |
| `groups_claim` | no | JSON claim name the `IdP` uses for group membership. Default `groups`. Read at callback by the role-mapping path. |
| `group_role_map` | no | JSON object mapping `IdP` group name → ministr role (`"owner"` / `"admin"` / `"member"`). Default `{}` (no role inference). The callback intersects this with the user's groups claim; highest-power role wins. Bootstrap-safe: an existing org owner is never downgraded by a less-privileged group mapping. |
| `email_claim` | no | JSON claim name carrying the user's email. Default `email`. |
| `name_claim` | no | JSON claim name carrying the display name. Default `name`. |
| `enforce_email_verified` | no | When `true` (default), reject sign-in if the ID token's `email_verified` claim is anything other than `true`. Flip to `false` only if your `IdP` doesn't ship a verified-email signal AND you trust the `IdP` to vet emails. |

The redirect URI ministr advertises to the `IdP` is:

```
${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/callback
```

where `MINISTR_CLOUD_BASE_URL` is the env var the cloud reads at
boot (see `cmd_serve_http` and `ministr-cloud/src/oidc.rs`'s
`build_redirect_uri`). Configure exactly this URL in your `IdP`;
trailing slashes, scheme mismatches, and wildcard substitutions
will fail.

Scopes requested at `/oidc/login`: `openid email profile`. Your
`IdP` must admit all three.

Claims read at callback: `email` (required — sign-in rejects without
it), `email_verified` (consulted when `enforce_email_verified=true`),
and the configured `groups_claim` (consulted by the role mapping
when `group_role_map` is non-empty; missing or non-array values fall
through to the no-role-inference path without erroring).

## Prerequisites

- A ministr cloud deployment with `MINISTR_PG_URL` and
  `MINISTR_CLOUD_BASE_URL` set. `MINISTR_CLOUD_BASE_URL` must be
  the **public** URL your end users will hit (e.g.
  `https://mcp.example.com`); `IdP`s validate the redirect URI
  against what they have registered, and your dev `127.0.0.1:8088`
  won't be reachable from the `IdP`'s server-side fetch.
- An org with at least one owner. Get an owner bearer:
  ```sh
  curl -X POST "${MINISTR_CLOUD_BASE_URL}/api/v1/orgs" \
      -H "authorization: Bearer ${OWNER_BEARER}" \
      -H "content-type: application/json" \
      -d '{"name":"acme"}'
  # Note the `id` in the response; that's the {org_id} below.
  ```
- `curl` and `jq` for the example commands. Substitute your own
  HTTP client / Postman / Insomnia if preferred.

## Keycloak

Keycloak's admin UI varies between minor versions; the field names
below are stable across 22.x–26.x.

### 1. Create an OIDC client

1. Sign in to the Keycloak admin console.
2. Select the realm your users live in (or create one).
3. **Clients → Create client**.
4. **General settings**:
   - **Client type**: `OpenID Connect`
   - **Client ID**: pick a stable string, e.g. `ministr-cloud`.
   - Click **Next**.
5. **Capability config**:
   - **Client authentication**: `On` (this gives you a
     `client_secret`).
   - **Standard flow**: `On` (authorization code grant).
   - Implicit + Direct access + Service accounts: all `Off`.
   - Click **Next**.
6. **Login settings**:
   - **Root URL**: `${MINISTR_CLOUD_BASE_URL}`
   - **Valid redirect URIs**:
     `${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/callback`
   - Web origins: leave default `+`.
   - Click **Save**.

### 2. Extract `client_secret`

1. Open the client you just created.
2. **Credentials** tab → copy **Client secret**.

### 3. Find `issuer_url`

Keycloak's discovery endpoint is at:

```
${KEYCLOAK_BASE}/realms/${REALM}/.well-known/openid-configuration
```

So `issuer_url` is `${KEYCLOAK_BASE}/realms/${REALM}` (no trailing
slash). Verify by fetching the JSON and checking the `issuer` field
matches byte-for-byte:

```sh
curl -s "${KEYCLOAK_BASE}/realms/${REALM}/.well-known/openid-configuration" | jq .issuer
# Must equal whatever you paste as `issuer_url` below.
```

### 4. POST the config to ministr

```sh
curl -X POST "${MINISTR_CLOUD_BASE_URL}/api/v1/orgs/{org_id}/oidc/config" \
    -H "authorization: Bearer ${OWNER_BEARER}" \
    -H "content-type: application/json" \
    -d '{
      "issuer_url": "https://keycloak.example.com/realms/acme",
      "client_id": "ministr-cloud",
      "client_secret": "PASTE-FROM-CREDENTIALS-TAB"
    }'
```

Expect HTTP 200 with the row JSON. `client_secret` in the response
will be `[REDACTED]` — your real secret is in
`org_oidc_configs.client_secret` in Postgres.

### 5. Verify

In a browser, visit:

```
${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/login
```

You should be redirected to Keycloak's authorize endpoint, see
the login screen, sign in, get redirected back to
`${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/callback?code=…&state=…`,
and finally land on a JSON response carrying
`{token, user_id, plan_id}`. The `token` is a ministr bearer; use
it for subsequent API calls.

## Auth0

Same steps work for any tenant on `${TENANT}.auth0.com` or
`developer.okta.com` — Okta absorbed Auth0 in 2021 and the
Application API surface is nearly identical.

### 1. Create an OIDC application

1. Sign in to the Auth0 dashboard (or Okta dev console).
2. **Applications → Applications → Create Application**.
3. **Name**: e.g. `ministr-cloud`.
4. **Type**: **Regular Web Applications** (server-side; the
   authorization code grant requires a client secret).
5. Click **Create**.

### 2. Configure URIs

In the new application's **Settings** tab:

- **Allowed Callback URLs**:
  `${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/callback`
- **Allowed Logout URLs**: leave blank (ministr doesn't drive
  Auth0's RP-initiated logout in v0).
- **Allowed Web Origins**: leave blank.

Scroll to the bottom and **Save Changes**.

### 3. Extract credentials

On the same **Settings** tab:

- **Domain**: e.g. `acme.us.auth0.com`. The `issuer_url` is
  `https://${domain}` (note: HTTPS, no trailing slash).
- **Client ID**: copy verbatim.
- **Client Secret**: click to reveal → copy verbatim.

### 4. POST the config

```sh
curl -X POST "${MINISTR_CLOUD_BASE_URL}/api/v1/orgs/{org_id}/oidc/config" \
    -H "authorization: Bearer ${OWNER_BEARER}" \
    -H "content-type: application/json" \
    -d '{
      "issuer_url": "https://acme.us.auth0.com",
      "client_id": "PASTE-CLIENT-ID",
      "client_secret": "PASTE-CLIENT-SECRET"
    }'
```

### 5. Verify

Identical to Keycloak's verification step.

### Okta-specific tweak

If you're configuring against a `developer.okta.com` tenant rather
than Auth0, the only difference is the issuer URL shape:

```
https://${SUBDOMAIN}.okta.com/oauth2/default
# or, for a custom authorization server:
https://${SUBDOMAIN}.okta.com/oauth2/${AS_ID}
```

Always confirm by fetching the discovery doc and reading the
`issuer` field.

## Google Workspace

Google Sign-In for Workspace domains uses Google's standard OIDC
endpoints; ministr treats it like any other `IdP`.

### 1. Create OAuth client credentials

1. Sign in to the [Google Cloud Console](https://console.cloud.google.com)
   with a Workspace admin account.
2. Select (or create) a project. The project doesn't have to
   match your Workspace org; it's just where the OAuth client
   credentials live.
3. **APIs &amp; Services → Credentials → + Create credentials → OAuth client ID**.
4. **Application type**: **Web application**.
5. **Name**: e.g. `ministr-cloud`.
6. **Authorized redirect URIs → + Add URI**:
   `${MINISTR_CLOUD_BASE_URL}/orgs/{org_id}/oidc/callback`
7. **Create**.

### 2. Extract credentials

The success modal shows:

- **Client ID** (suffix `.apps.googleusercontent.com`)
- **Client secret**

Both can be re-downloaded later as a JSON file under the same
**Credentials** view.

### 3. Restrict to your Workspace domain

By default, the client accepts sign-ins from *any* Google account.
For a Workspace-only deployment:

1. **OAuth consent screen → User type → Internal**.
2. Add the OAuth client's redirect URI under **Authorized domains**.

Note: **Internal** is only selectable when you're signed in as a
Workspace admin; personal Gmail accounts get the **External** path
with a consent-screen review.

### 4. POST the config

Google's OIDC issuer is fixed:

```sh
curl -X POST "${MINISTR_CLOUD_BASE_URL}/api/v1/orgs/{org_id}/oidc/config" \
    -H "authorization: Bearer ${OWNER_BEARER}" \
    -H "content-type: application/json" \
    -d '{
      "issuer_url": "https://accounts.google.com",
      "client_id": "PASTE-CLIENT-ID.apps.googleusercontent.com",
      "client_secret": "PASTE-CLIENT-SECRET"
    }'
```

### 5. Verify

Same as the other providers. Users will see Google's standard
"Choose an account" picker; only accounts in your Workspace domain
admit (when you set Internal in step 3).

## Troubleshooting

Each error below is something you can hit *for real* with the
the OIDC callback path. The fix column maps to a concrete next step.

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `discover_async: …Validation error: unexpected issuer URI` | `issuer_url` in the config doesn't byte-match the `issuer` field in the IdP's discovery doc. | Fetch `${issuer_url}/.well-known/openid-configuration` and copy the `issuer` field verbatim back into the config. Common mismatch: `https://` vs `http://`, trailing slash, hostname alias (`example.com` vs `www.example.com`). |
| `id_token's email_verified claim is false` | IdP didn't mark this user's email verified. | Either fix the IdP (preferred — admit only verified users) or POST the config again with `"enforce_email_verified": false` if you trust the IdP's own vetting. |
| `id_token carries no email claim` | The IdP's `email` scope wasn't granted or the claim is namespaced. | Confirm the `email` scope is admitted by the IdP's client config. If the IdP issues a non-standard claim name, point `email_claim` at it (the callback reads this column; the OIDC SDK exposes the standard `email` claim via accessor, so non-standard names won't surface in v0 — a future release extends the callback to honour the column). |
| `id_token validation: …Failed to verify ID token` | Signature mismatch (JWKS rotated mid-flight), `aud` mismatch (`client_id` you POSTed doesn't match the JWT's `aud` claim), or clock skew (>5 min off the IdP). | Re-check `client_id`; ensure pod clocks are in sync via NTP; force a JWKS refresh by waiting up to 1h (the discovery cache TTL) or restarting the serve pod. |
| `unknown or expired state` | The browser hit `/oidc/callback` more than 10 minutes after `/oidc/login`, or the pod restarted between the two steps. | Re-initiate at `/oidc/login`. The PendingLogin map is in-memory only; The pending-login map is in-memory only. A future release may move it to Postgres if cross-pod resilience is needed. |
| `oidc config not found for org` (404) | No `org_oidc_configs` row for this `org_id`. | POST the config (Step 4 above). |
| `forbidden` (403) on POST/GET/DELETE | Bearer's subject isn't an owner or admin of `org_id`. | Confirm the user is in `org_members` with role `owner` or `admin`. Lookup: `SELECT role FROM org_members WHERE org_id = '{org_id}'::uuid AND user_id = '{user_id}'::uuid;` |

## Removing the config

Disable OIDC SSO for an org without affecting other sign-in paths:

```sh
curl -X DELETE "${MINISTR_CLOUD_BASE_URL}/api/v1/orgs/{org_id}/oidc/config" \
    -H "authorization: Bearer ${OWNER_BEARER}"
# Returns HTTP 204 on success, 404 if no row was present.
```

After this, `/orgs/{org_id}/oidc/login` returns 404 and users fall
back to whatever other auth paths the org has configured (GitHub
sign-in, SAML if SAML is configured, etc).

## Reference

- Code: [ministr-cloud/src/oidc.rs](../../ministr-cloud/src/oidc.rs) — the
  cloud-side endpoints (`handle_login`, `handle_callback`,
  `handle_oidc_config_upsert/get/delete`).
- Migration: [ministr-cloud/migrations/0011_org_oidc_configs.sql](../../ministr-cloud/migrations/0011_org_oidc_configs.sql) —
  table schema.
- Spec: [OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html)
  and [OpenID Connect Discovery 1.0](https://openid.net/specs/openid-connect-discovery-1_0.html)
  are the contracts ministr conforms to.
- Library: [openidconnect-rs](https://docs.rs/openidconnect) — the
  Rust client crate ministr uses for discovery + token exchange +
  ID-token validation.
