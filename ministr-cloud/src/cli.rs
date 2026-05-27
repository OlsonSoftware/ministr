//! F31.2b — `ClassicCloudMounter` implements the
//! [`ministr_api::CloudRouterMounter`] MIT seam.
//!
//! Owns the cloud-mode side of `cmd_serve_http`: validating the
//! Enterprise license, opening the Postgres pool, running migrations
//! and audit-partition seeding, building the blob backend, mounting
//! every cloud axum router, and wiring `Arc<dyn AdapterTrait>` cloud
//! sinks into the returned `CloudMountOutput` for the MIT serve to
//! splice into its daemon / OAuth / server state.
//!
//! Constructed by the `ministr-cloud-tools` proprietary binary and
//! passed to `ministr_cli::commands::cmd_serve_http` as
//! `Some(&mounter)`. The public `ministr` binary passes `None` and
//! never depends on this crate at compile time.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use ministr_api::{
    ApiError, CloudAdminAdapters, CloudDaemonAdapters, CloudMountInput, CloudMountOutput,
    CloudOAuthAdapters, CloudRouterMounter, CloudServerAdapters, RevocationHandle,
};

use crate::revocation_fetch::RevocationShutdownHandle;

/// The classic (today-default) cloud overlay used by the
/// `ministr-cloud-tools serve` subcommand.
///
/// Encapsulates the entire boot-time cloud-mode wiring previously
/// inlined in `cmd_serve_http`. See [`mount_cloud_routes`] for the
/// step-by-step body and [`CloudRouterMounter`] for the trait contract.
#[derive(Debug, Default)]
pub struct ClassicCloudMounter {
    _private: (),
}

impl ClassicCloudMounter {
    /// Build a fresh mounter. The mounter owns no state up front; every
    /// cloud resource is opened lazily inside [`setup`].
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl CloudRouterMounter for ClassicCloudMounter {
    fn setup<'a>(
        &'a self,
        input: &'a CloudMountInput,
    ) -> Pin<Box<dyn Future<Output = Result<CloudMountOutput, ApiError>> + Send + 'a>> {
        Box::pin(mount_cloud_routes(input))
    }
}

/// Implements the classic cloud overlay. Mirrors the cloud branch
/// previously inlined in `ministr_cli::commands::cmd_serve_http`.
///
/// # Errors
///
/// Returns an [`ApiError`] when license validation refuses boot, the
/// Postgres pool fails to open, migrations fail to apply, or any other
/// cloud resource refuses to come up.
pub async fn mount_cloud_routes(
    _input: &CloudMountInput,
) -> Result<CloudMountOutput, ApiError> {
    // F31.2b-ii — `MINISTR_PG_URL` gates the entire cloud overlay.
    // Unset (self-hosted / dev / community) ⇒ no cloud routes mount.
    // Chunks C+ progressively populate this function with the routers
    // migrated out of `cmd_serve_http`'s inline branch.
    let Some(pg_url) = trimmed_env("MINISTR_PG_URL") else {
        tracing::info!(
            "ClassicCloudMounter: MINISTR_PG_URL unset — returning empty CloudMountOutput"
        );
        return Ok(CloudMountOutput::default());
    };

    // Self-contained Postgres pool owned by the cloud impl. Today the
    // MIT serve's inline cloud branch ALSO opens its own pool for the
    // adapters/routes still wired there — F31.2b-ii's progressive
    // migration sunsets that branch over chunks C+. Two pools is
    // temporary; both share the same Postgres tables.
    let pool = Arc::new(
        crate::connect(&pg_url).map_err(|e| ApiError {
            code: "cloud_pg_pool_open_failed".into(),
            message: format!("open cloud postgres pool: {e}"),
        })?,
    );

    // Self-contained OAuth store for scope-wrapping the cloud routes.
    // Same Postgres tables as the MIT serve's OAuth store — both
    // instances stay in sync via shared DB state. Issuer URL doesn't
    // matter for scope validation; only the metadata endpoints (which
    // ministr-mcp's MIT serve mounts) use it. Default config is fine.
    let oauth_store = ministr_mcp::auth::OAuthStore::postgres(
        ministr_mcp::auth::OAuthConfig::default(),
        &pg_url,
    )
    .await
    .map_err(|e| ApiError {
        code: "cloud_oauth_store_open_failed".into(),
        message: format!("open cloud OAuth store: {e}"),
    })?;

    let mut router = Router::new();
    let mut daemon_adapters = CloudDaemonAdapters::default();
    let mut server_adapters = CloudServerAdapters::default();

    // F2.x-b + F3.2-iii — single PostgresTenantCorpusFilter instance
    // wired into BOTH the MCP server (gates /mcp tool calls) and the
    // daemon list endpoint (filters GET /api/v1/corpora by tenant
    // + ACL). One Arc shared via two trait casts so the SQL pool +
    // visibility semantics are identical across surfaces.
    {
        let concrete = std::sync::Arc::new(
            crate::PostgresTenantCorpusFilter::new(Arc::clone(&pool)),
        );
        let filter: std::sync::Arc<dyn ministr_api::TenantCorpusFilter> =
            std::sync::Arc::clone(&concrete) as _;
        let visibility: std::sync::Arc<dyn ministr_api::TenantCorpusVisibility> =
            std::sync::Arc::clone(&concrete) as _;
        server_adapters.tenant_filter = Some(filter);
        daemon_adapters.corpus_visibility = Some(visibility);
        tracing::info!(
            "PostgresTenantCorpusFilter wired via CloudRouterMounter — MCP gate + daemon visibility"
        );
    }

    // F1.4 — PostgresUsageSink for billable usage events on daemon
    // mutations.
    {
        let sink: std::sync::Arc<dyn ministr_api::UsageSink> = std::sync::Arc::new(
            crate::PostgresUsageSink::from_arc(Arc::clone(&pool)),
        );
        daemon_adapters.usage_sink = Some(sink);
    }

    // F3.7b — PostgresAuditSink (single sink, not the chain) for
    // daemon-side corpus-mutation events (corpus.created/cloned/deleted).
    // The full chain (Postgres → WebhookFanout → Splunk → PerOrgSiem)
    // used by orgs/api_keys/webhooks routes is built later as those
    // routes migrate.
    {
        let sink: std::sync::Arc<dyn ministr_api::AuditSink> = std::sync::Arc::new(
            crate::PostgresAuditSink::from_arc(Arc::clone(&pool)),
        );
        daemon_adapters.audit_sink = Some(sink);
    }

    // PHASE3 chunk 4 — PostgresIndexJobSink routes POST /api/v1/corpora
    // and clone routes through the cloud index-job queue instead of
    // running ingestion inline. The serve pod's in-process WorkerLoop
    // (still inline in cmd_serve_http) drains the queue.
    {
        let sink: std::sync::Arc<dyn ministr_api::IndexJobSink> = std::sync::Arc::new(
            crate::PostgresIndexJobSink::new(Arc::clone(&pool), None),
        );
        daemon_adapters.index_job_sink = Some(sink);
    }

    // F2.6 — Atlas v0 pilot. Manifest + per-slug query stubs.
    // Migrated from cmd_serve_http inline branch in F31.2b-ii-C.
    // Mounted behind `ministr:read` so any paid-tier token admits;
    // the (still-inline) F2.3 `AtlasAccessRule` runs higher up in the
    // composed stack and short-circuits unauthenticated / free callers
    // with the 402 paywall.
    {
        let atlas_router =
            ministr_atlas::atlas_routes(ministr_atlas::AtlasState::from_seed_list());
        let atlas_protected = ministr_mcp::auth::scope_protected_router(
            atlas_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(atlas_protected);
        tracing::info!(
            count = ministr_atlas::ATLAS_SEED_REPOS.len(),
            "atlas v0 mounted via CloudRouterMounter — GET /atlas/manifest.json + /atlas/{{slug}}/*"
        );
    }

    // F1.5 sub-bullet 3 — Stripe webhook receiver. Public route
    // (Stripe is the caller); the signature check inside is the only
    // auth. Mounts only when MINISTR_STRIPE_WEBHOOK_SECRET is set.
    // Migrated from cmd_serve_http inline branch in F31.2b-ii-D.
    if let Some(stripe_secret) = trimmed_env("MINISTR_STRIPE_WEBHOOK_SECRET") {
        let stripe_router = crate::billing::stripe::stripe_webhook_routes(
            crate::StripeWebhookState::new(Arc::clone(&pool), stripe_secret),
        );
        router = router.merge(stripe_router);
        tracing::info!("stripe webhook mounted via CloudRouterMounter — POST /webhooks/stripe");
    }

    // F3.1b-ii-c — Resend bounce webhook. Public route (Resend is the
    // caller); svix signature is the only auth. Mounts only when
    // MINISTR_RESEND_WEBHOOK_SECRET is set. Migrated from cmd_serve_http
    // inline branch in F31.2b-ii-D.
    if let Some(resend_secret) = trimmed_env("MINISTR_RESEND_WEBHOOK_SECRET") {
        let resend_router = crate::resend_webhook_routes(
            crate::ResendWebhookState::new(Arc::clone(&pool), resend_secret),
        );
        router = router.merge(resend_router);
        tracing::info!("resend webhook mounted via CloudRouterMounter — POST /webhooks/resend");
    }

    // F5.1-b — SAML SP endpoints. Public routes (IdP doesn't carry
    // bearer tokens); per-org config in `saml_configs` gates whether
    // a given org has SAML SSO enabled. Migrated from cmd_serve_http
    // inline branch in F31.2b-ii-E.
    {
        let saml_router =
            crate::saml_routes(crate::SamlState::new(Arc::clone(&pool)));
        router = router.merge(saml_router);
        tracing::info!(
            "saml SP routes mounted via CloudRouterMounter — GET /orgs/{{id}}/saml/metadata.xml + /login"
        );
    }

    // F1.4 sub-bullet 4 — billing endpoint (GET /api/v1/billing/usage).
    // Mounted behind `ministr:read`. Migrated from cmd_serve_http
    // inline branch in F31.2b-ii-F.
    {
        let billing_router = crate::billing_routes(
            crate::BillingState::from_arc(Arc::clone(&pool)),
        );
        let billing_protected = ministr_mcp::auth::scope_protected_router(
            billing_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(billing_protected);
        tracing::info!(
            "billing endpoint mounted via CloudRouterMounter — GET /api/v1/billing/usage"
        );
    }

    // F3.3a — per-org usage dashboard endpoint (GET /api/v1/orgs/{id}/usage).
    // Aggregates `usage_rollups` across `org_members`. Owner/admin only
    // (enforced in handler). Mounted behind `ministr:read`. Migrated
    // from cmd_serve_http inline branch in F31.2b-ii-F.
    {
        let org_usage_router = crate::org_usage_routes(
            crate::orgs::OrgUsageState::from_arc(Arc::clone(&pool)),
        );
        let org_usage_protected = ministr_mcp::auth::scope_protected_router(
            org_usage_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(org_usage_protected);
        tracing::info!(
            "org usage endpoint mounted via CloudRouterMounter — GET /api/v1/orgs/{{id}}/usage"
        );
    }

    // F5.1-d — per-org SAML config CRUD. Owner-only ACL enforced
    // inside each handler via assert_owner_or_admin; the
    // scope_protected_router wrapper supplies the Tenant extension
    // that the ACL reads. Migrated from cmd_serve_http inline branch
    // in F31.2b-ii-G.
    {
        let saml_config_state = crate::SamlState::new(Arc::clone(&pool));
        let saml_config_router = crate::saml_config_routes(saml_config_state);
        let saml_config_protected = ministr_mcp::auth::scope_protected_router(
            saml_config_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(saml_config_protected);
        tracing::info!(
            "saml config CRUD mounted via CloudRouterMounter — POST/GET/DELETE /api/v1/orgs/{{id}}/saml/config"
        );
    }

    // F5.2-d — per-org OIDC config CRUD. Same shape as the F5.1-d
    // SAML block: owner-only ACL inside each handler.
    {
        let oidc_config_state = crate::OidcState::new(Arc::clone(&pool));
        let oidc_config_router = crate::oidc_config_routes(oidc_config_state);
        let oidc_config_protected = ministr_mcp::auth::scope_protected_router(
            oidc_config_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(oidc_config_protected);
        tracing::info!(
            "oidc config CRUD mounted via CloudRouterMounter — POST/GET/DELETE /api/v1/orgs/{{id}}/oidc/config"
        );
    }

    // F5.3-d-ii-config — per-org SIEM config CRUD. Owner-only ACL
    // inside each handler.
    {
        let siem_config_state = crate::SiemConfigState::from_arc(Arc::clone(&pool));
        let siem_config_router = crate::siem_config_routes(siem_config_state);
        let siem_config_protected = ministr_mcp::auth::scope_protected_router(
            siem_config_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(siem_config_protected);
        tracing::info!(
            "siem config CRUD mounted via CloudRouterMounter — POST/GET/DELETE /api/v1/orgs/{{id}}/siem/config"
        );
    }

    // F3.7a — per-org audit list endpoint (GET /api/v1/orgs/{id}/audit).
    // Owner / admin only; members get 403. Mounted behind `ministr:read`
    // so any authenticated org member's token can call it; the role
    // check inside is the actual gate. F5.3-c-ii-archive-read attaches
    // an optional archive dir for `/audit/archived` lookups.
    {
        let mut audit_state = crate::AuditState::from_arc(Arc::clone(&pool));
        if let Some(dir) = trimmed_env("MINISTR_AUDIT_ARCHIVE_DIR") {
            audit_state = audit_state.with_archive_dir(dir);
            tracing::info!("audit archive dir wired (MINISTR_AUDIT_ARCHIVE_DIR)");
        }
        let audit_router = crate::audit_routes(audit_state);
        let audit_protected = ministr_mcp::auth::scope_protected_router(
            audit_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(audit_protected);
        tracing::info!(
            "audit endpoint mounted via CloudRouterMounter — GET /api/v1/orgs/{{id}}/audit"
        );
    }

    Ok(CloudMountOutput {
        router,
        daemon_adapters,
        server_adapters,
        oauth_adapters: CloudOAuthAdapters::default(),
        admin_adapters: CloudAdminAdapters::default(),
        shutdown: None,
    })
}

/// Read an env var, trim, and treat the empty string as absent.
fn trimmed_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

impl RevocationHandle for RevocationShutdownHandle {
    fn shutdown_future(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.shutdown.notified().await;
        })
    }

    fn is_revoked(&self) -> bool {
        self.revoked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl ClassicCloudMounter {
    /// Helper for callers that want the revocation handle as the
    /// MIT-seam trait object (`Arc<dyn RevocationHandle>`).
    #[must_use]
    pub fn revocation_handle_dyn(handle: RevocationShutdownHandle) -> Arc<dyn RevocationHandle> {
        Arc::new(handle)
    }
}
