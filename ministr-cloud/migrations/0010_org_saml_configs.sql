-- F5.1-a — per-org SAML 2.0 SP configuration.
--
-- ministr acts as the Service Provider; each customer org points us
-- at their own IdP (Okta, Entra ID, OneLogin, Google Workspace, ...).
-- One row per org that has SAML SSO enabled.
--
-- Pinning model: we trust assertions ONLY when signed by the IdP
-- whose public x509 cert matches `idp_x509_cert` AND whose Issuer
-- matches `idp_entity_id`. There is no fallback to a "global" trust
-- anchor — every assertion must validate against the per-org pinned
-- material. Rotating the IdP's signing cert is a CRUD-update on this
-- row.
--
-- SP-side metadata (`sp_entity_id`, `sp_acs_url`) is what we publish
-- in `GET /orgs/{slug}/saml/metadata.xml` for the IdP admin to
-- configure on their end. We don't sign AuthnRequests by default
-- (most IdPs don't require it); the columns to support that land in
-- F5.1-b only if we discover a customer IdP that demands it.
--
-- Attribute mapping: SAML assertions carry user attributes as named
-- `<Attribute>` elements; the IETF doesn't fix a canonical email
-- name, so each IdP uses a different URN. The default below is the
-- WS-Federation URN that Okta + Entra ID both default to; any org
-- can override per their IdP's actual emission shape.
--
-- Forward-only; idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS org_saml_configs (
    -- One SAML config per org. `org_id` is both PK and FK; an org
    -- either has SAML SSO or doesn't.
    org_id UUID PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,

    -- IdP side (the customer's Okta/Entra/OneLogin/etc.). These
    -- three columns together are the trust anchor for assertion
    -- verification.
    idp_entity_id TEXT NOT NULL,
    idp_sso_url TEXT NOT NULL,
    idp_x509_cert TEXT NOT NULL,
    -- Optional Single-Logout endpoint; if set, our /saml/slo handler
    -- can forward LogoutRequests to it.
    idp_slo_url TEXT,

    -- SP side (us). `sp_entity_id` is the URI the IdP knows us by;
    -- by convention `https://mcp.ministr.ai/orgs/{slug}/saml`. The
    -- ACS URL is where the IdP POSTs assertions; we mount the route
    -- at the same per-org path. Stored explicitly rather than
    -- derived so a customer with a non-default hostname (e.g.,
    -- on-prem mirror at sso.acme.com) can override.
    sp_entity_id TEXT NOT NULL,
    sp_acs_url TEXT NOT NULL,

    -- Attribute name URNs to pull from the assertion. Email is
    -- required (we match SAML subjects to ministr users by email).
    -- Display-name is optional cosmetic UX.
    attribute_email TEXT NOT NULL DEFAULT 'http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress',
    attribute_display_name TEXT,

    -- Hardening flags. Default ON; can be relaxed per-org only by
    -- explicit operator action. We never accept unsigned assertions
    -- in default config — this column exists so a future migration
    -- can audit any org that's set it to FALSE.
    enforce_signed_assertions BOOLEAN NOT NULL DEFAULT TRUE,

    -- Audit-light. F5.1-d's CRUD endpoints maintain `updated_at`.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Lookup by IdP entity-id is needed when validating an inbound
-- assertion: we extract Issuer from the assertion XML and locate
-- the matching org. The PK on org_id covers the reverse lookup.
CREATE INDEX IF NOT EXISTS idx_org_saml_configs_idp_entity_id
    ON org_saml_configs(idp_entity_id);

COMMIT;
