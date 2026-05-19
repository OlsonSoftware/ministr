-- F1.2 initial schema — multi-tenant data model.
--
-- Forward-only, idempotent. Every CREATE uses IF NOT EXISTS so re-running
-- the migration on a partially-applied database is safe; the migration
-- runner in `ministr-cloud/src/db.rs` records the version in
-- `schema_migrations` so a fully-applied database short-circuits.
--
-- Shape: shared tables with a tenant_id column on metering/audit rows
-- (per the 2026 Crunchy/PlanetScale guidance). UUID v4 keys via the
-- built-in `gen_random_uuid()` (Postgres 13+); switch to v7 once
-- ecosystem support stabilises post-PG18.
--
-- The schema mirrors §4 F1.2 of ROADMAP.md verbatim:
--   users, orgs, org_members, corpora, corpus_acl, api_keys,
--   usage_events, audit_events.

BEGIN;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version    BIGINT      PRIMARY KEY,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS users (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    email              TEXT        NOT NULL UNIQUE,
    github_id          BIGINT      UNIQUE,
    plan_id            TEXT        NOT NULL,
    stripe_customer_id TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS orgs (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name               TEXT        NOT NULL,
    plan_id            TEXT        NOT NULL,
    stripe_customer_id TEXT,
    billing_email      TEXT,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS org_members (
    org_id     UUID        NOT NULL REFERENCES orgs(id)  ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       TEXT        NOT NULL CHECK (role IN ('owner','admin','member')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_org_members_user ON org_members (user_id);

CREATE TABLE IF NOT EXISTS corpora (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id    UUID                 REFERENCES users(id) ON DELETE CASCADE,
    owner_org_id     UUID                 REFERENCES orgs(id)  ON DELETE CASCADE,
    name             TEXT        NOT NULL,
    source_url       TEXT,
    status           TEXT        NOT NULL DEFAULT 'pending',
    last_indexed_at  TIMESTAMPTZ,
    byte_size        BIGINT      NOT NULL DEFAULT 0,
    vec_count        BIGINT      NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((owner_user_id IS NOT NULL) <> (owner_org_id IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_corpora_owner_user ON corpora (owner_user_id) WHERE owner_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_corpora_owner_org  ON corpora (owner_org_id)  WHERE owner_org_id  IS NOT NULL;

CREATE TABLE IF NOT EXISTS corpus_acl (
    corpus_id  UUID        NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    user_id    UUID                 REFERENCES users(id)   ON DELETE CASCADE,
    org_id     UUID                 REFERENCES orgs(id)    ON DELETE CASCADE,
    scope      TEXT        NOT NULL CHECK (scope IN ('read','write')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((user_id IS NOT NULL) <> (org_id IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_corpus_acl_user
    ON corpus_acl (corpus_id, user_id) WHERE user_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_corpus_acl_org
    ON corpus_acl (corpus_id, org_id)  WHERE org_id  IS NOT NULL;

CREATE TABLE IF NOT EXISTS api_keys (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id UUID                 REFERENCES users(id) ON DELETE CASCADE,
    owner_org_id  UUID                 REFERENCES orgs(id)  ON DELETE CASCADE,
    name          TEXT        NOT NULL,
    hash          TEXT        NOT NULL UNIQUE,
    scopes        TEXT        NOT NULL,
    last_used_at  TIMESTAMPTZ,
    expires_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((owner_user_id IS NOT NULL) <> (owner_org_id IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS idx_api_keys_owner_user ON api_keys (owner_user_id) WHERE owner_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_api_keys_owner_org  ON api_keys (owner_org_id)  WHERE owner_org_id  IS NOT NULL;

-- Polymorphic tenant_id — references users.id OR orgs.id. Not declared
-- as a FK because Postgres has no native polymorphic FK; the resolver in
-- ministr-mcp/src/auth/middleware.rs guarantees the value points at a
-- live row at write time. Append-heavy; BIGSERIAL keeps inserts cheap.
CREATE TABLE IF NOT EXISTS usage_events (
    id        BIGSERIAL   PRIMARY KEY,
    tenant_id UUID        NOT NULL,
    kind      TEXT        NOT NULL,
    count     BIGINT      NOT NULL DEFAULT 1,
    ts        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_usage_events_tenant_ts ON usage_events (tenant_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_kind_ts   ON usage_events (kind, ts DESC);

CREATE TABLE IF NOT EXISTS audit_events (
    id       BIGSERIAL   PRIMARY KEY,
    org_id   UUID,
    actor    UUID,
    action   TEXT        NOT NULL,
    resource TEXT        NOT NULL,
    ts       TIMESTAMPTZ NOT NULL DEFAULT now(),
    ip       INET,
    ua       TEXT
);
CREATE INDEX IF NOT EXISTS idx_audit_events_org_ts ON audit_events (org_id, ts DESC) WHERE org_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_audit_events_ts     ON audit_events (ts DESC);

COMMIT;
