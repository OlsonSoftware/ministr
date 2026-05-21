-- F3.2-i — corpus access-control list for cloud_corpora.
--
-- The F1.2 initial migration shipped a UUID-keyed `corpus_acl` table
-- against the (also UUID-keyed) `corpora` table, neither of which has
-- callers today. PHASE3's `cloud_corpora` table (migration 0003) is
-- the live registry the daemon reads/writes — TEXT-keyed by corpus_id
-- (deterministic hash of paths / clone URL). F3.2-i needs an ACL
-- against the live table, so we mirror the F1.2 shape here against
-- `cloud_corpora.corpus_id` instead.
--
-- # Initial grant scope
--
-- v0 supports org-level grants only (`org_id IS NOT NULL`). User-
-- level grants (`user_id IS NOT NULL`) are reserved in the schema
-- for a follow-up — the table accepts either column today, but the
-- F3.2-i routes only mint org grants. `scope = 'read'` is the only
-- value the routes accept; the CHECK constraint admits 'write' too
-- so a follow-up that supports collaborative write can land without
-- a schema change.
--
-- # Tenant filter integration
--
-- F2.x-b's `PostgresTenantCorpusFilter::allowed` is extended in the
-- same chunk to consult this table when direct ownership
-- (cloud_corpora.tenant_id) doesn't match. The lookup is keyed on
-- `(corpus_id, org_id)` so the existing partial index is the
-- fast path.
--
-- Forward-only, idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS cloud_corpus_acl (
    corpus_id  TEXT        NOT NULL REFERENCES cloud_corpora(corpus_id) ON DELETE CASCADE,
    org_id     UUID                 REFERENCES orgs(id)  ON DELETE CASCADE,
    user_id    UUID                 REFERENCES users(id) ON DELETE CASCADE,
    scope      TEXT        NOT NULL CHECK (scope IN ('read', 'write')),
    granted_by UUID        NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((org_id IS NOT NULL) <> (user_id IS NOT NULL))
);

-- Hot path: tenant filter joins cloud_corpus_acl → org_members →
-- users to check ACL membership. Index on (corpus_id, org_id) keeps
-- the filter's added cost bounded.
CREATE UNIQUE INDEX IF NOT EXISTS idx_cloud_corpus_acl_org
    ON cloud_corpus_acl (corpus_id, org_id) WHERE org_id IS NOT NULL;

-- Reserved for user-level grants (F3.2 follow-up). Partial-index
-- pattern matches the org case so future query plans stay symmetric.
CREATE UNIQUE INDEX IF NOT EXISTS idx_cloud_corpus_acl_user
    ON cloud_corpus_acl (corpus_id, user_id) WHERE user_id IS NOT NULL;

-- Admin list path: enumerate grants on a corpus. Same shape as the
-- F3.1b invites' org_invites idx_org_active.
CREATE INDEX IF NOT EXISTS idx_cloud_corpus_acl_corpus_created
    ON cloud_corpus_acl (corpus_id, created_at DESC);

COMMIT;
