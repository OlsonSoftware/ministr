-- F5.5-b-persist-write — periodic snapshot of the in-process
-- LatencyTracker (ministr-mcp/src/admin/latency.rs). Each row is one
-- pod's percentiles at one moment in time; the future
-- F5.5-b-persist-read /sla extension aggregates across rows for
-- "what was p95 last week" historical evidence.
--
-- Single-table shape; no FKs (latency snapshots aren't tied to a
-- tenant or org — they're a fleet-wide signal). Per-pod rows are
-- distinguishable by ts_unix granularity only today; the eventual
-- pod_id column lands when F5.5-c-dedicated-aca makes per-Enterprise
-- ACAs a thing.
--
-- Storage cost: ~30 days × 1440 min × 1 row/min/pod × ~50 bytes =
-- ~2 MB/pod/month. Negligible against the existing Postgres flex
-- footprint.

CREATE TABLE IF NOT EXISTS request_latency_snapshots (
    id           BIGSERIAL PRIMARY KEY,
    ts_unix      BIGINT  NOT NULL,
    sample_count INTEGER NOT NULL CHECK (sample_count >= 0),
    p50_us       INTEGER NOT NULL CHECK (p50_us >= 0),
    p95_us       INTEGER NOT NULL CHECK (p95_us >= 0),
    p99_us       INTEGER NOT NULL CHECK (p99_us >= 0)
);

-- DESC because every query is "what's recent" — pull the head, never
-- the tail. The future cleanup chunk (F5.5-b-persist-retention)
-- will DELETE WHERE ts_unix < NOW() - 30d via the same index.
CREATE INDEX IF NOT EXISTS idx_request_latency_snapshots_ts
    ON request_latency_snapshots (ts_unix DESC);
