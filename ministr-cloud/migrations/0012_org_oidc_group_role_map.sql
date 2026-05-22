-- F5.2-f — `group_role_map JSONB` column on `org_oidc_configs`.
--
-- Maps OIDC group names (whatever the IdP issues in the `groups`
-- claim — the column the F5.2-b/c row's `groups_claim` field names)
-- to ministr org roles: "owner" | "admin" | "member".
--
-- Shape: a JSON object `{"<idp_group_name>": "<role>", ...}`.
-- Examples:
--
--   {}                                  -- no role inference; v0 path
--   {"acme-admins": "admin"}            -- one group → one role
--   {"acme-leads": "owner",
--    "acme-engineers": "member"}        -- multi-group mapping
--
-- When a user signs in via /oidc/callback (F5.2-c), the handler
-- intersects the user's `groups` claim with this mapping. If any
-- match, the user is upserted into `org_members` with the highest
-- mapped role (owner > admin > member). An EXISTING owner is never
-- downgraded — bootstrap-safe so an over-broad `member` mapping
-- can't accidentally lock the customer out of their own org.
--
-- Default `'{}'::jsonb` means existing rows behave exactly as
-- before: no role inference, no `org_members` writes.

BEGIN;

ALTER TABLE org_oidc_configs
    ADD COLUMN IF NOT EXISTS group_role_map JSONB NOT NULL DEFAULT '{}'::jsonb;

COMMIT;
