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
#[allow(clippy::too_many_lines)] // sequential migration of inline cloud branch — each block is one route/adapter
pub async fn mount_cloud_routes(
    input: &CloudMountInput,
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

    // F5.4-a — license-key gate. Two env vars unset → community mode
    // (no gate, log info). Both set + valid → Enterprise mode. Invalid
    // → refuse to boot. F5.4-e-revoke-* spawns a background refresh
    // task and returns a shutdown handle so graceful_shutdown fires on
    // mid-flight revocation. Migrated from cmd_serve_http inline branch
    // in F31.2b-ii-L.
    let mut shutdown: Option<std::sync::Arc<dyn RevocationHandle>> = None;
    let license_claims: Option<std::sync::Arc<crate::LicenseClaims>> =
        match crate::validate_license_from_env().await {
            Ok(None) => {
                tracing::info!(
                    "ClassicCloudMounter: community mode (no MINISTR_LICENSE_KEY / MINISTR_LICENSE_PUBLIC_KEY set)"
                );
                None
            }
            Ok(Some(claims)) => {
                tracing::info!(
                    license = %crate::render_license_summary(&claims),
                    "Enterprise license validated via CloudRouterMounter"
                );
                if let Some((url, cache_path, grace_secs)) = crate::revocation_url_config() {
                    let refresh_secs = crate::revocation_refresh_secs();
                    let jwt = std::env::var("MINISTR_LICENSE_KEY").unwrap_or_default();
                    let hash = crate::license_jwt_id_hash(&jwt);
                    let handle = RevocationShutdownHandle::new();
                    shutdown = Some(std::sync::Arc::new(handle.clone()));
                    crate::spawn_refresh_task(
                        url,
                        cache_path,
                        refresh_secs,
                        grace_secs,
                        hash,
                        Some(handle),
                    );
                }
                Some(std::sync::Arc::new(claims))
            }
            Err(e) => {
                return Err(ApiError {
                    code: "cloud_license_refused_boot".into(),
                    message: format!(
                        "license gate refused boot: {e}. Set both MINISTR_LICENSE_KEY + \
                         MINISTR_LICENSE_PUBLIC_KEY, OR unset both to run in community mode."
                    ),
                });
            }
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
    let mut oauth_adapters = CloudOAuthAdapters::default();
    let mut admin_adapters = CloudAdminAdapters::default();

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

    // F2.1 — GitHub App installation-token minter for private-repo
    // cloning. Built independently of the GitHub OAuth IdP (F1.3) so a
    // deployment can enable App-driven clones without also enabling
    // the user-facing GitHub sign-in flow (or vice versa). Migrated
    // from cmd_serve_http inline branch in F31.2b-ii-J.
    if let (Some(app_id), Some(pem)) = (
        trimmed_env("MINISTR_GITHUB_APP_ID"),
        std::env::var("MINISTR_GITHUB_APP_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty()),
    ) {
        match crate::GitHubAppClient::new(app_id.clone(), &pem) {
            Ok(client) => {
                let minter: std::sync::Arc<dyn ministr_api::InstallationTokenMinter> =
                    std::sync::Arc::new(client);
                daemon_adapters.installation_minter = Some(minter);
                tracing::info!(
                    app_id = %app_id,
                    "GitHubAppClient wired via CloudRouterMounter — private-repo cloning via installation tokens"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "GitHubAppClient disabled — MINISTR_GITHUB_APP_ID/PRIVATE_KEY rejected"
                );
            }
        }
    } else if trimmed_env("MINISTR_GITHUB_APP_ID").is_some()
        || std::env::var("MINISTR_GITHUB_APP_PRIVATE_KEY")
            .ok()
            .is_some_and(|s| !s.trim().is_empty())
    {
        tracing::warn!(
            "GitHub App NOT wired — both MINISTR_GITHUB_APP_ID and MINISTR_GITHUB_APP_PRIVATE_KEY must be set"
        );
    }

    // F3.4a + F5.5-a-plan-lookup — OAuth-store extensions.
    // `PostgresApiKeyResolver` lets `mst_pk_…` service-account tokens
    // authenticate via the `api_keys` table; `PostgresPlanResolver`
    // resolves a Tenant's `plan` from `users.plan_id` instead of the
    // hardcoded Pro default. Both fed via `CloudOAuthAdapters` slots.
    {
        let api_key_resolver =
            crate::PostgresApiKeyResolver::new((*pool).clone()).into_dyn();
        oauth_adapters.api_key_resolver = Some(api_key_resolver);
        let plan_resolver =
            crate::PostgresPlanResolver::new((*pool).clone()).into_dyn();
        oauth_adapters.plan_resolver = Some(plan_resolver);
        tracing::info!(
            "PostgresApiKeyResolver + PostgresPlanResolver wired via CloudRouterMounter"
        );
    }

    // F5.5-b-persist-read — `PostgresSlaWindowStore` feeds the /sla
    // endpoint's `latency.window_30d_max_p95_ms` field. Migrated from
    // cmd_serve_http inline branch in F31.2b-ii-J.
    {
        admin_adapters.sla_window_store = Some(
            crate::PostgresSlaWindowStore::new((*pool).clone()).into_dyn(),
        );
        tracing::info!(
            "PostgresSlaWindowStore wired via CloudRouterMounter"
        );
    }

    // PHASE3 chunk 1 — `PostgresCorporaRepo` makes Postgres the
    // source of truth for which corpora exist, so the list survives
    // ACA pod recycling (the on-disk corpora.json is pod-ephemeral).
    // Migrated from cmd_serve_http inline branch in F31.2b-ii-J.
    {
        let repo: std::sync::Arc<dyn ministr_api::CorporaRepo> = std::sync::Arc::new(
            crate::PostgresCorporaRepo::new(Arc::clone(&pool), None),
        );
        server_adapters.corpora_repo = Some(repo);
        tracing::info!(
            "PostgresCorporaRepo wired via CloudRouterMounter"
        );
    }

    // F6.1-g — `PostgresSessionStorage` + `PostgresDropsLedger` for
    // agent-session persistence across pod recycle. Migrated in
    // F31.2b-ii-K.
    {
        let storage: std::sync::Arc<dyn ministr_api::SessionStorage> = std::sync::Arc::new(
            crate::PostgresSessionStorage::from_arc(Arc::clone(&pool)),
        );
        let ledger: std::sync::Arc<dyn ministr_api::DropsLedger> = std::sync::Arc::new(
            crate::PostgresDropsLedger::from_arc(Arc::clone(&pool)),
        );
        server_adapters.session_storage = Some(storage);
        server_adapters.drops_ledger = Some(ledger);
        tracing::info!(
            "PostgresSessionStorage + PostgresDropsLedger wired via CloudRouterMounter"
        );
    }

    // F6.2-c — `CloudSessionBundleStore` for signed-URL bundle export.
    // Requires Azure account + signing secret + cloud base URL all set;
    // returns None otherwise (handler falls back to inline-tar shape).
    match crate::build_session_bundle_store_from_env(
        trimmed_env("MINISTR_CLOUD_BASE_URL").as_deref(),
    ) {
        Ok(Some(store)) => {
            let store: std::sync::Arc<dyn ministr_api::SessionBundleStore> =
                std::sync::Arc::new(store);
            server_adapters.session_bundle_store = Some(store);
            tracing::info!(
                "session bundle store wired via CloudRouterMounter — uploads to blob + returns signed URL"
            );
        }
        Ok(None) => {
            tracing::debug!(
                "session bundle store disabled — inline-tar export shape continues"
            );
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "session bundle store construction failed — falling back to inline tar"
            );
        }
    }

    // PHASE3 chunk 5 — blob backend (Azure Blob / filesystem). Build
    // once; feeds both the `BlobCorpusRestorer` (lazy bundle downloads
    // on cold start) and the `BlobBackendSink` (post-ingest uploads).
    // Migrated from cmd_serve_http inline branch in F31.2b-ii-K.
    let blob_backend = crate::build_blob_backend_from_env().map_err(|e| ApiError {
        code: "cloud_blob_backend_open_failed".into(),
        message: format!("build blob backend from env: {e}"),
    })?;
    let blob_backend_arc = blob_backend.map(std::sync::Arc::new);

    if let Some(backend_arc) = blob_backend_arc.as_ref() {
        // F6.1-f — lazy on-demand bundle restorer. First query that
        // touches a corpus_id missing from in-memory but present in
        // `cloud_corpora` triggers `BlobCorpusRestorer::download`.
        let restorer: std::sync::Arc<dyn ministr_api::CorpusRestorer> = std::sync::Arc::new(
            crate::BlobCorpusRestorer::new(std::sync::Arc::clone(backend_arc)),
        );
        server_adapters.corpus_restorer = Some(restorer);
        tracing::info!(
            "BlobCorpusRestorer wired via CloudRouterMounter — first query lazy-downloads bundles"
        );

        // PHASE2 chunk 4 — durable corpus uploads. cmd_serve_http
        // owns the completion channel + reactor (BlobSink::enqueue_upload
        // calls on the mpsc rx); the mounter just supplies the sink.
        let sink: std::sync::Arc<dyn ministr_api::BlobSink> = std::sync::Arc::new(
            crate::BlobBackendSink::new(
                std::sync::Arc::clone(backend_arc),
                input.resolved_model.clone(),
            ),
        );
        daemon_adapters.blob_sink = Some(sink);
        tracing::info!(
            "BlobBackendSink wired via CloudRouterMounter — bundles uploaded after every ingest"
        );
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

    // F3.1c-i + F2.4 — outbound Stripe client. Used by orgs (Customer
    // creation at org-creation), checkout/portal routes, and the
    // github-signin Customer-seed hook. Built once when
    // MINISTR_STRIPE_SECRET_KEY is set. Migrated in F31.2b-ii-M.
    let stripe_client = trimmed_env("MINISTR_STRIPE_SECRET_KEY").and_then(|key| {
        match crate::StripeClient::new(key) {
            Ok(c) => {
                tracing::info!(
                    "stripe outbound client built via CloudRouterMounter — Customer creation + Meters API enabled"
                );
                Some(std::sync::Arc::new(c))
            }
            Err(e) => {
                tracing::warn!(error = %e, "stripe client disabled — STRIPE_SECRET_KEY rejected");
                None
            }
        }
    });

    // F3.5a — outbound webhook dispatcher. Shared between the
    // webhook-routes router (CRUD + /test endpoint, chunk N) AND the
    // F3.5b-i WebhookFanoutSink below so both paths reuse one TLS
    // connection pool. Migrated in F31.2b-ii-M.
    let webhook_dispatcher = match crate::WebhookDispatcher::new() {
        Ok(d) => Some(std::sync::Arc::new(d)),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "webhook dispatcher init failed; webhooks + fan-out disabled"
            );
            None
        }
    };

    // F3.7a + F3.5b-i + F5.3-d-i — audit sink chain. Postgres always
    // lands first (durable BEFORE outbound dispatch). WebhookFanoutSink
    // joins when the dispatcher initialised; SplunkHecSink joins when
    // SIEM env is set; PerOrgSiemDispatcher always joins (it no-ops
    // for events without org_id or for orgs without a SIEM config).
    // Used by orgs_routes/api_keys_routes/oidc/github_signin.
    let cloud_audit_sink: std::sync::Arc<dyn ministr_api::AuditSink> = {
        let postgres_audit: std::sync::Arc<dyn ministr_api::AuditSink> = std::sync::Arc::new(
            crate::PostgresAuditSink::from_arc(Arc::clone(&pool)),
        );
        let mut sinks: Vec<std::sync::Arc<dyn ministr_api::AuditSink>> =
            vec![std::sync::Arc::clone(&postgres_audit)];
        let mut chain_desc = String::from("PostgresAuditSink");
        if let Some(d) = webhook_dispatcher.as_ref() {
            let fanout = crate::WebhookFanoutSink::new(
                Arc::clone(&pool),
                std::sync::Arc::clone(d),
            );
            sinks.push(std::sync::Arc::new(fanout));
            chain_desc.push_str(" → WebhookFanoutSink");
        }
        if let Some(hec) = crate::SplunkHecSink::from_env() {
            sinks.push(std::sync::Arc::new(hec));
            chain_desc.push_str(" → SplunkHecSink");
        }
        {
            let per_org = crate::PerOrgSiemDispatcher::new(Arc::clone(&pool));
            sinks.push(std::sync::Arc::new(per_org));
            chain_desc.push_str(" → PerOrgSiemDispatcher");
        }
        tracing::info!(chain = %chain_desc, "audit pipeline wired via CloudRouterMounter");
        if sinks.len() == 1 {
            postgres_audit
        } else {
            std::sync::Arc::new(crate::ChainedAuditSink::new(sinks))
        }
    };

    // F3.1a/b — orgs CRUD + member listing + magic-link invites.
    // Migrated in F31.2b-ii-M.
    {
        let mut orgs_state = crate::OrgsState::from_arc(Arc::clone(&pool));
        if let Some(base) = trimmed_env("MINISTR_CLOUD_BASE_URL") {
            orgs_state = orgs_state.with_cloud_base_url(&base);
        }
        if let Some(stripe) = stripe_client.as_ref() {
            orgs_state = orgs_state.with_stripe(std::sync::Arc::clone(stripe));
        }
        orgs_state = orgs_state.with_audit(std::sync::Arc::clone(&cloud_audit_sink));
        let mailer = crate::build_mail_sender_from_env();
        orgs_state = orgs_state.with_mailer(mailer);
        // F5.4-b — thread the cached license claims so the invite
        // handler enforces the contracted seat cap. None → community
        // mode (no cap).
        if let Some(claims) = license_claims.as_ref() {
            orgs_state = orgs_state.with_license(std::sync::Arc::clone(claims));
        }
        let orgs_router = crate::orgs_routes(orgs_state);
        let orgs_protected = ministr_mcp::auth::scope_protected_router(
            orgs_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(orgs_protected);
        tracing::info!(
            "orgs endpoints mounted via CloudRouterMounter — POST/GET /api/v1/orgs, members, invites"
        );
    }

    // F3.4a — service-account API keys (mint, list, revoke).
    // Cloud-only; mounted behind `ministr:read` because every action
    // targets the caller's own keys. Migrated in F31.2b-ii-M.
    {
        let api_keys_router = crate::api_keys_routes(
            crate::ApiKeysState::new((*pool).clone())
                .with_audit(std::sync::Arc::clone(&cloud_audit_sink)),
        );
        let api_keys_protected = ministr_mcp::auth::scope_protected_router(
            api_keys_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(api_keys_protected);
        tracing::info!(
            "api_keys endpoints mounted via CloudRouterMounter — POST/GET/DELETE /api/v1/api_keys"
        );
    }

    // F3.5a — outbound webhook subscriptions CRUD + /test. Owner/admin
    // ACL inside the handlers. Re-uses `webhook_dispatcher` so CRUD
    // and the audit-fanout sink share TLS pool. Migrated in F31.2b-ii-N.
    if let Some(dispatcher) = webhook_dispatcher.as_ref() {
        let webhooks_state = crate::WebhooksState::new(
            Arc::clone(&pool),
            std::sync::Arc::clone(dispatcher),
        );
        let webhooks_router = crate::webhooks_routes(webhooks_state);
        let webhooks_protected = ministr_mcp::auth::scope_protected_router(
            webhooks_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(webhooks_protected);
        tracing::info!(
            "webhook endpoints mounted via CloudRouterMounter — POST/GET/DELETE /api/v1/orgs/{{id}}/webhooks"
        );
    }

    // F5.2-b/c — OIDC RP login + callback endpoints. Public routes
    // (browser-initiated; IdP doesn't carry bearer tokens). Wired to
    // the cloud's OAuth store (bearer tokens indistinguishable
    // downstream), the cloud base URL (registered RP redirect_uri),
    // and the audit chain (`oidc.login` events). Migrated in F31.2b-ii-N.
    {
        let mut oidc_state = crate::OidcState::new(Arc::clone(&pool))
            .with_oauth_store(oauth_store.clone())
            .with_audit(std::sync::Arc::clone(&cloud_audit_sink));
        if let Some(base_url) = trimmed_env("MINISTR_CLOUD_BASE_URL") {
            oidc_state = oidc_state.with_cloud_base_url(base_url);
        }
        let oidc_router = crate::oidc_routes(oidc_state);
        router = router.merge(oidc_router);
        tracing::info!(
            "oidc RP routes mounted via CloudRouterMounter — GET /orgs/{{id}}/oidc/{{login,callback}}"
        );
    }

    // F2.4 — Stripe Checkout + Customer Portal routes. Requires the
    // outbound stripe client AND the cloud base URL. Mounted behind
    // `ministr:read`. Migrated in F31.2b-ii-N.
    if let (Some(stripe), Some(base_url)) = (
        stripe_client.as_ref(),
        trimmed_env("MINISTR_CLOUD_BASE_URL"),
    ) {
        let catalog: std::sync::Arc<dyn crate::PriceCatalog> =
            std::sync::Arc::new(crate::EnvPriceCatalog::new(
                trimmed_env("MINISTR_STRIPE_PRICE_PRO"),
                trimmed_env("MINISTR_STRIPE_PRICE_TEAM"),
            ));
        let checkout_state = crate::CheckoutState::new(
            std::sync::Arc::clone(stripe),
            Arc::clone(&pool),
            catalog,
            base_url,
        );
        let checkout_router = crate::checkout_routes(checkout_state);
        let checkout_protected = ministr_mcp::auth::scope_protected_router(
            checkout_router,
            oauth_store.clone(),
            "ministr:read",
        );
        router = router.merge(checkout_protected);
        tracing::info!(
            "stripe checkout + portal mounted via CloudRouterMounter — POST /api/v1/billing/{{checkout,portal}}"
        );
    }

    // F1.3 — GitHub sign-in flow. Mounted when GitHub OAuth App
    // credentials AND a public base URL are present. Public routes
    // (sign-in must be reachable without an existing token). Migrated
    // in F31.2b-ii-N.
    if let (Some(cid), Some(secret), Some(base_url)) = (
        trimmed_env("MINISTR_GITHUB_CLIENT_ID"),
        trimmed_env("MINISTR_GITHUB_CLIENT_SECRET"),
        trimmed_env("MINISTR_CLOUD_BASE_URL"),
    ) {
        match crate::GitHubIdp::new(cid, secret) {
            Ok(idp) => {
                let mut state = crate::GitHubSigninState::new(
                    std::sync::Arc::new(idp),
                    (*pool).clone(),
                    oauth_store.clone(),
                    base_url.clone(),
                );
                if let Some(stripe) = stripe_client.as_ref() {
                    state = state.with_stripe(std::sync::Arc::clone(stripe));
                }
                state = state.with_audit(std::sync::Arc::clone(&cloud_audit_sink));
                if let Some(raw) = trimmed_env("MINISTR_WEB_ALLOWED_ORIGINS") {
                    let origins: Vec<String> = raw
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !origins.is_empty() {
                        state = state.with_web_allowed_origins(origins);
                    }
                }
                router = router.merge(crate::github_signin_routes(state));
                tracing::info!(
                    base_url = %base_url,
                    stripe_customer_seed = stripe_client.is_some(),
                    "github sign-in mounted via CloudRouterMounter — GET /auth/github/{{start,callback}}"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "github sign-in disabled — invalid credentials"
                );
            }
        }
    } else if trimmed_env("MINISTR_GITHUB_CLIENT_ID").is_some()
        || trimmed_env("MINISTR_GITHUB_CLIENT_SECRET").is_some()
        || trimmed_env("MINISTR_CLOUD_BASE_URL").is_some()
    {
        tracing::warn!(
            "github sign-in NOT mounted — MINISTR_GITHUB_CLIENT_ID, MINISTR_GITHUB_CLIENT_SECRET, and MINISTR_CLOUD_BASE_URL must ALL be set"
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
        oauth_adapters,
        admin_adapters,
        shutdown,
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
