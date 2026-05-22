-- F5.3-d-ii — per-org SIEM exporter config table.
--
-- One row per org. `kind` discriminates the SIEM provider; F5.3-d-i
-- ships only `"splunk_hec"`. F5.3-d-iii will add Datadog Logs /
-- S3 JSON-lines / syslog-CEF and tighten this column with a CHECK
-- constraint at that time. Rust-side validators in the CRUD path
-- reject unknown kinds today.
--
-- `token` is bearer material — every HTTP read of this table from
-- the CRUD endpoint returns the `[REDACTED]` sentinel string (the
-- same one F5.2-d uses for OIDC client_secret). The DB still has
-- the real value so the dispatch path can sign HEC POSTs.
--
-- `endpoint_url` is the full URL the dispatcher hits. For Splunk HEC
-- this is typically `https://splunk.example.com:8088/services/collector/event`.
-- The CRUD validator requires `http://` or `https://` prefix; F5.3-d-iii
-- can tighten further per-provider when other kinds land.

BEGIN;

CREATE TABLE IF NOT EXISTS org_siem_configs (
    org_id        UUID         PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    kind          TEXT         NOT NULL,
    endpoint_url  TEXT         NOT NULL,
    token         TEXT         NOT NULL,
    enabled       BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ  NOT NULL DEFAULT now()
);

-- F5.3-d-ii-dispatch will query this index when the audit pipeline
-- looks up an org's config on every emission. Building it now
-- means the dispatch chunk is a code-only change.
CREATE INDEX IF NOT EXISTS idx_org_siem_configs_enabled
    ON org_siem_configs (org_id)
    WHERE enabled = TRUE;

COMMIT;
