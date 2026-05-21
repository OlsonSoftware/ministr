-- F3.4a — service-account API keys.
--
-- The api_keys table itself shipped in 0001 (F1.2). This migration adds
-- two columns the resolver path needs:
--   * prefix      — last 8 chars of the raw secret, surfaced in list-keys
--                   UI so a user can pick the key to revoke without
--                   the cloud ever holding the full token. The full
--                   secret is shown to the caller exactly once at
--                   create time and otherwise only its SHA-256 hash
--                   lives in `api_keys.hash`.
--   * revoked_at  — soft-revoke timestamp. `NULL` means active;
--                   non-NULL hides the row from the resolver (the same
--                   index used on the hot path also excludes revoked
--                   rows). Revoked keys are retained so F3.7 audit can
--                   surface "this key was revoked at T" history.
--
-- Forward-only; the column additions are idempotent via IF NOT EXISTS.

BEGIN;

ALTER TABLE api_keys
    ADD COLUMN IF NOT EXISTS prefix     TEXT,
    ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;

-- Hot-path resolver lookup: hash + active. The covering index lets the
-- resolver answer "is this key valid?" in a single index probe.
CREATE INDEX IF NOT EXISTS idx_api_keys_active_hash
    ON api_keys (hash)
    WHERE revoked_at IS NULL;

COMMIT;
