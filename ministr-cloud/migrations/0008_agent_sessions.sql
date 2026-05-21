-- F6.1-a — durable agent-session snapshots.
--
-- One row per active session a tenant has against a corpus. The cloud
-- writes a checkpoint on session mutations (F6.1-b will wire the
-- snapshot-on-write hook) so a fresh pod can resume by hydrating
-- `budget_used` + `coherence_score` from the row, rather than treating
-- a pod restart as a session reset.
--
-- v0 carries only the load-bearing snapshot fields. Future iterations
-- may attach a JSONB column for memory / dropped-claim shadow state;
-- the table is kept narrow today so the snapshot write is a single
-- short UPDATE on a heavily-indexed row.
--
-- Forward-only; idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS agent_sessions (
    -- Session identifier as the agent presented it (free-form string).
    -- Kept as TEXT rather than UUID because SessionId in ministr-core
    -- is a wrapped String, not a UUID — agents pick their own ids.
    id                TEXT        NOT NULL,
    -- The owning tenant. Sessions are tenant-scoped: a session id is
    -- only meaningful in the context of the tenant who opened it, so
    -- the primary key is (tenant_id, id) — two tenants can use the
    -- same string id without colliding.
    tenant_id         UUID        NOT NULL,
    -- Corpus this session is bound to. TEXT to match
    -- cloud_corpora.corpus_id; nullable in case a session is opened
    -- before a default corpus is picked.
    corpus_id         TEXT,
    opened_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Cumulative tokens this session has consumed. Source of truth for
    -- F6.1-c lazy-restore: a fresh pod hydrates a UsageTracker
    -- pre-seeded to this value so prior consumption stays accounted.
    budget_used       BIGINT      NOT NULL DEFAULT 0,
    -- Cross-session coherence score. Range [0.0, 1.0]; the F1.x
    -- coherence module recomputes this per delivery. Persisting it
    -- means a resumed session can resume with the same "warm" score
    -- rather than starting cold.
    coherence_score   DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    PRIMARY KEY (tenant_id, id)
);

-- Lookup by (tenant, corpus): "list sessions this tenant has open on
-- this corpus" — useful for admin tooling and for the future cleanup
-- cron that prunes long-stale sessions.
CREATE INDEX IF NOT EXISTS idx_agent_sessions_tenant_corpus
    ON agent_sessions (tenant_id, corpus_id);

-- Lookup by last_seen_at — for the eventual stale-session prune cron
-- that runs after F6.1 lands the snapshot-on-write hook.
CREATE INDEX IF NOT EXISTS idx_agent_sessions_last_seen
    ON agent_sessions (last_seen_at);

COMMIT;
