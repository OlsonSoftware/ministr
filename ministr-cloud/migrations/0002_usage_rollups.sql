-- F1.4 sub-bullet 3 — daily aggregator output.
--
-- `usage_rollups` is the per-(day, tenant, kind) sum of `usage_events`
-- rows. The aggregator (`billing::rollup::rollup_day`) runs as a
-- nightly Container Apps Job at 01:00 UTC and rolls up the prior day;
-- the `/api/v1/billing/usage` endpoint reads from this table rather
-- than scanning raw events.
--
-- Forward-only, idempotent. Primary key `(day, tenant_id, kind)` so
-- the aggregator can use INSERT ... ON CONFLICT to re-run a day's
-- rollup safely (e.g. after a cron retry, or for mid-day partial
-- rollups the billing UI surfaces as "today, so far").

BEGIN;

CREATE TABLE IF NOT EXISTS usage_rollups (
    day           DATE        NOT NULL,
    tenant_id     UUID        NOT NULL,
    kind          TEXT        NOT NULL,
    total         BIGINT      NOT NULL,
    rolled_up_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (day, tenant_id, kind)
);

-- Tenant-side reads: the billing UI's per-tenant breakdown filters
-- by tenant_id + day range.
CREATE INDEX IF NOT EXISTS idx_usage_rollups_tenant_day
    ON usage_rollups (tenant_id, day DESC);

-- Kind-side reads: the ops dashboard's "queries/day across all
-- tenants" series filters by kind + day range.
CREATE INDEX IF NOT EXISTS idx_usage_rollups_kind_day
    ON usage_rollups (kind, day DESC);

COMMIT;
