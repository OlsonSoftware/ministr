-- F5.4-e-audit-db — DB-backed mirror of the F5.4-e-audit JSONL
-- mint log. Adds multi-operator visibility: when several operators
-- issue licenses from different hosts, the JSONL files on each
-- host's disk can't see each other, but the shared cloud Postgres
-- they all point at can. Operators wire `mint-license --pg-url
-- URL` (or `MINISTR_PG_URL` env var) to dual-write JSONL + DB;
-- `list-licenses --pg-url URL` reads the unified view.
--
-- Field shape mirrors the JSONL exactly so the two backends are
-- interchangeable consumers of the same data model.
--
-- UNIQUE on jwt_id_hash is load-bearing: it makes the persist
-- function idempotent under retries via `ON CONFLICT DO NOTHING`.
-- An operator re-running `mint-license` (e.g. after a transient
-- backend blip caused the JSONL write to succeed but the PG write
-- to fail) gets a no-op INSERT, not a duplicate row.
--
-- Storage cost: ~120 bytes per issuance × even 10K customers/year
-- = ~1.2 MB/year. Effectively free against the existing PG flex
-- footprint.

CREATE TABLE IF NOT EXISTS license_issuances (
    id            BIGSERIAL  PRIMARY KEY,
    ts_unix       BIGINT     NOT NULL,
    ts_iso        TEXT       NOT NULL,
    enterprise_id TEXT       NOT NULL,
    seat_count    INTEGER    NOT NULL CHECK (seat_count >= 0),
    valid_days    INTEGER    NOT NULL CHECK (valid_days > 0),
    exp           BIGINT     NOT NULL,
    jwt_id_hash   CHAR(16)   NOT NULL UNIQUE
);

-- DESC because every query is "what's recent" — table view scrolls
-- by ts_unix descending the same way the JSONL list view does.
CREATE INDEX IF NOT EXISTS idx_license_issuances_ts
    ON license_issuances (ts_unix DESC);

-- enterprise_id index for "did I already issue acme-corp this
-- quarter?" lookups via list-licenses filtered by enterprise.
CREATE INDEX IF NOT EXISTS idx_license_issuances_enterprise
    ON license_issuances (enterprise_id, ts_unix DESC);
