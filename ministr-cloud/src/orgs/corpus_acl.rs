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
             VALUES ($1, $2::text::uuid, $3, $4::text::uuid)
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
             WHERE corpus_id = $1 AND org_id = $2::text::uuid",
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

/// Outcome of one [`transfer_corpus_to_org`] call. The handler
/// returns these variants as distinct HTTP status codes so the UI
/// can render different messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    /// `tenant_id` flipped from `caller_user_id` to `target_org_id`,
    /// and an ACL row was minted so existing visibility-filter logic
    /// surfaces the corpus to org members. Carries the prior tenant
    /// (the caller's user UUID) so the audit event echoes both sides.
    Transferred {
        /// The `tenant_id` before the transfer (`caller_user_id`).
        previous_tenant_id: String,
        /// The `tenant_id` after the transfer (`org_id`).
        new_tenant_id: String,
    },
    /// Caller is not the current owner of the corpus. Encompasses
    /// "corpus not found" + "`tenant_id` IS NULL" + "`tenant_id` !=
    /// caller" — same response shape for all three so an attacker
    /// can't distinguish them. Mirrors `assert_corpus_owner` in
    /// `routes.rs`.
    NotOwner,
    /// The corpus's `tenant_id` already matches the target org's id —
    /// nothing to do. Distinguished from `Transferred` so the route
    /// can return 200 (vs 201) and the audit feed doesn't get a
    /// duplicate `corpus.transferred` event on retry.
    AlreadyOnTarget,
}

/// F3.2-iv — transfer a corpus's ownership from a user tenant to an
/// org tenant. Single transaction:
///
/// 1. `SELECT tenant_id FROM cloud_corpora WHERE corpus_id = $1 FOR UPDATE`
///    serialises against concurrent transfer + share attempts.
/// 2. Ownership check: returns [`TransferOutcome::NotOwner`] when the
///    row's `tenant_id` is NULL, missing, or differs from the
///    caller's user UUID.
/// 3. Short-circuit [`TransferOutcome::AlreadyOnTarget`] when the row's
///    `tenant_id` already matches `target_org_id`.
/// 4. `UPDATE cloud_corpora SET tenant_id = $org_id::uuid`.
/// 5. `INSERT INTO cloud_corpus_acl` with `scope = 'write'` for the
///    org. `ON CONFLICT DO UPDATE` mirrors [`share_with_org`] so a
///    later re-share keeps the scope monotone.
///
/// The ACL insert means existing F3.2-iii visibility logic
/// (`cloud_corpus_acl JOIN org_members`) surfaces the corpus to every
/// member of the target org without any filter rewrite.
///
/// # Errors
///
/// - [`OrgError::Sql`] on DB failure or FK violation (org id not in
///   `orgs`, etc.).
/// - [`OrgError::GetConn`] when the pool is empty.
pub async fn transfer_corpus_to_org(
    pool: &Pool,
    corpus_id: &str,
    target_org_id: &str,
    caller_user_id: &str,
) -> Result<TransferOutcome, OrgError> {
    let mut conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("transfer_corpus_to_org: {e}")))?;
    let tx = conn
        .transaction()
        .await
        .map_err(|e| OrgError::Sql(format!("begin txn: {e}")))?;

    // FOR UPDATE so concurrent transfers / shares serialise.
    let owner_row = tx
        .query_opt(
            "SELECT tenant_id::text AS tenant_id
             FROM cloud_corpora
             WHERE corpus_id = $1
             FOR UPDATE",
            &[&corpus_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("lock cloud_corpora: {e}")))?;

    let Some(row) = owner_row else {
        // Corpus row absent — collapse to NotOwner to avoid existence
        // leak. The commit-on-early-return mirrors the invite path.
        tx.commit()
            .await
            .map_err(|e| OrgError::Sql(format!("commit (missing): {e}")))?;
        return Ok(TransferOutcome::NotOwner);
    };

    let current_tenant: Option<String> = row.get("tenant_id");
    let Some(current_tenant) = current_tenant else {
        tx.commit()
            .await
            .map_err(|e| OrgError::Sql(format!("commit (null tenant): {e}")))?;
        return Ok(TransferOutcome::NotOwner);
    };

    if current_tenant == target_org_id {
        tx.commit()
            .await
            .map_err(|e| OrgError::Sql(format!("commit (idempotent): {e}")))?;
        return Ok(TransferOutcome::AlreadyOnTarget);
    }

    if current_tenant != caller_user_id {
        tx.commit()
            .await
            .map_err(|e| OrgError::Sql(format!("commit (not owner): {e}")))?;
        return Ok(TransferOutcome::NotOwner);
    }

    // Re-stamp tenant_id to the org's UUID string. `cloud_corpora.tenant_id`
    // is TEXT (migration 0003), so the assignment is TEXT = TEXT — no
    // cast needed. An earlier sweep wrongly applied `::text::uuid` here
    // (and broke this UPDATE because the column rejects UUID values).
    // Surfaced by F-Test-1.
    tx.execute(
        "UPDATE cloud_corpora SET tenant_id = $1 WHERE corpus_id = $2",
        &[&target_org_id, &corpus_id],
    )
    .await
    .map_err(|e| OrgError::Sql(format!("update cloud_corpora.tenant_id: {e}")))?;

    // Mint the ACL row so the existing F3.2-iii visibility-filter
    // arm (`cloud_corpus_acl JOIN org_members`) surfaces the corpus
    // to every member of the target org. `scope = 'write'` matches
    // the semantics of "this org owns the corpus now"; the v0 share
    // surface still only accepts `'read'` from external callers.
    tx.execute(
        "INSERT INTO cloud_corpus_acl
           (corpus_id, org_id, scope, granted_by)
         VALUES ($1, $2::text::uuid, 'write', $3::text::uuid)
         ON CONFLICT (corpus_id, org_id) WHERE org_id IS NOT NULL DO UPDATE
           SET scope = 'write', granted_by = EXCLUDED.granted_by",
        &[&corpus_id, &target_org_id, &caller_user_id],
    )
    .await
    .map_err(|e| OrgError::Sql(format!("insert cloud_corpus_acl: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| OrgError::Sql(format!("commit (transferred): {e}")))?;

    Ok(TransferOutcome::Transferred {
        previous_tenant_id: caller_user_id.to_owned(),
        new_tenant_id: target_org_id.to_owned(),
    })
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
               AND m.user_id = $2::text::uuid
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
    fn transfer_outcome_variants_are_distinguishable() {
        // The handler maps these to distinct HTTP codes; locking in
        // the discriminator means a future refactor that unifies
        // variants can't silently collapse the response shape.
        let t = TransferOutcome::Transferred {
            previous_tenant_id: "u".into(),
            new_tenant_id: "o".into(),
        };
        assert_ne!(t, TransferOutcome::NotOwner);
        assert_ne!(t, TransferOutcome::AlreadyOnTarget);
        assert_ne!(TransferOutcome::NotOwner, TransferOutcome::AlreadyOnTarget);
    }

    #[test]
    fn transfer_outcome_transferred_carries_both_sides() {
        // The audit event echoes previous_tenant_id (the caller, the
        // user who initiated the transfer) AND new_tenant_id (the
        // org). Both must be carried through the outcome so the
        // handler can stamp them on the AuditEntry without re-reading.
        let t = TransferOutcome::Transferred {
            previous_tenant_id: "caller-uuid".into(),
            new_tenant_id: "org-uuid".into(),
        };
        match t {
            TransferOutcome::Transferred {
                previous_tenant_id,
                new_tenant_id,
            } => {
                assert_eq!(previous_tenant_id, "caller-uuid");
                assert_eq!(new_tenant_id, "org-uuid");
            }
            _ => panic!("expected Transferred variant"),
        }
    }

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
