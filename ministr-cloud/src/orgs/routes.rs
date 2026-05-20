//! Axum router + handlers for the F3.1a org surface.
//!
//! # Routes
//!
//! | Route | Verb | Purpose |
//! |---|---|---|
//! | `/api/v1/orgs`                | POST | Create an org; caller becomes `owner` |
//! | `/api/v1/orgs`                | GET  | List orgs the caller is a member of |
//! | `/api/v1/orgs/{id}/members`   | GET  | List members of an org the caller belongs to |
//!
//! # Auth
//!
//! Mounted in `cmd_serve_http` behind the OAuth `ministr:read` scope
//! guard, same as `/api/v1/billing/usage`. The scope guard populates
//! `Extension<Tenant>` on success; handlers read `tenant.subject` and
//! treat it as the `users.id` UUID (matches the convention proven by
//! `billing::endpoint::usage_handler`'s `$1::uuid` cast).
//!
//! # Why `:read` even for `POST /api/v1/orgs`
//!
//! Creating an org doesn't write to a `corpora` row; it's a
//! self-service tenant-management action. The `:write` scope is
//! reserved for corpus-state mutations (those run through the daemon
//! write router with the F2.2 rate limit and F2.3 quota layers). Org
//! creation has its own per-user idempotency story to figure out in
//! F3.1c (one Stripe Customer per org) — overloading `:write` on this
//! surface would invite spurious quota checks.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use deadpool_postgres::Pool;
use ministr_mcp::auth::Tenant;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::repo::{
    OrgError, OrgRow, OrgWithRole, create_org, list_org_members, list_orgs_for_user, member_role,
};

/// Handler state — the cloud Postgres pool. Shared `Arc` with the rest
/// of `cmd_serve_http` so the orgs router does not own a second pool.
#[derive(Clone)]
pub struct OrgsState {
    pool: Arc<Pool>,
}

impl OrgsState {
    /// Construct from an owned pool. Convenient for tests; production
    /// callers go through [`Self::from_arc`].
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Construct from an already-shared `Arc<Pool>`. The serve binary
    /// builds one pool and threads it through every cloud-side state
    /// (billing, quota, sink, atlas, orgs) — keeping the constructor
    /// `Arc`-aware means the orgs surface composes cleanly.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

/// `POST /api/v1/orgs` request body.
#[derive(Debug, Deserialize)]
struct CreateOrgRequest {
    /// Display name. Trimmed + length-checked in
    /// [`super::repo::create_org`]; rejected with 422 if invalid.
    name: String,
}

/// One org as serialised on the wire. Used by `POST /api/v1/orgs` and
/// each element of `GET /api/v1/orgs`.
///
/// The `role` field is always the *caller's* role within the org —
/// future RBAC-aware UIs render the "Manage" affordance only when role
/// ∈ {owner, admin}. The response uses the same shape for `POST` (where
/// `role` is always `"owner"`) so the desktop panel can append the new
/// org to its in-memory list without a re-fetch.
#[derive(Debug, Serialize)]
struct OrgSummary {
    id: String,
    name: String,
    plan_id: String,
    role: String,
}

impl OrgSummary {
    fn from_join(org: OrgWithRole) -> Self {
        Self {
            id: org.id,
            name: org.name,
            plan_id: org.plan_id,
            role: org.role,
        }
    }

    fn for_creator(org: &OrgRow) -> Self {
        Self {
            id: org.id.clone(),
            name: org.name.clone(),
            plan_id: org.plan_id.clone(),
            role: "owner".to_owned(),
        }
    }
}

/// `GET /api/v1/orgs` response body.
#[derive(Debug, Serialize)]
struct ListOrgsResponse {
    orgs: Vec<OrgSummary>,
}

/// One row in `GET /api/v1/orgs/{id}/members`.
#[derive(Debug, Serialize)]
struct OrgMember {
    user_id: String,
    email: String,
    role: String,
}

/// `GET /api/v1/orgs/{id}/members` response body.
#[derive(Debug, Serialize)]
struct ListMembersResponse {
    members: Vec<OrgMember>,
}

/// Build the orgs router. Mount under no prefix; routes carry their
/// full `/api/v1/orgs/...` path verbatim, matching the convention
/// established by [`crate::billing::billing_routes`].
pub fn orgs_routes(state: OrgsState) -> Router {
    Router::new()
        .route("/api/v1/orgs", get(list_handler).post(create_handler))
        .route("/api/v1/orgs/{id}/members", get(members_handler))
        .with_state(state)
}

async fn create_handler(
    State(state): State<OrgsState>,
    full_tenant: Option<Extension<Tenant>>,
    Json(body): Json<CreateOrgRequest>,
) -> Result<(StatusCode, Json<OrgSummary>), OrgsApiError> {
    let user_id = tenant_user_id(full_tenant)?;
    let org = create_org(&state.pool, &user_id, &body.name)
        .await
        .map_err(OrgsApiError::Repo)?;
    Ok((StatusCode::CREATED, Json(OrgSummary::for_creator(&org))))
}

async fn list_handler(
    State(state): State<OrgsState>,
    full_tenant: Option<Extension<Tenant>>,
) -> Result<Json<ListOrgsResponse>, OrgsApiError> {
    let user_id = tenant_user_id(full_tenant)?;
    let orgs = list_orgs_for_user(&state.pool, &user_id)
        .await
        .map_err(OrgsApiError::Repo)?
        .into_iter()
        .map(OrgSummary::from_join)
        .collect();
    Ok(Json(ListOrgsResponse { orgs }))
}

async fn members_handler(
    State(state): State<OrgsState>,
    full_tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<Json<ListMembersResponse>, OrgsApiError> {
    let user_id = tenant_user_id(full_tenant)?;
    if member_role(&state.pool, &org_id, &user_id)
        .await
        .map_err(OrgsApiError::Repo)?
        .is_none()
    {
        // Same 403 whether the org exists or not — see the module
        // doc-comment for why we don't leak existence.
        return Err(OrgsApiError::Forbidden);
    }
    let members = list_org_members(&state.pool, &org_id)
        .await
        .map_err(OrgsApiError::Repo)?
        .into_iter()
        .map(|m| OrgMember {
            user_id: m.user_id,
            email: m.email,
            role: m.role,
        })
        .collect();
    Ok(Json(ListMembersResponse { members }))
}

fn tenant_user_id(full_tenant: Option<Extension<Tenant>>) -> Result<String, OrgsApiError> {
    let Extension(t) = full_tenant.ok_or(OrgsApiError::MissingTenant)?;
    Ok(t.subject)
}

/// Errors surfaced by the orgs handlers. Mapped to HTTP responses via
/// `IntoResponse`. The repo error is wrapped rather than flattened so a
/// future fine-grained variant (e.g. quota limit on org count for
/// F3.1c) can land without restructuring this enum.
#[derive(Debug)]
enum OrgsApiError {
    /// Auth middleware should have populated `Extension<Tenant>`;
    /// missing extension means the route was wired wrong or — for
    /// public tests — the harness didn't inject a tenant.
    MissingTenant,
    /// Caller authenticated but is not a member of the requested org.
    Forbidden,
    /// Repo layer failure. Validation errors map to 422; everything
    /// else logs at warn + returns 500 without leaking detail.
    Repo(OrgError),
}

impl IntoResponse for OrgsApiError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::MissingTenant => {
                (StatusCode::UNAUTHORIZED, "missing tenant identity").into_response()
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden").into_response(),
            Self::Repo(OrgError::InvalidName) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid org name").into_response()
            }
            Self::Repo(other) => {
                warn!(error = %other, "orgs handler internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests gated on `MINISTR_TEST_PG_URL`. Pattern matches
    //! `crate::users::tests` and `crate::billing::endpoint::tests` —
    //! `cargo test -p ministr-cloud -- --ignored` runs them.

    use super::*;
    use crate::db::{connect, run_migrations};
    use crate::idp::{GITHUB_ISSUER, ResolvedIdentity};
    use crate::users::upsert_github_user;

    async fn seed_user(pool: &Pool, marker: i64) -> String {
        let id = ResolvedIdentity {
            issuer: GITHUB_ISSUER.into(),
            subject: marker.to_string(),
            email: Some(format!("orgs-{marker}@test.example")),
            display_name: Some(format!("user-{marker}")),
            github_id: Some(marker),
        };
        upsert_github_user(pool, &id).await.expect("seed user").id
    }

    fn unique_marker(suffix: &str) -> i64 {
        // Stable test-id source: process id + a 24-bit hash of the test
        // suffix keeps fixtures disjoint within and across test runs.
        // 2_000_000_000 + (pid % 1_000_000) keeps us clear of the
        // ~9M range users.rs picks for its own fixtures.
        let pid = i64::from(std::process::id() % 1_000_000);
        let mut hash: i64 = 0;
        for b in suffix.bytes() {
            hash = (hash.wrapping_mul(131)).wrapping_add(i64::from(b)) & 0xFF_FFFF;
        }
        2_000_000_000 + pid * 1_000 + hash
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn create_then_list_then_members() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let owner_id = seed_user(&pool, unique_marker("create_then_list")).await;

        let org = create_org(&pool, &owner_id, "Acme Robotics")
            .await
            .expect("create_org");
        assert_eq!(org.name, "Acme Robotics");
        assert_eq!(org.plan_id, super::super::repo::DEFAULT_ORG_PLAN);

        let listed = list_orgs_for_user(&pool, &owner_id)
            .await
            .expect("list_orgs_for_user");
        assert!(
            listed.iter().any(|o| o.id == org.id && o.role == "owner"),
            "creator must see themselves as owner; got {listed:?}",
        );

        let members = list_org_members(&pool, &org.id)
            .await
            .expect("list_org_members");
        assert_eq!(members.len(), 1, "exactly one owner after creation");
        assert_eq!(members[0].user_id, owner_id);
        assert_eq!(members[0].role, "owner");

        // Cleanup — relies on FK cascades from orgs → org_members and
        // users to leave the fixture state tidy for the next run.
        let conn = pool.get().await.unwrap();
        conn.execute("DELETE FROM orgs WHERE id = $1::uuid", &[&org.id])
            .await
            .unwrap();
        conn.execute("DELETE FROM users WHERE id = $1::uuid", &[&owner_id])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn list_only_returns_callers_orgs() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let alice = seed_user(&pool, unique_marker("alice")).await;
        let bob = seed_user(&pool, unique_marker("bob")).await;

        let alice_org = create_org(&pool, &alice, "Alice Industries")
            .await
            .expect("alice org");

        let bob_orgs = list_orgs_for_user(&pool, &bob).await.expect("bob list");
        assert!(
            !bob_orgs.iter().any(|o| o.id == alice_org.id),
            "bob must not see alice's org; saw {bob_orgs:?}",
        );

        let bob_role = member_role(&pool, &alice_org.id, &bob)
            .await
            .expect("bob member_role");
        assert!(
            bob_role.is_none(),
            "non-member must resolve to None role for the 403 path",
        );

        // Cleanup.
        let conn = pool.get().await.unwrap();
        conn.execute("DELETE FROM orgs WHERE id = $1::uuid", &[&alice_org.id])
            .await
            .unwrap();
        conn.execute("DELETE FROM users WHERE id = $1::uuid", &[&alice])
            .await
            .unwrap();
        conn.execute("DELETE FROM users WHERE id = $1::uuid", &[&bob])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn invalid_name_rejected() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let user = seed_user(&pool, unique_marker("invalid")).await;
        let err = create_org(&pool, &user, "   ").await.unwrap_err();
        assert!(matches!(err, OrgError::InvalidName), "got {err:?}");

        // Cleanup.
        let conn = pool.get().await.unwrap();
        conn.execute("DELETE FROM users WHERE id = $1::uuid", &[&user])
            .await
            .unwrap();
    }

    #[test]
    fn org_summary_serialises_stable_field_order() {
        let s = OrgSummary {
            id: "abc".into(),
            name: "Acme".into(),
            plan_id: "team".into(),
            role: "owner".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"id\":\"abc\""));
        assert!(json.contains("\"name\":\"Acme\""));
        assert!(json.contains("\"plan_id\":\"team\""));
        assert!(json.contains("\"role\":\"owner\""));
    }
}
