-- F6.1-d — append-only ledger of agent-session evictions.
--
-- One row per claim-id evicted from an agent's context window. The
-- cloud writes a row whenever the session's drop path fires (caller-
-- site wiring lands in a follow-up; this table + the trait + impl
-- ship today so the seam is open). On pod restart, `try_restore` can
-- list the persisted drops to hydrate the resumed session's
-- evicted-content awareness.
--
-- Append-only by convention. No DELETEs from runtime; a future
-- retention cron may prune cold rows once the table grows.
--
-- Forward-only; idempotent.

BEGIN;

CREATE TABLE IF NOT EXISTS session_drops (
    -- Monotonic id used as the ORDER BY key for chronological replay.
    -- BIGSERIAL is wider than necessary today but matches the
    -- audit_events convention and gives ~2^63 events of headroom.
    id           BIGSERIAL   PRIMARY KEY,
    -- Session identifier as the agent presented it.
    session_id   TEXT        NOT NULL,
    -- Owning tenant. Composite hot-path lookup is (tenant_id,
    -- session_id) — see idx_session_drops_tenant_session below.
    tenant_id    UUID        NOT NULL,
    -- Claim / content id that was evicted.
    claim_id     TEXT        NOT NULL,
    -- Wall-clock at the eviction. The ledger writer captures this
    -- client-side because the eviction itself happened in-process —
    -- DEFAULT now() would record the persist time, not the evict time.
    -- We accept the small clock skew because the row's value is "this
    -- claim was evicted on this session, in this order".
    evicted_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Hot-path lookup: every restore consults
-- (tenant_id, session_id) → list of drops. Covering both columns
-- means the planner can satisfy the WHERE clause from the index.
CREATE INDEX IF NOT EXISTS idx_session_drops_tenant_session
    ON session_drops (tenant_id, session_id, id);

-- evicted_at lookup is reserved for the future retention cron
-- (prune drops older than N days). Kept as a separate index so
-- the hot path stays narrow.
CREATE INDEX IF NOT EXISTS idx_session_drops_evicted_at
    ON session_drops (evicted_at);

COMMIT;
