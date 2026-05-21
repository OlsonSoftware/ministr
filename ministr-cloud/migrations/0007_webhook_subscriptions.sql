-- F3.5a — outbound webhook subscriptions.
--
-- One row per (org, receiver URL) pair. The cloud POSTs HMAC-SHA256-
-- signed JSON payloads to the URL when an event matching `event_filter`
-- fires. v0 fires on the F3.7a audit feed (wired in F3.5b); the test
-- endpoint in F3.5a lets admins validate their receiver before any
-- real events flow.
--
-- Forward-only; idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id            UUID        NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    url               TEXT        NOT NULL,
    -- 32-byte CSPRNG secret stored in plaintext. Used as the HMAC key
    -- for outbound payloads; the receiver verifies by recomputing
    -- HMAC-SHA256(secret, timestamp + "." + body). The secret is
    -- shown to the caller exactly once at create time (via the
    -- POST response) and otherwise only readable from the DB; the
    -- list endpoint never returns it.
    secret            TEXT        NOT NULL,
    -- Comma-separated audit action filter, or "*" to admit every
    -- action. v0 admits whatever F3.7 already audits (api_key.*,
    -- corpus.*, share.*, org.created, invite.created, member.added).
    -- F3.5b extends with non-audit event sources (atlas re-index
    -- complete, etc.) and may switch this column to a richer shape.
    event_filter      TEXT        NOT NULL DEFAULT '*',
    created_by        UUID        NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_delivered_at TIMESTAMPTZ
);

-- Hot-path lookup: which subscriptions does an event for this org
-- match? Partial-on-active-only would be cleaner once a soft-revoke
-- column lands; v0 uses hard delete so the full column suffices.
CREATE INDEX IF NOT EXISTS idx_webhook_subs_org
    ON webhook_subscriptions (org_id);

COMMIT;
