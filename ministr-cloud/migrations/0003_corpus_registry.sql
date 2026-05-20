-- PHASE3 chunk 1 — pod-shared registry of which corpora exist.
--
-- Today the daemon writes the list of registered corpora to a
-- per-pod-local `corpora.json`; on ACA pod recycle the list is lost
-- and clients must re-register source paths to recover. This table
-- moves that list into Postgres so every replica and every restart
-- sees the same set of corpora.
--
-- Distinct from the F1.2 `corpora` UUID-keyed table — that one is
-- shaped for the future ACL/billing/owner story (owner_user_id /
-- owner_org_id with an exclusive CHECK constraint) and has no
-- callers yet. `cloud_corpora` is the working pod-shared registry
-- the daemon's `CorpusRegistry` reads/writes today; the two will
-- be joined or merged when multi-tenant ACL lands.
--
-- Forward-only, idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS cloud_corpora (
    corpus_id     TEXT        PRIMARY KEY,
    tenant_id     TEXT,
    paths         JSONB       NOT NULL,
    display_name  TEXT,
    status        TEXT        NOT NULL DEFAULT 'pending',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Tenant scan: every pod's `restore()` reads its tenant's rows. Once
-- multi-tenant pivots in, this index keeps the cold-start cost bounded
-- to one tenant's corpora rather than the whole platform's.
CREATE INDEX IF NOT EXISTS idx_cloud_corpora_tenant
    ON cloud_corpora (tenant_id, created_at DESC)
    WHERE tenant_id IS NOT NULL;

COMMIT;
