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
    routing::{get, post},
};
use deadpool_postgres::Pool;
use ministr_mcp::auth::Tenant;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::invites::{DEFAULT_INVITE_TTL, create_invite};
use super::repo::{
    OrgError, OrgRow, OrgWithRole, create_org, list_org_members, list_orgs_for_user, member_role,
    set_org_stripe_customer_id, user_email,
};
use crate::billing::StripeClient;

/// Handler state — the cloud Postgres pool. Shared `Arc` with the rest
/// of `cmd_serve_http` so the orgs router does not own a second pool.
#[derive(Clone)]
pub struct OrgsState {
    pool: Arc<Pool>,
    /// F3.1b-i — cloud base URL for invite-link building. `None` falls
    /// back to a relative path in the response, which the caller can
    /// prefix client-side; production deployments should always set
    /// this via [`Self::with_cloud_base_url`].
    cloud_base_url: Option<String>,
    /// F3.1c-i — optional Stripe client. `Some` triggers org-Customer
    /// creation on org insert (best-effort, mirrors the user-side
    /// pattern in `auth::github_signin`). `None` leaves
    /// `orgs.stripe_customer_id` NULL — populated later by a sync
    /// job or the F3.1c-iii personal-to-org transfer.
    stripe: Option<Arc<StripeClient>>,
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

impl OrgsState {
    /// Construct from an owned pool. Convenient for tests; production
    /// callers go through [`Self::from_arc`].
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
            cloud_base_url: None,
            stripe: None,
        }
    }

    /// Construct from an already-shared `Arc<Pool>`. The serve binary
    /// builds one pool and threads it through every cloud-side state
    /// (billing, quota, sink, atlas, orgs) — keeping the constructor
    /// `Arc`-aware means the orgs surface composes cleanly.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            cloud_base_url: None,
            stripe: None,
        }
    }

    /// F3.1b-i — the absolute base URL of the cloud. Used to build the
    /// invite URL returned by `POST /api/v1/orgs/{id}/invites`. Set
    /// from `MINISTR_CLOUD_BASE_URL` in `cmd_serve_http`. `None` on
    /// self-hosted serve (where the orgs router is not mounted anyway).
    #[must_use]
    pub fn with_cloud_base_url(mut self, base: impl Into<String>) -> Self {
        let raw = base.into();
        self.cloud_base_url = Some(trim_trailing_slashes(raw));
        self
    }

    /// F3.1c-i — wire a [`StripeClient`] so the `POST /api/v1/orgs`
    /// handler mints an org-owned Stripe Customer alongside the org
    /// insert. Best-effort: a Stripe outage doesn't block org
    /// creation (mirrors the user-side pattern in
    /// `auth::github_signin::handle_github_callback`).
    #[must_use]
    pub fn with_stripe(mut self, stripe: Arc<StripeClient>) -> Self {
        self.stripe = Some(stripe);
        self
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
        .route("/api/v1/orgs/{id}/invites", post(create_invite_handler))
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

    // F3.1c-i — best-effort Stripe Customer creation. A Stripe outage
    // must not block tenant onboarding; failures log + continue and a
    // follow-up sync job (future F3.1c-iv) can fill rows that race
    // past this hook with `orgs.stripe_customer_id IS NULL`. Mirrors
    // the user-side post-sign-in pattern in
    // `auth::github_signin::handle_github_callback`.
    if let Some(stripe) = state.stripe.as_ref() {
        // Derive the billing email from the owner's `users.email`
        // rather than asking the create-org form for it — F3.1a's
        // form doesn't carry the field and the owner is the obvious
        // first invoice recipient. F3.1c-ii (or a settings UI) will
        // add an explicit override later.
        match user_email(&state.pool, &user_id).await {
            Ok(Some(email)) => {
                match stripe
                    .create_org_customer(&org.id, &org.name, &email)
                    .await
                {
                    Ok(customer_id) => {
                        if let Err(e) =
                            set_org_stripe_customer_id(&state.pool, &org.id, &customer_id).await
                        {
                            warn!(
                                error = %e,
                                org_id = %org.id,
                                "persist orgs.stripe_customer_id failed",
                            );
                        } else {
                            tracing::info!(
                                org_id = %org.id,
                                stripe_customer_id = %customer_id,
                                "stripe customer created for new org",
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            org_id = %org.id,
                            "stripe org customer creation failed — org creation proceeds, follow-up sync will retry",
                        );
                    }
                }
            }
            Ok(None) => {
                warn!(
                    user_id = %user_id,
                    org_id = %org.id,
                    "owner user row missing — skipping stripe customer creation",
                );
            }
            Err(e) => {
                warn!(
                    error = %e,
                    user_id = %user_id,
                    "lookup owner email failed — skipping stripe customer creation",
                );
            }
        }
    }

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

/// `POST /api/v1/orgs/{id}/invites` request body. `role` defaults to
/// `"member"`; the route rejects any other value than `member|admin`
/// (only owners can mint owners — implementer-side guardrail, not a
/// product feature yet).
#[derive(Debug, Deserialize)]
struct CreateInviteRequest {
    email: String,
    #[serde(default)]
    role: Option<String>,
}

/// `POST /api/v1/orgs/{id}/invites` response body. The `invite_url`
/// is the URL the caller pastes into Slack / DM / their preferred
/// channel until F3.1b-ii ships email delivery. `expires_at` is
/// ISO-8601 UTC so the UI can render a human-readable countdown.
#[derive(Debug, Serialize)]
struct CreateInviteResponse {
    invite_id: String,
    invite_url: String,
    role: String,
    expires_at: String,
}

async fn create_invite_handler(
    State(state): State<OrgsState>,
    full_tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreateInviteResponse>), OrgsApiError> {
    let user_id = tenant_user_id(full_tenant)?;

    // Authz: only owners and admins of the org may mint invites.
    // Same shape as `members_handler` — non-members hit a 403 without
    // existence-leak.
    let Some(role) = member_role(&state.pool, &org_id, &user_id)
        .await
        .map_err(OrgsApiError::Repo)?
    else {
        return Err(OrgsApiError::Forbidden);
    };
    if role != "owner" && role != "admin" {
        return Err(OrgsApiError::Forbidden);
    }

    // Pick + validate the target role. Default member; admin allowed;
    // owner explicitly forbidden via the invite path (org-creator is
    // the only owner today, and a future "transfer ownership" flow
    // belongs on its own endpoint).
    let target_role = body.role.unwrap_or_else(|| "member".to_owned());
    if target_role != "member" && target_role != "admin" {
        return Err(OrgsApiError::InvalidInviteRole);
    }

    let trimmed = body.email.trim();
    if trimmed.is_empty() || !trimmed.contains('@') {
        return Err(OrgsApiError::InvalidInviteEmail);
    }

    let created = create_invite(
        &state.pool,
        &org_id,
        trimmed,
        &target_role,
        &user_id,
        DEFAULT_INVITE_TTL,
    )
    .await
    .map_err(OrgsApiError::Repo)?;

    let invite_url = build_invite_url(state.cloud_base_url.as_deref(), &created.raw_token);

    Ok((
        StatusCode::CREATED,
        Json(CreateInviteResponse {
            invite_id: created.row.id,
            invite_url,
            role: created.row.role,
            expires_at: created.row.expires_at,
        }),
    ))
}

/// F3.1b-i — assemble the magic-link URL. When `cloud_base` is set
/// the URL is absolute; otherwise the caller gets a relative path
/// (`/auth/github/start?invite=...`) and must prepend its own origin.
fn build_invite_url(cloud_base: Option<&str>, raw_token: &str) -> String {
    let path = format!(
        "/auth/github/start?invite={}",
        url_percent_encode(raw_token)
    );
    match cloud_base {
        Some(b) => format!("{b}{path}"),
        None => path,
    }
}

fn url_percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let allowed = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if allowed {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn hex_nibble(b: u8) -> char {
    match b {
        0..=9 => (b'0' + b) as char,
        10..=15 => (b'A' + (b - 10)) as char,
        _ => '0',
    }
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
    /// F3.1b-i — `role` field on the invite payload was neither
    /// `"member"` nor `"admin"`.
    InvalidInviteRole,
    /// F3.1b-i — `email` field on the invite payload was empty or
    /// missing an `@`.
    InvalidInviteEmail,
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
            Self::InvalidInviteRole => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invite role must be 'member' or 'admin'",
            )
                .into_response(),
            Self::InvalidInviteEmail => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invite email must contain '@'",
            )
                .into_response(),
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
