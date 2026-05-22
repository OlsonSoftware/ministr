-- F5.3-b — REVOKE for audit_events immutability.
--
-- Creates a constrained `ministr_audit_runtime` role with
-- INSERT+SELECT only on audit_events. The production cloud serve
-- connects as this role (F5.3-b-deploy, Pulumi work) so a compromised
-- runtime — including via SQL injection in a handler — cannot DELETE
-- or UPDATE rows that have already landed. The pruner cron uses a
-- separate elevated role (ministr or a future ministr_audit_admin).
--
-- Today the dev DB's `ministr` superuser still bypasses the REVOKE;
-- the e2e harness verifies the policy by `SET LOCAL ROLE
-- ministr_audit_runtime` inside a transaction and asserting DELETE
-- fails with "permission denied".
--
-- Honest finding pinned at migration-design time: PG does NOT
-- cascade GRANT on a partitioned parent to existing child
-- partitions. Verified empirically:
--
--    GRANT INSERT ON audit_events TO testrole;
--    SELECT has_table_privilege('testrole', 'audit_events_y2026q2', 'INSERT');
--    → false
--
-- So this migration walks pg_inherits and grants to each existing
-- partition individually. F5.3-c-ii-boot's ensure_audit_partitions
-- helper will need a sibling change to also grant on partitions it
-- creates at runtime; tracked as a follow-up in F5.3-b-deploy.

BEGIN;

-- NOLOGIN means the role exists for SET ROLE purposes today; the
-- production Pulumi work will ALTER ROLE … WITH LOGIN PASSWORD …
-- and rotate the credential into the cloud's connection string.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ministr_audit_runtime') THEN
        CREATE ROLE ministr_audit_runtime NOLOGIN;
    END IF;
END
$$;

-- PG 15+ removed the implicit USAGE-to-PUBLIC grant on the public
-- schema; without an explicit grant, a SET ROLE'd session sees
-- "relation audit_events does not exist" rather than "permission
-- denied" — the role can't see the schema at all. Granting USAGE
-- lets the role *reference* tables in the schema; the per-table
-- INSERT/SELECT grants below then govern what it can actually do.
GRANT USAGE ON SCHEMA public TO ministr_audit_runtime;

-- Belt-and-suspenders: REVOKE first, then GRANT. REVOKE is
-- idempotent on a fresh role; mirrors the production posture where
-- the role might pre-exist from a prior migration attempt.
REVOKE ALL ON audit_events FROM ministr_audit_runtime;
GRANT INSERT, SELECT ON audit_events TO ministr_audit_runtime;

-- The sequence backing audit_events.id needs USAGE for nextval()
-- during INSERT. Without it, INSERT fails with "permission denied
-- for sequence audit_events_id_seq" even though the table grants
-- are correct — a confusing failure mode the cloud serve would hit
-- on first audit emission.
GRANT USAGE ON SEQUENCE audit_events_id_seq TO ministr_audit_runtime;

-- Apply the same INSERT+SELECT grants to every existing partition.
-- pg_inherits enumerates child partitions; format() + EXECUTE
-- bind the table name as an identifier (safe against SQL injection
-- via the partition name).
DO $$
DECLARE
    part_name TEXT;
BEGIN
    FOR part_name IN
        SELECT c.relname
        FROM pg_inherits i
        JOIN pg_class c ON c.oid = i.inhrelid
        WHERE i.inhparent = 'audit_events'::regclass
    LOOP
        EXECUTE format('REVOKE ALL ON %I FROM ministr_audit_runtime', part_name);
        EXECUTE format('GRANT INSERT, SELECT ON %I TO ministr_audit_runtime', part_name);
    END LOOP;
END
$$;

COMMIT;
