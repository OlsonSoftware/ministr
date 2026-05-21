//! F3.2-i — share corpora with orgs.
//!
//! v0 supports `org_id` grants only at the `scope = 'read'` level.
//! The table schema admits `'write'` and `user_id` grants too; the
//! routes deliberately reject anything outside the v0 surface so a
//! follow-up can widen the surface without a schema change.
//!
//! # Authz
//!
//! Mint/revoke require the caller to be the corpus owner (the tenant
//! whose `tenant_subject` matches `cloud_corpora.tenant_id`). List
//! reads the corpus's ACL when the caller is the owner OR a member
//! of an org that has been granted access. The `TenantCorpusFilter`
//! (F2.x-b) is extended in the same chunk to consult this ACL so
//! tool dispatch admits members of an org that's been granted.
//!
//! # Audit trail
//!
//! Every row carries `granted_by` (the user that minted the grant);
//! a future F3.7 audit-light feed reads from here.

use deadpool_postgres::Pool;
use serde::{Deserialize, Serialize};

use super::repo::OrgError;

/// One row from `cloud_corpus_acl`. Only one of `org_id` / `user_id`
/// is populated (DB CHECK constraint enforces). v0 only mints
/// org-side grants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclEntry {
    pub corpus_id: String,
    pub org_id: Option<String>,
    pub user_id: Option<String>,
    pub scope: String,
    pub granted_by: String,
    /// ISO-8601 UTC.
    pub created_at: String,
}

/// Mint a new ACL grant for an org. Idempotent on
/// `(corpus_id, org_id)` via the partial-unique index — re-issuing
/// the same grant returns the existing row's `created_at`.
///
/// # Errors
///
/// - [`OrgError::Sql`] on DB failure or FK violation (corpus / org /
///   user not found).
/// - [`OrgError::GetConn`] when the pool is empty.
pub async fn share_with_org(
    pool: &Pool,
    corpus_id: &str,
    org_id: &str,
    scope: &str,
    granted_by_user_id: &str,
) -> Result<AclEntry, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("share_with_org: {e}")))?;
    let row = conn
        .query_one(
            "INSERT INTO cloud_corpus_acl
               (corpus_id, org_id, scope, granted_by)
             VALUES ($1, $2::uuid, $3, $4::uuid)
             ON CONFLICT (corpus_id, org_id) WHERE org_id IS NOT NULL DO UPDATE
               SET scope = EXCLUDED.scope, granted_by = EXCLUDED.granted_by
             RETURNING
                 corpus_id,
                 org_id::text  AS org_id_text,
                 NULL::text    AS user_id_text,
                 scope,
                 granted_by::text AS granted_by_text,
                 to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso",
            &[&corpus_id, &org_id, &scope, &granted_by_user_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("share_with_org: {e}")))?;
    Ok(AclEntry {
        corpus_id: row.get("corpus_id"),
        org_id: row.get("org_id_text"),
        user_id: row.get("user_id_text"),
        scope: row.get("scope"),
        granted_by: row.get("granted_by_text"),
        created_at: row.get("created_at_iso"),
    })
}

/// Revoke an org's access to a corpus. Idempotent — revoking a row
/// that doesn't exist returns `Ok(false)`.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn revoke_org_share(
    pool: &Pool,
    corpus_id: &str,
    org_id: &str,
) -> Result<bool, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("revoke_org_share: {e}")))?;
    let affected = conn
        .execute(
            "DELETE FROM cloud_corpus_acl
             WHERE corpus_id = $1 AND org_id = $2::uuid",
            &[&corpus_id, &org_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("revoke_org_share: {e}")))?;
    Ok(affected > 0)
}

/// List every ACL grant on a corpus, ordered by `created_at DESC`.
/// Mixes org and user grants — v0 only mints org grants so the
/// user-side rows are always empty today.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn list_acl(pool: &Pool, corpus_id: &str) -> Result<Vec<AclEntry>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("list_acl: {e}")))?;
    let rows = conn
        .query(
            "SELECT
                 corpus_id,
                 org_id::text     AS org_id_text,
                 user_id::text    AS user_id_text,
                 scope,
                 granted_by::text AS granted_by_text,
                 to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                                  AS created_at_iso
             FROM cloud_corpus_acl
             WHERE corpus_id = $1
             ORDER BY created_at DESC",
            &[&corpus_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("list_acl: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| AclEntry {
            corpus_id: r.get("corpus_id"),
            org_id: r.get("org_id_text"),
            user_id: r.get("user_id_text"),
            scope: r.get("scope"),
            granted_by: r.get("granted_by_text"),
            created_at: r.get("created_at_iso"),
        })
        .collect())
}

/// Resolve `cloud_corpora.tenant_id` for a corpus. Used by the share
/// routes to authz the caller as the corpus owner. Returns `None`
/// when the corpus row doesn't exist OR when its `tenant_id` is
/// NULL (legacy / pre-F2.x-d rows — the route treats those as
/// un-shareable).
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn corpus_owner_tenant(
    pool: &Pool,
    corpus_id: &str,
) -> Result<Option<String>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("corpus_owner_tenant: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT tenant_id FROM cloud_corpora WHERE corpus_id = $1",
            &[&corpus_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("corpus_owner_tenant: {e}")))?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>("tenant_id")))
}

/// F3.2-i — extended F2.x-b filter check: does an ACL grant the
/// `tenant_subject` access to `corpus_id` via membership in an org
/// that's been granted? Two-way join: `cloud_corpus_acl` ↔
/// `org_members` (resolving the `tenant_subject` directly against
/// `org_members.user_id`, matching the convention proven by the
/// orgs handler stack — `tenant.subject == users.id::text`).
///
/// Returns `Ok(true)` when at least one org-grant gives the tenant
/// access.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn acl_grants_access(
    pool: &Pool,
    corpus_id: &str,
    tenant_subject: &str,
) -> Result<bool, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("acl_grants_access: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT 1
             FROM cloud_corpus_acl a
             JOIN org_members m ON m.org_id = a.org_id
             WHERE a.corpus_id = $1
               AND a.org_id IS NOT NULL
               AND m.user_id = $2::uuid
             LIMIT 1",
            &[&corpus_id, &tenant_subject],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("acl_grants_access: {e}")))?;
    Ok(row.is_some())
}

#[cfg(test)]
mod tests {
    //! Pure-Rust shape checks. Postgres integration is covered
    //! indirectly by F2.x-b's tenant-filter tests; the SQL paths
    //! above mirror the same idioms.

    use super::*;

    #[test]
    fn acl_entry_serialises_canonically() {
        let entry = AclEntry {
            corpus_id: "abc".into(),
            org_id: Some("org-uuid".into()),
            user_id: None,
            scope: "read".into(),
            granted_by: "user-uuid".into(),
            created_at: "2026-05-20T00:00:00Z".into(),
        };
        let s = serde_json::to_string(&entry).unwrap();
        assert!(s.contains("\"corpus_id\":\"abc\""));
        assert!(s.contains("\"org_id\":\"org-uuid\""));
        assert!(s.contains("\"user_id\":null"));
        assert!(s.contains("\"scope\":\"read\""));
    }
}
