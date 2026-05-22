-- F5.3-c-i — convert `audit_events` to `PARTITION BY RANGE (ts)`.
--
-- One partition per calendar quarter. The composite PK `(id, ts)` is
-- required because Postgres partition keys must be part of any PK on
-- the parent table; `id` alone wouldn't admit `PARTITION BY ts`.
--
-- Why partition: F5.3 sets up an immutable + archived audit story.
-- Per-quarter partitioning gives F5.3-b a path to revoke runtime
-- DELETE permission (the pruner switches to `DROP PARTITION` which
-- only needs DDL ownership) and gives F5.3-c-ii a unit of work for
-- cold-blob archive (move one quarter's partition to Blob Object
-- Replication and detach).
--
-- Migration strategy: create-new + copy + atomic-swap. Postgres
-- can't `ALTER TABLE … PARTITION BY` on an existing non-partitioned
-- table; the only path is to build a fresh partitioned table, copy
-- rows in, and rename. The sequence (`audit_events_id_seq`) is
-- detached from the old table BEFORE the drop so it survives the
-- swap and stays at the same `last_value` (subsequent `nextval`
-- calls return id values past the previously-seen max).

BEGIN;

-- ── 1. Detach the existing sequence so DROP TABLE leaves it alone.
ALTER SEQUENCE audit_events_id_seq OWNED BY NONE;

-- ── 2. Create the new partitioned parent. Schema mirrors the
-- original 0001 audit_events shape PLUS the composite PK.
CREATE TABLE audit_events_new (
    id        BIGINT       NOT NULL DEFAULT nextval('audit_events_id_seq'),
    org_id    UUID,
    actor     UUID,
    action    TEXT         NOT NULL,
    resource  TEXT         NOT NULL,
    ts        TIMESTAMPTZ  NOT NULL DEFAULT now(),
    ip        INET,
    ua        TEXT,
    PRIMARY KEY (id, ts)
) PARTITION BY RANGE (ts);

-- ── 3. Quarterly partitions covering 2024-Q1 through 2027-Q4.
-- 16 partitions. The lower bound is far enough back that any data
-- generated before F3.7a launched (mid-2026) falls in 2026 partitions;
-- 2024-2025 partitions stay empty but pre-seeded so an out-of-order
-- backdated INSERT (e.g. an SIEM-side audit replay) lands cleanly.
-- F5.3-c-ii will add a boot-time `ensure_audit_partitions(lookahead)`
-- helper that extends the forward edge as time advances.
DO $$
DECLARE
    qstart timestamptz;
    qend   timestamptz;
    pname  text;
BEGIN
    FOR qstart IN
        SELECT generate_series(
            '2024-01-01 00:00:00+00'::timestamptz,
            '2027-10-01 00:00:00+00'::timestamptz,
            '3 months'::interval
        )
    LOOP
        qend  := qstart + interval '3 months';
        pname := format(
            'audit_events_y%sq%s',
            to_char(qstart, 'YYYY'),
            ceil(extract(month FROM qstart) / 3.0)::int
        );
        EXECUTE format(
            'CREATE TABLE %I PARTITION OF audit_events_new FOR VALUES FROM (%L) TO (%L)',
            pname, qstart, qend
        );
    END LOOP;
END
$$;

-- ── 4. Indexes mirror the originals from 0001. PG 11+ propagates
-- indexes created on the parent down to every existing + future
-- partition. Suffixed with `_new` because index names are
-- schema-scoped and would collide with 0001's `idx_audit_events_*`
-- while both tables exist; renamed back to canonical names in
-- step 7 after the old table drops.
CREATE INDEX idx_audit_events_org_ts_new
    ON audit_events_new (org_id, ts DESC)
    WHERE org_id IS NOT NULL;
CREATE INDEX idx_audit_events_ts_new
    ON audit_events_new (ts DESC);

-- ── 5. Copy every existing row. Listing columns explicitly so the
-- sequence on `id` isn't bumped (we pass the original id rather
-- than letting DEFAULT fire). After the copy the sequence's
-- last_value matches what it was before the migration; the next
-- nextval still produces a unique id past max(audit_events.id).
INSERT INTO audit_events_new (id, org_id, actor, action, resource, ts, ip, ua)
SELECT id, org_id, actor, action, resource, ts, ip, ua
FROM   audit_events;

-- ── 6. Drop the old non-partitioned table. The sequence survives
-- because we detached its OWNED BY in step 1.
DROP TABLE audit_events;

-- ── 7. Promote the new table to the canonical name + rename the
-- step-4 `_new`-suffixed indexes back to their canonical names.
-- The old indexes were dropped along with the old table in step 6
-- so the names are free.
ALTER TABLE audit_events_new RENAME TO audit_events;
ALTER INDEX idx_audit_events_org_ts_new RENAME TO idx_audit_events_org_ts;
ALTER INDEX idx_audit_events_ts_new      RENAME TO idx_audit_events_ts;

-- ── 8. Re-bind the sequence to the new column so a future
-- `DROP TABLE audit_events` would cascade-drop it again (mirrors
-- the 0001 ownership contract). Without this step the sequence
-- would survive a future drop and leak across migrations.
ALTER SEQUENCE audit_events_id_seq OWNED BY audit_events.id;

COMMIT;
