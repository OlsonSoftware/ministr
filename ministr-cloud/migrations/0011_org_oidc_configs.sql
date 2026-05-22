-- F5.2-a — per-org OIDC (OpenID Connect) SP configuration.
--
-- ministr acts as a Relying Party (RP); each customer org points us
-- at their own OIDC IdP (Keycloak, Auth0, Ory, Google Workspace,
-- Microsoft Entra, ...). One row per org that has OIDC SSO enabled.
--
-- Parallel track to org_saml_configs (migration 0010). The two are
-- independent — an org can have neither, one, or both configured.
-- When both are present, F5.2-c's login chooser surfaces both
-- options to the user; the F5.1-b /saml/login + F5.2-b /oidc/login
-- routes are wired independently.
--
-- Trust anchor: the IdP's `issuer_url` + the JWKS it publishes at
-- `<issuer_url>/.well-known/jwks.json` (fetched at runtime; not
-- pinned in this table since JWKS rotates more often than the IdP
-- entity itself). The library fetches discovery doc at runtime and
-- caches both discovery + JWKS per RFC 8414 / OIDC Discovery 1.0.
--
-- Per-org `client_id` + `client_secret`: the IdP issues these when
-- we register ministr as a client app. Stored server-side only;
-- never exposed to the browser. F5.2-d's CRUD endpoint redacts
-- `client_secret` in GET responses.
--
-- Claim names: standard OIDC defaults (`groups`, `email`, `name`)
-- with per-org overrides for IdPs that emit non-standard claims.
-- The role-mapping JSON (groups → org_member role) lands as its
-- own column in F5.2-d when actually consumed by the callback path.
--
-- Forward-only; idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS org_oidc_configs (
    -- One OIDC config per org. `org_id` is both PK and FK; an org
    -- either has OIDC SSO or doesn't.
    org_id UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,

    -- IdP side (the customer's Keycloak/Auth0/Ory/etc.).
    -- `issuer_url` is the canonical Issuer Identifier per OIDC
    -- Core 1.0 §2; e.g. `https://accounts.google.com` or
    -- `https://acme.okta.com`. Discovery is fetched from
    -- `<issuer_url>/.well-known/openid-configuration` at runtime.
    issuer_url TEXT NOT NULL,

    -- RP credentials issued by the IdP when ministr is registered
    -- as a client app. `client_secret` is server-side only — never
    -- returned by F5.2-d's GET endpoint (redacted to "[REDACTED]").
    client_id TEXT NOT NULL,
    client_secret TEXT NOT NULL,

    -- Claim name overrides. Defaults match the OIDC Core 1.0 +
    -- "groups" Standard Claims convention. Most IdPs emit these
    -- as-is; the override exists for non-standard IdPs.
    groups_claim TEXT NOT NULL DEFAULT 'groups',
    email_claim TEXT NOT NULL DEFAULT 'email',
    name_claim TEXT NOT NULL DEFAULT 'name',

    -- Hardening flag. ID token MUST have `email_verified: true`
    -- when this is TRUE (default). Set to FALSE only by explicit
    -- operator action for IdPs that don't emit email_verified
    -- (some self-hosted Keycloak realms).
    enforce_email_verified BOOLEAN NOT NULL DEFAULT TRUE,

    -- Audit-light timestamps. F5.2-d's CRUD endpoint maintains
    -- `updated_at`.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lookup by issuer_url is needed when validating an inbound ID
-- token: we extract `iss` from the token and locate the matching
-- org_id. The PK on org_id covers the reverse lookup.
CREATE INDEX IF NOT EXISTS idx_org_oidc_configs_issuer_url
    ON org_oidc_configs(issuer_url);

COMMIT;
