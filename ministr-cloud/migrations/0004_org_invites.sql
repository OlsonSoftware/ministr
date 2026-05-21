-- F3.1b-i — org invites (link-generation half).
--
-- Magic-link invites for new org members. The full F3.1b chunk also
-- covered email delivery; that half is deferred to F3.1b-ii. This
-- migration unlocks the in-app "copy invite link" flow today —
-- owners/admins POST to create an invite, share the URL, and the
-- recipient lands in the org via the GitHub sign-in callback.
--
-- # Token storage
--
-- Only the SHA-256 of the raw invite token is stored. The plaintext
-- only ever appears in the response to the create call (so the owner
-- can copy the URL) and in the URL the recipient opens. Treating it
-- like a password makes a leaked DB dump useless for joining an org.
--
-- # Lifecycle
--
-- - `accepted_at IS NULL` is the active state. The acceptance path
--   (`POST /auth/github/callback` with `invite=`) atomically marks the
--   row + inserts `org_members`.
-- - `expires_at` is enforced at consume time. Default TTL is set by
--   the handler (7 days at time of writing).
-- - `org_invites` rows are NEVER DELETEd by the runtime; expired and
--   accepted rows stay around as an audit trail and as a guard
--   against a recipient replaying the same URL. A future retention
--   policy (90d, mirror of the audit-light retention) can prune.
--
-- Forward-only, idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS org_invites (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      UUID        NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    email       TEXT        NOT NULL,
    -- Hex-encoded SHA-256 of the raw token. The raw token never lands
    -- in the DB; the hex form keeps queries simple (text equality) at
    -- the cost of doubling the row size vs BYTEA — fine at invite
    -- volumes.
    token_hash  TEXT        NOT NULL UNIQUE,
    role        TEXT        NOT NULL DEFAULT 'member'
        CHECK (role IN ('owner', 'admin', 'member')),
    invited_by  UUID        NOT NULL REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at TIMESTAMPTZ,
    expires_at  TIMESTAMPTZ NOT NULL
);

-- Hot path on the consume side: look up an invite by its token hash
-- and reject if accepted or expired. The partial-index predicate
-- skips already-accepted rows so the index stays small.
CREATE INDEX IF NOT EXISTS idx_org_invites_token_hash_active
    ON org_invites (token_hash)
    WHERE accepted_at IS NULL;

-- List/admin path: enumerate an org's outstanding invites.
CREATE INDEX IF NOT EXISTS idx_org_invites_org_active
    ON org_invites (org_id, created_at DESC)
    WHERE accepted_at IS NULL;

COMMIT;
