//! CLI subcommand implementations for the ministr CLI.
//!
//! Each `pub(crate)` function corresponds to a CLI subcommand dispatched from
//! [`main`](crate::main). This module keeps `main.rs` focused on argument
//! parsing and dispatch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt as _;

use ministr_core::index::VectorIndex as _;
use ministr_core::index::VectorIndexLoad as _;

use crate::infra;
use crate::ingestion;

// ---------------------------------------------------------------------------
// ministr serve --transport stdio  →  DELETED
//
// The monolithic per-corpus stdio primary (flock + deterministic TCP
// port + in-process MinistrServer, with HTTP secondaries) was a second,
// redundant "one primary serves many clients" subsystem alongside the
// UDS daemon. stdio now ALWAYS runs the thin proxy, which self-spawns
// the headless daemon (`ministr __daemon`). See `cmd_serve_proxy_stdio`.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ministr serve --transport http
// ---------------------------------------------------------------------------

/// Cloud-mode environment knobs honoured by `cmd_serve_http`.
///
/// `MINISTR_CLOUD_DATA_DIR` — persistence directory for OAuth + admin
/// `SQLite` databases. When unset the server runs with in-memory state.
/// `MINISTR_GITHUB_WEBHOOK_SECRET` — enables `/webhook/github` and
/// authenticates incoming push events via HMAC-SHA256.
/// `MINISTR_PG_URL` — libpq connection string for the cloud Postgres
/// database. When set, OAuth state is persisted to Postgres
/// (`OAuthBackend::Postgres`) and takes precedence over the `SQLite`
/// path; multi-pod cloud deployments rely on this so every pod shares
/// the same `oauth_clients`/`oauth_codes`/`oauth_tokens` rows.
/// `MINISTR_STRIPE_WEBHOOK_SECRET` — endpoint signing secret from the
/// Stripe dashboard (prefixed `whsec_`). When set, mounts
/// `POST /webhooks/stripe`; the handler rejects all events without a
/// matching signature.
/// `MINISTR_STRIPE_SECRET_KEY` — Stripe API secret key (prefixed
/// `sk_test_` or `sk_live_`) used for OUTBOUND calls to Stripe (Customer
/// creation on signup in F1.5; Meter events later). When unset, the
/// cloud runs without ever calling Stripe; the GitHub sign-in flow
/// still works, just without seeding a Stripe Customer.
/// `MINISTR_STRIPE_PRICE_PRO` / `MINISTR_STRIPE_PRICE_TEAM` — Stripe
/// price IDs configured in the dashboard for Pro / Team subscription
/// products (F2.4). When unset, `POST /api/v1/billing/checkout` for
/// the corresponding plan returns 503 `price_not_configured`. Pricing
/// matches §3 of the roadmap.
/// `MINISTR_GITHUB_CLIENT_ID` / `MINISTR_GITHUB_CLIENT_SECRET` — the
/// GitHub OAuth App credentials registered on github.com. Both must be
/// present together for the F1.3 `/auth/github/*` sign-in routes to
/// mount; absence keeps the cloud running on the OAuth-only code-grant
/// path (self-hosted single-user serve).
/// `MINISTR_CLOUD_BASE_URL` — absolute base URL the public Internet
/// reaches the cloud at (e.g. `https://mcp.ministr.ai`). Required when
/// GitHub sign-in is enabled because the `redirect_uri` passed to the
/// GitHub authorize endpoint must exactly match the value registered
/// in the App's settings.
/// `MINISTR_GITHUB_APP_ID` / `MINISTR_GITHUB_APP_PRIVATE_KEY` — the
/// GitHub App credentials for private-repo cloning (F2.1). The private
/// key is the multi-line PEM downloaded from the App settings page —
/// pass it verbatim (Container Apps secrets handle newlines correctly).
/// Both must be present together. When unset, `clone_repo` requests
/// carrying `github_installation_id` fail with 400.
struct CloudEnv {
    data_dir: Option<PathBuf>,
    webhook_secret: Option<String>,
    pg_url: Option<String>,
    stripe_webhook_secret: Option<String>,
    stripe_secret_key: Option<String>,
    github_client_id: Option<String>,
    github_client_secret: Option<String>,
    cloud_base_url: Option<String>,
    github_app_id: Option<String>,
    github_app_private_key: Option<String>,
    stripe_price_pro: Option<String>,
    stripe_price_team: Option<String>,
    /// F3.6-b-ii-a — comma-separated CORS allowlist
    /// (`MINISTR_CORS_ALLOWED_ORIGINS`). Unset (`None`) ⇒ no CORS
    /// layer mounted ⇒ self-hosted serve has zero browser exposure.
    /// Each entry is a full origin (`https://ministr.ai`).
    cors_allowed_origins: Option<String>,
}

fn read_cloud_env() -> CloudEnv {
    let trimmed = |k: &str| -> Option<String> {
        std::env::var(k)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    CloudEnv {
        data_dir: std::env::var("MINISTR_CLOUD_DATA_DIR").ok().map(PathBuf::from),
        webhook_secret: std::env::var("MINISTR_GITHUB_WEBHOOK_SECRET").ok(),
        pg_url: trimmed("MINISTR_PG_URL"),
        stripe_webhook_secret: trimmed("MINISTR_STRIPE_WEBHOOK_SECRET"),
        stripe_secret_key: trimmed("MINISTR_STRIPE_SECRET_KEY"),
        github_client_id: trimmed("MINISTR_GITHUB_CLIENT_ID"),
        github_client_secret: trimmed("MINISTR_GITHUB_CLIENT_SECRET"),
        cloud_base_url: trimmed("MINISTR_CLOUD_BASE_URL"),
        github_app_id: trimmed("MINISTR_GITHUB_APP_ID"),
        // Don't trim the PEM body — that would strip newlines + drop
        // the trailing footer line. Just reject when entirely blank.
        github_app_private_key: std::env::var("MINISTR_GITHUB_APP_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty()),
        stripe_price_pro: trimmed("MINISTR_STRIPE_PRICE_PRO"),
        stripe_price_team: trimmed("MINISTR_STRIPE_PRICE_TEAM"),
        cors_allowed_origins: trimmed("MINISTR_CORS_ALLOWED_ORIGINS"),
    }
}

/// F3.6-b-ii-a — parse a comma-separated CORS allowlist into
/// validated origins. Each entry must look like a full origin
/// (`scheme://host[:port]`). Malformed entries are dropped with a
/// warn log so a typo in one slot doesn't disable CORS entirely.
///
/// Returns `None` when input is `None` or every entry was rejected;
/// `Some(vec)` otherwise. The empty-output case logs an explicit
/// warn so an operator who typoed every entry hears about it.
#[must_use]
fn parse_cors_allowed_origins(raw: Option<&str>) -> Option<Vec<String>> {
    let raw = raw?;
    let parsed: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            // Minimal sanity check: must contain "://" and no whitespace.
            // axum + tower-http will reject malformed values at layer-build
            // time too, but we filter+log here so the boot log is
            // operator-debuggable.
            if s.contains("://") && !s.chars().any(char::is_whitespace) {
                Some(s.to_owned())
            } else {
                tracing::warn!(origin = %s, "MINISTR_CORS_ALLOWED_ORIGINS — skipping malformed entry");
                None
            }
        })
        .collect();
    if parsed.is_empty() {
        tracing::warn!(
            "MINISTR_CORS_ALLOWED_ORIGINS was set but no valid origins parsed — CORS disabled"
        );
        return None;
    }
    Some(parsed)
}

/// F3.6-b-ii-a — build a `CorsLayer` from a parsed allowlist.
///
/// Methods admitted: GET / POST / PUT / DELETE / OPTIONS — the union
/// of methods the cloud HTTP surface exposes today.
/// Headers admitted: `Authorization` + `Content-Type` (the only
/// non-CORS-safelisted headers callers send) — request-side. Browsers
/// auto-send the safelisted ones.
///
/// Credentials NOT allowed: agents authenticate via `Authorization:
/// Bearer …` rather than cookies; widening to credentials-included
/// would expand the attack surface for no benefit.
fn build_cors_layer(origins: &[String]) -> tower_http::cors::CorsLayer {
    use axum::http::{HeaderName, HeaderValue, Method};
    use tower_http::cors::{AllowOrigin, CorsLayer};

    let header_origins: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(header_origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
        ])
        .max_age(std::time::Duration::from_secs(600))
}

fn build_admin_state(env: &CloudEnv, corpus_count: usize) -> Result<ministr_mcp::admin::AdminState> {
    let state = match env.data_dir.as_deref() {
        Some(dir) => {
            std::fs::create_dir_all(dir)
                .into_diagnostic()
                .wrap_err_with(|| format!("create cloud data dir {}", dir.display()))?;
            ministr_mcp::admin::AdminState::persistent(
                &dir.join("jobs.db"),
                env.webhook_secret.clone(),
            )
            .into_diagnostic()
            .wrap_err("open persistent admin state")?
        }
        None => ministr_mcp::admin::AdminState::in_memory(env.webhook_secret.clone()),
    };
    state.set_corpus_count(corpus_count);
    Ok(state)
}

async fn build_oauth_store(
    cfg: ministr_mcp::auth::OAuthConfig,
    data_dir: Option<&Path>,
    pg_url: Option<&str>,
) -> Result<ministr_mcp::auth::OAuthStore> {
    // Selector order — Postgres wins when MINISTR_PG_URL is set so
    // multi-pod cloud deployments cannot accidentally fall through to a
    // pod-local SQLite file. SQLite is the self-hosted persistent
    // option; in-memory is dev-only.
    if let Some(url) = pg_url {
        return ministr_mcp::auth::OAuthStore::postgres(cfg, url)
            .await
            .into_diagnostic()
            .wrap_err("open postgres oauth store");
    }
    match data_dir {
        Some(dir) => ministr_mcp::auth::OAuthStore::persistent(cfg, &dir.join("oauth.db"))
            .into_diagnostic()
            .wrap_err("open persistent oauth store"),
        None => Ok(ministr_mcp::auth::OAuthStore::new(cfg)),
    }
}

/// `ministr serve --transport http` — Streamable HTTP MCP server.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) async fn cmd_serve_http(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    host: &str,
    port: u16,
    oauth_config: Option<ministr_mcp::auth::OAuthConfig>,
    resolved_model: &str,
    repo_config_dir: Option<&Path>,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let (server, ctx, _coherence_handle) = infra::build_server(
        corpus_paths,
        config_path,
        config,
        Some(resolved_model),
        resolved_dimension,
        rerank_depth,
    )
    .await?;

    // PHASE3 chunk 5 — replaces PHASE2's boot-time bulk download.
    //
    // Build the blob backend once. `None` on self-hosted serve (no
    // `MINISTR_BLOB_*` env), in which case every blob path below
    // collapses to a no-op and the binary behaves exactly like before.
    // The serve pod no longer pulls every bundle at boot; instead the
    // first query against a corpus_id triggers an on-demand restore
    // via `CorpusRegistry::get` → `ensure_present` → `BlobCorpusRestorer`.
    let blob_backend = ministr_cloud::build_blob_backend_from_env()
        .into_diagnostic()
        .wrap_err("build blob backend from env")?;
    let blob_backend_arc = blob_backend.map(Arc::new);

    // Cloud env + Postgres pool. Hoisted above the corpus registry
    // build so we can wire a `PostgresCorporaRepo` BEFORE `restore()`
    // and the repo is the source of truth at cold start. Self-hosted
    // serve leaves cloud_pool = None and skips the repo entirely;
    // `restore()` falls back to the on-disk `corpora.json` as before.
    let cloud_env = read_cloud_env();
    let cloud_pool = if let Some(pg_url) = cloud_env.pg_url.as_deref() {
        let pool = ministr_cloud::connect(pg_url)
            .into_diagnostic()
            .wrap_err("open cloud postgres pool")?;
        // Auto-apply F1.2 + later migrations on every pod boot. The
        // runner short-circuits when the schema is up to date so this
        // is cheap on warm starts; on a fresh DB it creates the
        // users / orgs / corpora / usage_events / audit_events tables
        // (and PHASE3's `cloud_corpora`) before any handler can query
        // them. Without this, a brand-new deployment crashes the first
        // time any tenant-scoped handler runs.
        ministr_cloud::run_migrations(&pool)
            .await
            .into_diagnostic()
            .wrap_err("apply cloud postgres migrations")?;
        tracing::info!("cloud postgres migrations applied");

        // F5.3-c-ii — extend the audit_events partition surface
        // forward to `now() + DEFAULT_PARTITION_LOOKAHEAD_QUARTERS`.
        // Migration 0013 seeded 16 partitions covering 2024-Q1 ..
        // 2027-Q4; without this call the pod silently breaks
        // audit_events INSERT once `now()` crosses into Q1 2028.
        // The helper is idempotent + cheap (single pg_inherits
        // query + zero CREATEs on warm starts).
        match ministr_cloud::ensure_audit_partitions(
            &pool,
            ministr_cloud::DEFAULT_PARTITION_LOOKAHEAD_QUARTERS,
        )
        .await
        {
            Ok(outcome) => tracing::info!(
                existing = outcome.existing,
                created = outcome.created,
                target_end_year = outcome.target_end_year,
                target_end_quarter = outcome.target_end_quarter,
                "audit_events partition surface extended"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "ensure_audit_partitions failed — boot continues; future audit INSERTs may fail \
                 if `now()` crosses past the highest seeded partition"
            ),
        }

        Some(Arc::new(pool))
    } else {
        None
    };

    // F1.2 sub-bullet 3 — build the corpus registry once and hand the
    // same `Arc<CorpusRegistry>` to both the MCP server and the daemon
    // REST router below. Both surfaces therefore observe a single
    // source of truth for what's indexed; restore() runs once.
    let corpus_registry = infra::build_corpus_registry(&ctx, config);

    // PHASE3 chunk 1 — wire the durable corpus registry repo before
    // `restore()`. In cloud mode this makes Postgres the source of
    // truth for which corpora exist, so the list survives ACA pod
    // recycling (the on-disk `corpora.json` is pod-ephemeral). In
    // self-hosted serve cloud_pool is None and this is a no-op.
    if let Some(pool) = cloud_pool.as_ref() {
        let repo: Arc<dyn ministr_api::CorporaRepo> = Arc::new(
            ministr_cloud::PostgresCorporaRepo::new(Arc::clone(pool), None),
        );
        corpus_registry.set_corpora_repo(repo);
        tracing::info!("PostgresCorporaRepo wired — corpus registry durable across pod recycle");
    }

    // PHASE3 chunk 5 — wire the on-demand bundle restorer. When a
    // query targets a corpus_id whose in-memory handle is absent but
    // `cloud_corpora` lists it, `CorpusRegistry::get` calls
    // `restorer.download` to lazy-fetch the bundle from blob. Replaces
    // PHASE2's boot-time bulk download.
    if let Some(backend_arc) = blob_backend_arc.as_ref() {
        let restorer: Arc<dyn ministr_api::CorpusRestorer> = Arc::new(
            ministr_cloud::BlobCorpusRestorer::new(Arc::clone(backend_arc)),
        );
        corpus_registry.set_corpus_restorer(restorer);
        tracing::info!("BlobCorpusRestorer wired — first query lazy-downloads bundles from blob");
    }

    corpus_registry.restore().await;

    // F2.x-b — wire the tenant-corpus filter so the new `Backend::Registry`
    // dispatch path gates `corpus_id` lookups by tenant. Self-hosted serve
    // (no cloud Postgres pool) keeps the unfiltered constructor; cross-
    // tenant access there is meaningless because there's only one user.
    //
    // F3.2-iii — the same `PostgresTenantCorpusFilter` struct also
    // implements `TenantCorpusVisibility` (the daemon-side list filter).
    // Build it once as a concrete `Arc` and cast to both trait objects
    // so the MCP gate and the daemon list share a single instance —
    // identical SQL pool, identical visibility semantics.
    let tenant_filter_concrete = cloud_pool.as_ref().map(|pool| {
        std::sync::Arc::new(ministr_cloud::PostgresTenantCorpusFilter::new(
            Arc::clone(pool),
        ))
    });
    let server = if let Some(concrete) = tenant_filter_concrete.as_ref() {
        let filter: std::sync::Arc<dyn ministr_api::TenantCorpusFilter> =
            Arc::clone(concrete) as _;
        tracing::info!(
            "PostgresTenantCorpusFilter wired — /mcp tool calls gated by cloud_corpora.tenant_id"
        );
        server.with_corpus_registry_and_filter(Arc::clone(&corpus_registry), filter)
    } else {
        server.with_corpus_registry(Arc::clone(&corpus_registry))
    };

    // F6.1-g — attach the Postgres-backed agent-session backends so the
    // helpers landed in F6.1-d-c (drops), F6.1-e (snapshot persist), and
    // F6.1-f (lazy restore) actually round-trip through durable storage.
    // Self-hosted serve leaves cloud_pool = None and both backends stay
    // None on the registry; the F6.1-* helpers each collapse to a no-op
    // in that branch.
    let server = if let Some(pool) = cloud_pool.as_ref() {
        let storage: std::sync::Arc<dyn ministr_api::SessionStorage> = std::sync::Arc::new(
            ministr_cloud::PostgresSessionStorage::from_arc(Arc::clone(pool)),
        );
        let ledger: std::sync::Arc<dyn ministr_api::DropsLedger> = std::sync::Arc::new(
            ministr_cloud::PostgresDropsLedger::from_arc(Arc::clone(pool)),
        );
        let server = server.with_session_storage(storage).await;
        let server = server.with_session_drops_ledger(ledger).await;
        tracing::info!(
            "PostgresSessionStorage + PostgresDropsLedger wired — agent sessions persist + restore across pod recycle"
        );
        server
    } else {
        server
    };

    let ingestion_progress = server.ingestion_progress_arc();

    // Extract Arcs before moving server into the factory closure. The HTTP
    // serve path is always local-engine mode, so `service_arc()` must be
    // present — A2A task handlers require direct service access.
    let a2a_service = server
        .service_arc()
        .expect("HTTP serve constructs MinistrServer in local mode");
    let a2a_registry = server.registry_arc();

    // Each HTTP session gets its own MinistrServer fork — shares the
    // Arc'd infrastructure (registry, prefetch, storage) but gets a
    // fresh `active_session_id` (uuid_v4) AND a fresh tenant_id_hint
    // Mutex<Option<String>>. Per F-Test-3b-fix-1-shared-bootstrap:
    // without the fork, all /mcp connections share the bootstrap
    // active_session_id → one SessionEntry → tenant A's first stamp
    // pins the entry to tenant A → tenant B's tool-call activity
    // mutates A's session shadow (data leak on F6.2 export).
    //
    // The prior attempt at this fix hung tool calls because handlers
    // like ministr_survey called `get_session(...).expect("active
    // session exists")` — the fresh fork session id doesn't exist
    // until first written. F-Test-3b-fix-1-shared-bootstrap Phase A
    // replaced those 5 panic sites with `ensure_session_mut` so
    // get-or-create happens automatically; this fork is now safe.
    let server_factory = move || Ok(server.fork_for_new_session());

    let session_manager = Arc::new(LocalSessionManager::default());
    // Override the default loopback-only allowed_hosts list with the
    // deployment's public hostnames when `MINISTR_ALLOWED_HOSTS` is set
    // (comma-separated). Default (no env var) keeps loopback for local dev.
    let mut sh_config = StreamableHttpServerConfig::default();
    if let Ok(hosts_raw) = std::env::var("MINISTR_ALLOWED_HOSTS")
        && !hosts_raw.trim().is_empty()
    {
        sh_config.allowed_hosts = hosts_raw
            .split(',')
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .collect();
        tracing::info!(
            count = sh_config.allowed_hosts.len(),
            "MINISTR_ALLOWED_HOSTS override applied to Streamable HTTP transport"
        );
    }
    let http_service = StreamableHttpService::new(server_factory, session_manager, sh_config);

    // F2.x-b — mount the tenant-scope middleware so tool handlers can
    // recover the resolved Tenant (set by validate_token_middleware
    // upstream) via the `tenant_scope::current` task-local. The layer
    // is added as the INNER layer here so when `protected_router`
    // wraps with `validate_token_middleware` below, the order becomes:
    //   validate_token_middleware → scope_tenant → StreamableHttpService
    // i.e. the tenant is populated into the extension BEFORE the
    // task-local scope is entered. Self-hosted serve still gets the
    // layer mounted (sees no Tenant, scopes None), so the same code
    // path runs in both modes.
    let mcp_router = axum::Router::new()
        .nest_service("/mcp", http_service)
        .layer(axum::middleware::from_fn(
            ministr_mcp::tenant_scope::scope_tenant,
        ));

    // A2A protocol endpoints (agent card + task submission)
    let a2a_state = ministr_mcp::a2a::A2aState {
        service: a2a_service,
        registry: Arc::clone(&a2a_registry),
        tasks: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };
    let a2a_router = ministr_mcp::a2a::a2a_routes(a2a_state);

    // Bundle-serving endpoints (read-only, public).
    let bundle_state = ministr_mcp::bundle_routes::BundleState {
        corpus_dir: ctx.corpus_dir.clone(),
        model_name: resolved_model.to_string(),
        storage: Arc::clone(&ctx.storage),
    };
    let bundle_router = ministr_mcp::bundle_routes::bundle_routes(bundle_state);

    // F6.2-a — session bundle export. Shares the same
    // `Arc<Mutex<SessionRegistry>>` already cloned for the A2A wiring
    // (line above this block), so all surfaces see the same live session
    // shadows. Mounted unconditionally (self-hosted users can also
    // export sessions for debugging); scope-gated as `ministr:read`
    // below alongside the orgs router.
    //
    // F6.2-b — when `cloud_pool` is_some, attach the
    // `PostgresDropsLedger` so the bundle's `drops.jsonl` is
    // populated from the persisted ledger (F6.1-d). Self-hosted serve
    // leaves the ledger `None` and the bundle ships the F6.2-a shape
    // (manifest + delivered only).
    let session_export_state = {
        let mut state =
            ministr_mcp::sessions::SessionExportState::new(Arc::clone(&a2a_registry));
        if let Some(pool) = cloud_pool.as_ref() {
            let ledger: Arc<dyn ministr_api::DropsLedger> = Arc::new(
                ministr_cloud::PostgresDropsLedger::from_arc(Arc::clone(pool)),
            );
            state = state.with_drops_ledger(ledger);
        }
        // F6.2-c — when the bundle-store env is configured (Azure
        // account + signing secret + cloud base URL), attach the
        // `CloudSessionBundleStore` so `handle_export` uploads + returns
        // a signed URL JSON response. Self-hosted serve / missing env
        // leaves `bundle_store = None` and the F6.2-a/b inline-tar
        // shape continues to ship.
        match ministr_cloud::build_session_bundle_store_from_env(
            cloud_env.cloud_base_url.as_deref(),
        ) {
            Ok(Some(store)) => {
                tracing::info!(
                    "session bundle store wired — uploads to blob + returns signed URL"
                );
                let store: Arc<dyn ministr_api::SessionBundleStore> = Arc::new(store);
                state = state.with_bundle_store(store);
            }
            Ok(None) => {
                tracing::debug!(
                    "session bundle store disabled — handle_export streams inline tar"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "session bundle store construction failed — falling back to inline tar"
                );
            }
        }
        state
    };
    // F-Test-3 finding: handle_list / handle_export read
    // `tenant_scope::current()` for cross-tenant 404 + list-scoping
    // (F6.2-e-followup-ii), but the task-local is only populated when
    // `scope_tenant` middleware is layered on the router. Without it,
    // every authenticated tenant sees the bootstrap `ministr-<hash>`
    // session (and any other unstamped legacy entries) — a real
    // cross-tenant leak surfaced by F-Test-3's session-isolation
    // assertions. The daemon routers below already mount this layer;
    // session_export was the gap.
    let session_export_scope_tenant_layer =
        axum::middleware::from_fn(ministr_mcp::tenant_scope::scope_tenant);
    let session_export_router = ministr_mcp::sessions::session_export_routes(session_export_state)
        .layer(session_export_scope_tenant_layer);

    let admin_state = build_admin_state(&cloud_env, corpus_paths.len())?;
    let admin_public = ministr_mcp::admin::admin_public_routes(admin_state.clone());
    let admin_protected = ministr_mcp::admin::admin_protected_routes(admin_state);

    // ── Daemon REST surface (/api/v1/corpora/* + /activity + /coherence-events) ─
    // Share the same `Arc<CorpusRegistry>` already wired into the MCP
    // server (see the build_corpus_registry call above). The daemon's
    // `record_activity` middleware is applied per sub-router so
    // observability spans authenticated calls only (auth check sits
    // outside the activity layer).
    //
    // cloud_pool was constructed above (hoisted for PHASE3 chunk 1).
    // PostgresUsageSink (F1.4 sub-bullet 2) and the billing endpoint
    // (sub-bullet 4) share that same pool.

    let mut daemon_state = ministr_daemon::state::AppState::from_arc(Arc::clone(&corpus_registry));
    // F3.2-iii — wire the visibility filter into the daemon-side
    // `GET /api/v1/corpora` handler. Same concrete instance as the
    // MCP gate (constructed above); the cast to `dyn
    // TenantCorpusVisibility` exposes the list-side method.
    if let Some(concrete) = tenant_filter_concrete.as_ref() {
        let visibility: std::sync::Arc<dyn ministr_api::TenantCorpusVisibility> =
            Arc::clone(concrete) as _;
        daemon_state = daemon_state.with_corpus_visibility(visibility);
        tracing::info!(
            "PostgresTenantCorpusFilter visibility wired — GET /api/v1/corpora filtered by tenant + ACL"
        );
    }
    if let Some(pool) = cloud_pool.as_ref() {
        let sink: std::sync::Arc<dyn ministr_api::UsageSink> =
            std::sync::Arc::new(ministr_cloud::PostgresUsageSink::from_arc(Arc::clone(pool)));
        daemon_state = daemon_state.with_usage_sink(sink);
        tracing::info!("PostgresUsageSink wired — billable usage events enabled");

        // F3.7b — wire the audit sink so the daemon's corpus-mutation
        // handlers (register/clone/unregister) emit audit_events rows.
        // Same Arc<dyn AuditSink> instance is shared with the orgs +
        // api_keys + GitHub-callback paths below, so every cloud-side
        // audit lands in the same logical pipeline.
        let audit_sink: std::sync::Arc<dyn ministr_api::AuditSink> = std::sync::Arc::new(
            ministr_cloud::PostgresAuditSink::from_arc(Arc::clone(pool)),
        );
        daemon_state = daemon_state.with_audit_sink(Arc::clone(&audit_sink));
        tracing::info!(
            "PostgresAuditSink wired into daemon — corpus.created/cloned/deleted emit audit_events"
        );

        // PHASE3 chunk 4 — route POST /api/v1/corpora and the clone
        // route through the cloud index-job queue instead of running
        // ingestion inline. The serve pod's in-process WorkerLoop
        // (PHASE6 chunk 2) drains the queue.
        let sink = ministr_cloud::PostgresIndexJobSink::new(Arc::clone(pool), None);
        let index_job_sink: std::sync::Arc<dyn ministr_api::IndexJobSink> =
            std::sync::Arc::new(sink);
        daemon_state = daemon_state.with_index_job_sink(index_job_sink);
        tracing::info!(
            "PostgresIndexJobSink wired — serve pod enqueues; WorkerLoop drains in-process"
        );

        // PHASE6 chunk 2 — in-process WorkerLoop. The serve pod now
        // drains the same `indexer_jobs` table the ACA Job currently
        // does (which will be deleted in chunk 3). Both compete via
        // `FOR UPDATE SKIP LOCKED` so this is safe to run alongside
        // the legacy Job path during the chunk-2-to-chunk-3 transition.
        //
        // The loop is spawned with an always-live cancel token; SIGTERM
        // handling lands in chunk 4 along with the operator-side deploy.
        match ministr_mcp::admin::jobs::PostgresJobQueue::open(
            cloud_env
                .pg_url
                .as_deref()
                .expect("cloud_pool is_some implies pg_url is_some"),
        )
        .await
        {
            Ok(pg_queue) => {
                let queue_backend = std::sync::Arc::new(
                    ministr_mcp::admin::jobs::JobQueueBackend::Postgres(pg_queue),
                );
                let runner = std::sync::Arc::new(crate::worker::IngestionRunner {
                    config: std::sync::Arc::new(config.clone()),
                    resolved_model: std::sync::Arc::from(resolved_model),
                    resolved_dimension,
                    rerank_depth,
                    blob_backend: blob_backend_arc
                        .as_ref()
                        .map(std::sync::Arc::clone),
                    queue: std::sync::Arc::clone(&queue_backend),
                });
                let runner_dyn: std::sync::Arc<dyn crate::worker::JobRunner> = runner;
                let worker_loop = crate::worker::WorkerLoop::new(
                    queue_backend,
                    runner_dyn,
                    tokio_util::sync::CancellationToken::new(),
                );
                tokio::spawn(worker_loop.run());
                tracing::info!(
                    "PHASE6 WorkerLoop spawned — serve pod drains indexer_jobs in-process"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "WorkerLoop disabled — PostgresJobQueue::open failed",
                );
            }
        }
    }

    // F2.1 — GitHub App installation-token minter for private-repo
    // cloning. Built independently of the GitHub OAuth IdP (F1.3) so a
    // deployment can enable App-driven clones without also enabling the
    // user-facing GitHub sign-in flow (or vice versa).
    if let (Some(app_id), Some(pem)) = (
        cloud_env.github_app_id.as_ref(),
        cloud_env.github_app_private_key.as_ref(),
    ) {
        match ministr_cloud::GitHubAppClient::new(app_id.clone(), pem) {
            Ok(client) => {
                let minter: std::sync::Arc<dyn ministr_api::InstallationTokenMinter> =
                    std::sync::Arc::new(client);
                daemon_state = daemon_state.with_installation_minter(minter);
                tracing::info!(
                    app_id = %app_id,
                    "GitHubAppClient wired — private-repo cloning via installation tokens enabled"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "GitHub App disabled — MINISTR_GITHUB_APP_ID/PRIVATE_KEY rejected"
                );
            }
        }
    } else if cloud_env.github_app_id.is_some() || cloud_env.github_app_private_key.is_some() {
        tracing::warn!(
            has_app_id = cloud_env.github_app_id.is_some(),
            has_private_key = cloud_env.github_app_private_key.is_some(),
            "GitHub App NOT wired — both MINISTR_GITHUB_APP_ID and MINISTR_GITHUB_APP_PRIVATE_KEY must be set"
        );
    }

    // PHASE2 chunk 4 — durable corpus uploads.
    //
    // Build the BlobBackendSink, install a completion channel on the
    // registry, and spawn the reactor that drains `(corpus_id,
    // corpus_dir)` events and fires `enqueue_upload`. The reactor is
    // serial (one upload at a time) so concurrent ingestion + upload
    // don't compete for fs I/O during the bundle's tar+zstd pass.
    //
    // Wiring runs AFTER `restore()` above: if the cloud cold-started
    // and `restore()` re-registered any corpora, those re-registrations
    // do NOT fire an upload — we just downloaded the bundle from blob,
    // it is already the source of truth.
    if let Some(backend_arc) = blob_backend_arc.as_ref() {
        let sink: Arc<dyn ministr_api::BlobSink> = Arc::new(
            ministr_cloud::BlobBackendSink::new(
                Arc::clone(backend_arc),
                resolved_model.to_string(),
            ),
        );
        let (tx, mut completion_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, std::path::PathBuf)>();
        corpus_registry.set_completion_sink(tx);
        daemon_state = daemon_state.with_blob_sink(Arc::clone(&sink));

        tokio::spawn(async move {
            while let Some((corpus_id, corpus_dir)) = completion_rx.recv().await {
                sink.enqueue_upload(corpus_id, corpus_dir);
            }
            tracing::info!("blob upload reactor exited — completion channel closed");
        });
        tracing::info!(
            "blob durability wired — bundles downloaded at boot, uploaded after every ingest"
        );
    }

    let activity_layer = axum::middleware::from_fn_with_state(
        daemon_state.clone(),
        ministr_daemon::activity::record,
    );
    // F2.x-d — mount scope_tenant on the daemon routers too. The
    // validate_scope_middleware wrapper added below by
    // scope_protected_router populates `Tenant` in the request
    // extensions; scope_tenant (one layer in from there) lifts that
    // into the `tenant_scope::current()` task-local for the duration
    // of the handler. The cloud-side upserts in `PostgresCorporaRepo`
    // and `PostgresIndexJobSink` consult that task-local to stamp
    // `cloud_corpora.tenant_id` — closing the F2.x-b permissive arm.
    let scope_tenant_layer =
        axum::middleware::from_fn(ministr_mcp::tenant_scope::scope_tenant);
    let daemon_read_router = ministr_daemon::daemon::corpora_read_router(daemon_state.clone())
        .layer(activity_layer.clone())
        .layer(scope_tenant_layer.clone());
    let daemon_write_router = ministr_daemon::daemon::corpora_write_router(daemon_state.clone())
        .layer(activity_layer.clone())
        .layer(scope_tenant_layer.clone());
    let daemon_bundle_router = ministr_daemon::daemon::corpora_bundle_router(daemon_state.clone())
        .layer(activity_layer.clone())
        .layer(scope_tenant_layer.clone());
    let daemon_obs_router = ministr_daemon::daemon::observability_router(daemon_state)
        .layer(activity_layer)
        .layer(scope_tenant_layer);
    // Note: `corpora_ask_router` is intentionally NOT mounted on cloud.
    // The container has no `claude` CLI; clients hitting /ask get 404.

    let app = if let Some(oauth_cfg) = oauth_config {
        tracing::info!(
            persistent = cloud_env.data_dir.is_some(),
            postgres = cloud_env.pg_url.is_some(),
            webhook = cloud_env.webhook_secret.is_some(),
            "OAuth 2.1 authentication enabled"
        );
        let store = build_oauth_store(
            oauth_cfg,
            cloud_env.data_dir.as_deref(),
            cloud_env.pg_url.as_deref(),
        )
        .await?;
        // F3.4a — wire the API-key resolver so the existing OAuth
        // token-validation middleware also authenticates `mst_pk_…`
        // service-account tokens. Cloud-only: the resolver needs the
        // Postgres pool to consult `api_keys`. Self-hosted serve
        // leaves the store untouched and only OAuth tokens
        // authenticate there.
        let store = if let Some(pool) = cloud_pool.as_ref() {
            let resolver = ministr_cloud::PostgresApiKeyResolver::new((**pool).clone()).into_dyn();
            tracing::info!(
                "PostgresApiKeyResolver wired — `mst_pk_…` tokens authenticate via api_keys table"
            );
            store.with_api_key_resolver(resolver)
        } else {
            store
        };
        let protected = ministr_mcp::auth::protected_router(mcp_router, store.clone());
        let protected_bundles = ministr_mcp::auth::scope_protected_router(
            bundle_router,
            store.clone(),
            "ministr:bundle:read",
        );
        let protected_admin = ministr_mcp::auth::scope_protected_router(
            admin_protected,
            store.clone(),
            "ministr:write",
        );
        let daemon_read_p = ministr_mcp::auth::scope_protected_router(
            daemon_read_router,
            store.clone(),
            "ministr:read",
        );
        // F2.3 — quota enforcement state. Probe selection: when
        // cloud_pool is Some, use PostgresCorporaProbe so the count is
        // tenant-scoped against cloud_corpora rows (the source of
        // truth in cloud mode — register_corpus writes here via
        // IndexJobSink before the worker indexes anything, so the
        // in-memory CorpusRegistry trails). Self-hosted serve uses
        // RegistryProbe over the in-memory registry, which is correct
        // because every corpus IS in-memory there. Surfaced by
        // F-Test-1's cloud-registry gap finding + lit up by F-Test-5.
        let probe: std::sync::Arc<dyn ministr_cloud::UsageProbe> =
            if let Some(pool) = cloud_pool.as_ref() {
                tracing::info!(
                    "PostgresCorporaProbe wired — quota counts cloud_corpora per-tenant"
                );
                std::sync::Arc::new(ministr_cloud::PostgresCorporaProbe::new(Arc::clone(pool)))
            } else {
                std::sync::Arc::new(ministr_cloud::RegistryProbe::new(Arc::clone(
                    &corpus_registry,
                )))
            };
        // Rules are ordered cheapest-first (CorpusCountRule's match
        // predicate is a string compare). Mounted as a single Tower
        // layer beneath the scope guards — see the daemon_write_q
        // binding below.
        let quota_state = ministr_cloud::QuotaState::new(
            vec![
                std::sync::Arc::new(ministr_cloud::CorpusCountRule),
                std::sync::Arc::new(ministr_cloud::AtlasAccessRule),
            ],
            probe,
        );

        // F2.2 — rate-limit write/clone routes on cloud only. Two
        // layers stack: per-IP first (rejects pre-auth abuse before
        // touching the bucket store with a tenant key), then
        // per-tenant (rejects leaked-key bursts from authenticated
        // callers). Self-hosted serve mounts neither — the
        // open-core stack stays untouched.
        let daemon_write_rl = if cloud_env.pg_url.is_some() {
            let ip_bucket = std::sync::Arc::new(ministr_cloud::InMemoryBucket::new(
                /* capacity */ 20.0,
                /* refill_per_sec */ 0.5,
            ));
            let tenant_bucket = std::sync::Arc::new(ministr_cloud::InMemoryBucket::new(
                /* capacity */ 60.0,
                /* refill_per_sec */ 1.0,
            ));
            let ip_cfg = std::sync::Arc::new(ministr_cloud::RateLimitConfig::new(
                ip_bucket,
                ministr_cloud::ip_key::<axum::body::Body>,
                "per-ip",
            ));
            let tenant_cfg = std::sync::Arc::new(ministr_cloud::RateLimitConfig::new(
                tenant_bucket,
                ministr_cloud::tenant_key::<axum::body::Body>,
                "per-tenant",
            ));
            daemon_write_router
                .layer(axum::middleware::from_fn_with_state(
                    tenant_cfg,
                    ministr_cloud::rate_limit_middleware,
                ))
                .layer(axum::middleware::from_fn_with_state(
                    ip_cfg,
                    ministr_cloud::rate_limit_middleware,
                ))
        } else {
            daemon_write_router
        };
        // F2.3 — quota check sits BETWEEN the scope guard (auth) and
        // the rate limit (anti-abuse). Order matters: the request needs
        // a populated `Tenant` extension (from scope_protected_router)
        // before the quota middleware can read it, and quota rejection
        // (402) should preempt rate-limit accounting (429) so an
        // already-over-cap tenant doesn't also burn rate-limit
        // tokens on the rejection.
        let daemon_write_q = if cloud_env.pg_url.is_some() {
            daemon_write_rl.layer(axum::middleware::from_fn_with_state(
                quota_state.clone(),
                ministr_cloud::quota_middleware,
            ))
        } else {
            daemon_write_rl
        };
        let daemon_write_p = ministr_mcp::auth::scope_protected_router(
            daemon_write_q,
            store.clone(),
            "ministr:write",
        );
        let daemon_bundle_p = ministr_mcp::auth::scope_protected_router(
            daemon_bundle_router,
            store.clone(),
            "ministr:bundle:write",
        );
        let daemon_obs_p = ministr_mcp::auth::scope_protected_router(
            daemon_obs_router,
            store.clone(),
            "ministr:write",
        );
        // F1.4 sub-bullet 4 — billing endpoint. Mounted only when
        // a cloud Postgres pool exists; otherwise the route is absent
        // and clients see 404, matching the absence of any billable
        // surface on self-hosted serve.
        // F6.2-a — session bundle export route, scope-gated as
        // `ministr:read`. Mounted unconditionally so self-hosted users
        // can export sessions too; the cloud's scope guard is the
        // standard `validate_token_middleware` path.
        let session_export_p = ministr_mcp::auth::scope_protected_router(
            session_export_router,
            store.clone(),
            "ministr:read",
        );
        let mut composed = a2a_router
            .merge(protected)
            .merge(protected_bundles)
            .merge(protected_admin)
            .merge(admin_public)
            .merge(daemon_read_p)
            .merge(daemon_write_p)
            .merge(daemon_bundle_p)
            .merge(daemon_obs_p)
            .merge(session_export_p);
        if let Some(pool) = cloud_pool.as_ref() {
            let billing_router = ministr_cloud::billing_routes(
                ministr_cloud::BillingState::from_arc(Arc::clone(pool)),
            );
            let billing_protected = ministr_mcp::auth::scope_protected_router(
                billing_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(billing_protected);
            tracing::info!("billing endpoint mounted — GET /api/v1/billing/usage");

            // F3.1a/b — orgs CRUD + member listing + magic-link invites.
            // Mounted only when the cloud Postgres pool exists (self-
            // hosted serve has no multi-tenant orgs surface). Behind
            // `ministr:read` — creating an org or minting an invite is
            // a self-service tenant action, not a corpus mutation, so
            // the `:write` scope's rate-limit + quota layers are
            // intentionally not in this path. F3.1c (Stripe seat sync)
            // will extend the same router.
            //
            // `with_cloud_base_url` provides the absolute origin the
            // invite handler stitches into the returned `invite_url`.
            // `cloud_env.cloud_base_url` is the same value the GitHub
            // sign-in handler uses for its OAuth redirect_uri, so the
            // invite URL is on the same origin as `/auth/github/start`
            // and the recipient's loopback callback works end-to-end.
            //
            // F3.1c-i — build the outbound Stripe client here (hoisted
            // from below) so the orgs router can use it to mint an
            // org-owned Customer at org-creation. F1.5's GitHub sign-in
            // hook and F2.4's Checkout routes both still reach the
            // same `stripe_client` lower in this function.
            let stripe_client = cloud_env.stripe_secret_key.as_ref().and_then(|key| {
                match ministr_cloud::StripeClient::new(key.clone()) {
                    Ok(c) => {
                        tracing::info!(
                            "stripe outbound client built — Customer creation + Meters API enabled"
                        );
                        Some(Arc::new(c))
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "stripe client disabled — STRIPE_SECRET_KEY rejected");
                        None
                    }
                }
            });

            // F3.5a — outbound webhook dispatcher. Built once and
            // shared between the webhook routes router (CRUD + /test
            // endpoint) AND the F3.5b-i WebhookFanoutSink below so
            // both call paths reuse the same TLS connection pool.
            let webhook_dispatcher = match ministr_cloud::WebhookDispatcher::new() {
                Ok(d) => Some(Arc::new(d)),
                Err(e) => {
                    tracing::warn!(error = %e, "webhook dispatcher init failed; webhooks + fan-out disabled");
                    None
                }
            };

            // F3.7a + F3.5b-i + F5.3-d-i — build the audit sink
            // chain. The Postgres sink always lands first so the
            // audit_events row is durable BEFORE any outbound
            // dispatch. The webhook fan-out sink joins when the
            // dispatcher initialised successfully; the SIEM (Splunk
            // HEC) sink joins when MINISTR_SIEM_HEC_URL +
            // MINISTR_SIEM_HEC_TOKEN are both set. All three
            // sinks are fire-and-forget so chaining doesn't
            // serialise their I/O.
            let postgres_audit: Arc<dyn ministr_api::AuditSink> = Arc::new(
                ministr_cloud::PostgresAuditSink::from_arc(Arc::clone(pool)),
            );
            let mut audit_sinks: Vec<Arc<dyn ministr_api::AuditSink>> =
                vec![Arc::clone(&postgres_audit)];
            let mut chain_desc = String::from("PostgresAuditSink");
            if let Some(d) = webhook_dispatcher.as_ref() {
                let fanout = ministr_cloud::WebhookFanoutSink::new(
                    Arc::clone(pool),
                    Arc::clone(d),
                );
                audit_sinks.push(Arc::new(fanout));
                chain_desc.push_str(" → WebhookFanoutSink");
            }
            if let Some(hec) = ministr_cloud::SplunkHecSink::from_env() {
                audit_sinks.push(Arc::new(hec));
                chain_desc.push_str(" → SplunkHecSink");
            }
            // F5.3-d-ii-dispatch — per-org SIEM dispatcher. Looks up
            // `org_siem_configs` on every audit event with org_id =
            // Some, fires alongside the global env-var sink so events
            // hit BOTH the operator's central SIEM AND the customer's
            // configured endpoint.
            {
                let per_org = ministr_cloud::PerOrgSiemDispatcher::new(
                    Arc::clone(pool),
                );
                audit_sinks.push(Arc::new(per_org));
                chain_desc.push_str(" → PerOrgSiemDispatcher");
            }
            let audit_sink: Arc<dyn ministr_api::AuditSink> = if audit_sinks.len() == 1 {
                tracing::info!(chain = %chain_desc, "audit pipeline wired");
                postgres_audit
            } else {
                tracing::info!(chain = %chain_desc, "audit pipeline wired");
                Arc::new(ministr_cloud::ChainedAuditSink::new(audit_sinks))
            };

            let mut orgs_state =
                ministr_cloud::OrgsState::from_arc(Arc::clone(pool));
            if let Some(base) = cloud_env.cloud_base_url.as_deref() {
                orgs_state = orgs_state.with_cloud_base_url(base);
            }
            if let Some(stripe) = stripe_client.as_ref() {
                orgs_state = orgs_state.with_stripe(Arc::clone(stripe));
            }
            orgs_state = orgs_state.with_audit(Arc::clone(&audit_sink));
            // F3.1b-ii-a — wire the default LogOnlyMailSender. A
            // concrete provider (Resend / SES) lands in F3.1b-ii-b
            // and replaces this `Arc` once the operator picks one.
            let mailer: Arc<dyn ministr_api::MailSender> =
                Arc::new(ministr_cloud::LogOnlyMailSender::new());
            orgs_state = orgs_state.with_mailer(Arc::clone(&mailer));
            tracing::info!(
                "mail sender wired: LogOnlyMailSender (no provider configured — invite URLs are in the HTTP response body)"
            );
            let orgs_router = ministr_cloud::orgs_routes(orgs_state);
            let orgs_protected = ministr_mcp::auth::scope_protected_router(
                orgs_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(orgs_protected);
            tracing::info!(
                "orgs endpoints mounted — POST /api/v1/orgs, GET /api/v1/orgs, GET /api/v1/orgs/{{id}}/members, POST /api/v1/orgs/{{id}}/invites"
            );

            // F3.3a — per-org usage dashboard endpoint. Aggregates
            // `usage_rollups` across `org_members`. Owner/admin only.
            // Mounted behind `ministr:read` alongside the orgs router.
            let org_usage_router = ministr_cloud::org_usage_routes(
                ministr_cloud::OrgUsageState::from_arc(Arc::clone(pool)),
            );
            let org_usage_protected = ministr_mcp::auth::scope_protected_router(
                org_usage_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(org_usage_protected);
            tracing::info!(
                "org usage endpoint mounted — GET /api/v1/orgs/{{id}}/usage"
            );

            // F3.4a — service-account API keys (mint, list, revoke).
            // Cloud-only: backed by the `api_keys` table. Mounted behind
            // `ministr:read` because every action targets the caller's
            // own keys (the WHERE clauses join on the calling tenant's
            // user_id). Create + revoke are write operations but they
            // do not touch corpus data, so the `:write` scope's quota
            // + rate-limit layers are intentionally bypassed — the
            // same posture as the orgs router.
            let api_keys_router = ministr_cloud::api_keys_routes(
                ministr_cloud::ApiKeysState::new((**pool).clone())
                    .with_audit(Arc::clone(&audit_sink)),
            );
            let api_keys_protected = ministr_mcp::auth::scope_protected_router(
                api_keys_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(api_keys_protected);
            tracing::info!(
                "api_keys endpoints mounted — POST /api/v1/api_keys, GET /api/v1/api_keys, DELETE /api/v1/api_keys/{{id}}"
            );

            // F3.7a — audit list endpoint. Owner / admin only;
            // members get 403. Mounted behind `ministr:read` so any
            // authenticated org member's token can call it; the
            // role check inside is the actual gate.
            let audit_router = ministr_cloud::audit_routes(
                ministr_cloud::AuditState::from_arc(Arc::clone(pool)),
            );
            let audit_protected = ministr_mcp::auth::scope_protected_router(
                audit_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(audit_protected);
            tracing::info!(
                "audit endpoint mounted — GET /api/v1/orgs/{{id}}/audit"
            );

            // F3.5a — outbound webhook subscriptions. Cloud-only;
            // owner/admin-only authz inside the handlers. Re-uses
            // the `webhook_dispatcher` constructed above so the CRUD
            // routes share TLS pool with the F3.5b-i fan-out path.
            if let Some(dispatcher) = webhook_dispatcher.as_ref() {
                let webhooks_state = ministr_cloud::WebhooksState::new(
                    Arc::clone(pool),
                    Arc::clone(dispatcher),
                );
                let webhooks_router = ministr_cloud::webhooks_routes(webhooks_state);
                let webhooks_protected = ministr_mcp::auth::scope_protected_router(
                    webhooks_router,
                    store.clone(),
                    "ministr:read",
                );
                composed = composed.merge(webhooks_protected);
                tracing::info!(
                    "webhook endpoints mounted — POST /api/v1/orgs/{{id}}/webhooks, GET/DELETE, POST /test"
                );
            }

            // F5.1-d — per-org SAML config CRUD. Owner-only ACL
            // enforced inside each handler via assert_owner_or_admin;
            // the scope_protected_router wrapper supplies the Tenant
            // extension that the ACL reads.
            {
                let saml_config_state = ministr_cloud::SamlState::new(Arc::clone(pool));
                let saml_config_router = ministr_cloud::saml_config_routes(saml_config_state);
                let saml_config_protected = ministr_mcp::auth::scope_protected_router(
                    saml_config_router,
                    store.clone(),
                    "ministr:read",
                );
                composed = composed.merge(saml_config_protected);
                tracing::info!(
                    "saml config CRUD mounted — POST/GET/DELETE /api/v1/orgs/{{id}}/saml/config"
                );
            }

            // F5.2-d — per-org OIDC config CRUD. Same shape as the
            // F5.1-d SAML block: owner-only ACL inside each handler,
            // scope_protected_router supplies the Tenant extension.
            // Uses a fresh OidcState (no need to share with the
            // /oidc/login + /oidc/callback wiring above — these are
            // pool-only handlers).
            {
                let oidc_config_state =
                    ministr_cloud::OidcState::new(Arc::clone(pool));
                let oidc_config_router = ministr_cloud::oidc_config_routes(oidc_config_state);
                let oidc_config_protected = ministr_mcp::auth::scope_protected_router(
                    oidc_config_router,
                    store.clone(),
                    "ministr:read",
                );
                composed = composed.merge(oidc_config_protected);
                tracing::info!(
                    "oidc config CRUD mounted — POST/GET/DELETE /api/v1/orgs/{{id}}/oidc/config"
                );
            }

            // F5.3-d-ii-config — per-org SIEM config CRUD. Owner-only
            // ACL inside each handler; scope_protected_router supplies
            // the Tenant extension. Lookup state for dispatch will
            // land in F5.3-d-ii-dispatch; this CRUD just persists.
            {
                let siem_config_state =
                    ministr_cloud::SiemConfigState::from_arc(Arc::clone(pool));
                let siem_config_router =
                    ministr_cloud::siem_config_routes(siem_config_state);
                let siem_config_protected = ministr_mcp::auth::scope_protected_router(
                    siem_config_router,
                    store.clone(),
                    "ministr:read",
                );
                composed = composed.merge(siem_config_protected);
                tracing::info!(
                    "siem config CRUD mounted — POST/GET/DELETE /api/v1/orgs/{{id}}/siem/config"
                );
            }

            // F2.6 — Atlas v0 pilot. Manifest + per-slug query stubs.
            // Mounted behind `ministr:read` so any paid-tier token
            // admits; the F2.3 `AtlasAccessRule` runs higher up in the
            // composed stack and short-circuits unauthenticated /
            // free callers with the 402 paywall. Cloud-only —
            // self-hosted serve leaves Atlas unmounted.
            let atlas_router =
                ministr_atlas::atlas_routes(ministr_atlas::AtlasState::from_seed_list());
            let atlas_protected = ministr_mcp::auth::scope_protected_router(
                atlas_router,
                store.clone(),
                "ministr:read",
            );
            composed = composed.merge(atlas_protected);
            tracing::info!(
                count = ministr_atlas::ATLAS_SEED_REPOS.len(),
                "atlas v0 mounted — GET /atlas/manifest.json + /atlas/{{slug}}/*"
            );
            // F1.5 sub-bullet 3 — Stripe webhook receiver. Mounted
            // when both the cloud pool AND the Stripe signing secret
            // are present. Public route (Stripe is the caller); the
            // signature check is the only auth.
            if let Some(stripe_secret) = cloud_env.stripe_webhook_secret.as_ref() {
                let stripe_router = ministr_cloud::billing::stripe::stripe_webhook_routes(
                    ministr_cloud::StripeWebhookState::new(
                        Arc::clone(pool),
                        stripe_secret.clone(),
                    ),
                );
                composed = composed.merge(stripe_router);
                tracing::info!("stripe webhook mounted — POST /webhooks/stripe");
            }
            // F5.1-b — SAML SP endpoints. Public routes (IdP doesn't
            // carry bearer tokens); per-org config gates whether
            // a given org has SAML SSO enabled. F5.1-c will add the
            // signed-assertion-receiving POST /orgs/{id}/saml/acs.
            {
                let saml_router = ministr_cloud::saml_routes(
                    ministr_cloud::SamlState::new(Arc::clone(pool)),
                );
                composed = composed.merge(saml_router);
                tracing::info!(
                    "saml SP routes mounted — GET /orgs/{{id}}/saml/metadata.xml + /login"
                );
            }
            // F5.2-b/c — OIDC RP login + callback endpoints. Public
            // routes (browser-initiated; IdP doesn't carry bearer
            // tokens). The callback handler is wired to the same
            // `OAuthStore` the GitHub flow uses (bearer tokens
            // indistinguishable downstream), the cloud base URL (so
            // the `redirect_uri` the IdP sees matches what's
            // registered for the Relying Party), and the audit sink
            // (so `oidc.login` events flow through the same pipeline
            // as `member.added` / `share.granted`).
            {
                let mut oidc_state = ministr_cloud::OidcState::new(Arc::clone(pool))
                    .with_oauth_store(store.clone())
                    .with_audit(Arc::clone(&audit_sink));
                if let Some(base_url) = cloud_env.cloud_base_url.as_ref() {
                    oidc_state = oidc_state.with_cloud_base_url(base_url.clone());
                }
                let oidc_router = ministr_cloud::oidc_routes(oidc_state);
                composed = composed.merge(oidc_router);
                tracing::info!(
                    cloud_base_url_set = cloud_env.cloud_base_url.is_some(),
                    "oidc RP routes mounted — GET /orgs/{{id}}/oidc/{{login,callback}}"
                );
            }
            // F1.3 sub-bullet — GitHub sign-in flow. Mounted when the
            // cloud Postgres pool, the GitHub OAuth App credentials,
            // and a public base URL are ALL present. Public routes
            // (sign-in must be reachable without an existing token); the
            // CSRF + loopback-allowlist check inside the handlers is
            // the only gate.
            // F1.5 — outbound Stripe client was hoisted above the orgs
            // wiring (F3.1c-i needs it at org-creation). The same
            // `stripe_client` is reused here for F2.4 (Checkout +
            // portal) and the F1.5 GitHub-callback hook.

            // F2.4 — Stripe Checkout + Customer Portal routes.
            // Requires the outbound stripe client AND the cloud base
            // URL (used to build success/return URLs Stripe redirects
            // back to). Mounted behind `ministr:read` — the calling
            // tenant authorises against its own Stripe Customer.
            if let (Some(stripe), Some(base_url)) =
                (stripe_client.as_ref(), cloud_env.cloud_base_url.as_ref())
            {
                let catalog: std::sync::Arc<dyn ministr_cloud::PriceCatalog> =
                    std::sync::Arc::new(ministr_cloud::EnvPriceCatalog::new(
                        cloud_env.stripe_price_pro.clone(),
                        cloud_env.stripe_price_team.clone(),
                    ));
                let checkout_state = ministr_cloud::CheckoutState::new(
                    Arc::clone(stripe),
                    Arc::clone(pool),
                    catalog,
                    base_url.clone(),
                );
                let checkout_router = ministr_cloud::checkout_routes(checkout_state);
                let checkout_protected = ministr_mcp::auth::scope_protected_router(
                    checkout_router,
                    store.clone(),
                    "ministr:read",
                );
                composed = composed.merge(checkout_protected);
                tracing::info!(
                    has_pro_price = cloud_env.stripe_price_pro.is_some(),
                    has_team_price = cloud_env.stripe_price_team.is_some(),
                    "stripe checkout + portal mounted — POST /api/v1/billing/{{checkout,portal}}"
                );
            }

            if let (Some(cid), Some(secret), Some(base_url)) = (
                cloud_env.github_client_id.as_ref(),
                cloud_env.github_client_secret.as_ref(),
                cloud_env.cloud_base_url.as_ref(),
            ) {
                match ministr_cloud::GitHubIdp::new(cid.clone(), secret.clone()) {
                    Ok(idp) => {
                        let mut state = ministr_cloud::GitHubSigninState::new(
                            Arc::new(idp),
                            (**pool).clone(),
                            store,
                            base_url.clone(),
                        );
                        if let Some(stripe) = stripe_client.as_ref() {
                            state = state.with_stripe(Arc::clone(stripe));
                        }
                        // F3.7b — wire the audit sink so invite
                        // acceptance fires a `member.added` row.
                        state = state.with_audit(Arc::clone(&audit_sink));
                        composed = composed.merge(ministr_cloud::github_signin_routes(state));
                        tracing::info!(
                            base_url = %base_url,
                            stripe_customer_seed = stripe_client.is_some(),
                            "github sign-in mounted — GET /auth/github/start, /auth/github/callback"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "github sign-in disabled — invalid credentials");
                    }
                }
            } else if cloud_env.github_client_id.is_some()
                || cloud_env.github_client_secret.is_some()
                || cloud_env.cloud_base_url.is_some()
            {
                tracing::warn!(
                    has_client_id = cloud_env.github_client_id.is_some(),
                    has_client_secret = cloud_env.github_client_secret.is_some(),
                    has_base_url = cloud_env.cloud_base_url.is_some(),
                    "github sign-in NOT mounted — MINISTR_GITHUB_CLIENT_ID, MINISTR_GITHUB_CLIENT_SECRET, and MINISTR_CLOUD_BASE_URL must ALL be set"
                );
            }
        }
        composed
    } else {
        // No OAuth: daemon + admin protected routes mount but anyone on the
        // network can hit them. Only safe for local dev — cloud deployments
        // must always set an `OAuthConfig`.
        a2a_router
            .merge(mcp_router)
            .merge(bundle_router)
            .merge(admin_protected)
            .merge(admin_public)
            .merge(daemon_read_router)
            .merge(daemon_write_router)
            .merge(daemon_bundle_router)
            .merge(daemon_obs_router)
            .merge(session_export_router)
    };

    // F3.6-b-ii-a — opt-in CORS layer. Mounted last so the headers
    // wrap the final composed router (including OAuth-protected and
    // public surfaces). Default-off: missing/blank
    // `MINISTR_CORS_ALLOWED_ORIGINS` leaves the layer unmounted and
    // self-hosted serve keeps its zero-browser-exposure posture.
    let app = if let Some(origins) = parse_cors_allowed_origins(cloud_env.cors_allowed_origins.as_deref()) {
        tracing::info!(
            origins = ?origins,
            "CORS enabled — Access-Control-Allow-Origin headers added for browser callers"
        );
        app.layer(build_cors_layer(&origins))
    } else {
        app
    };

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to bind HTTP server to {bind_addr}"))?;

    tracing::info!(address = %bind_addr, "ministr HTTP server listening");

    // Ingest in background AFTER the HTTP server is bound.
    infra::spawn_background_ingestion(corpus_paths, git_includes, &ctx, &ingestion_progress);

    // Watch .ministr.toml for path changes and re-index automatically.
    let _config_watcher_handle = repo_config_dir.and_then(|dir| {
        ingestion::spawn_config_watcher(
            dir.to_path_buf(),
            corpus_paths.to_vec(),
            &ctx,
            &ingestion_progress,
        )
    });

    axum::serve(listener, app)
        .await
        .into_diagnostic()
        .wrap_err("HTTP server exited with error")?;

    tracing::info!("ministr shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr serve --proxy
// ---------------------------------------------------------------------------

/// `ministr serve --proxy` — thin MCP proxy over stdin/stdout.
///
/// Connects to the ministr daemon at `~/.ministr/ministrd.sock` and routes
/// every shared MCP tool call through it via the [`QueryBackend`] trait
/// abstraction. No ONNX model, no indexes, no `SQLite` in this process —
/// just HTTP over UDS.
///
/// **Linked projects:** the per-call `project` argument routing the
/// previous `ProxyServer` supported is not currently re-implemented on the
/// unified `MinistrServer` — that requires extending the backend trait
/// with multi-corpus dispatch. For now, linked projects in
/// `.ministr.toml` are accepted but silently ignored; only the primary
/// corpus is queryable through this path. Users who need multi-corpus
/// queries can use the desktop app (which connects to the daemon
/// directly) until this gap closes.
#[allow(
    clippy::too_many_lines,
    reason = "orchestration entry point — sequential setup (register \
              primary corpus, create session, resolve linked projects, \
              build the backend, install signal handlers, run the MCP \
              loop, run cleanup); each step is unique and inlining keeps \
              the startup order auditable"
)]
pub(crate) async fn cmd_serve_proxy_stdio(
    corpus_paths: &[String],
    linked: &[ministr_core::config::ResolvedLinkedProject],
) -> Result<()> {
    eprintln!(
        "ministr: proxy starting with {} corpus paths, {} linked project(s)",
        corpus_paths.len(),
        linked.len()
    );

    // Resolve the primary corpus + session.
    let client = std::sync::Arc::new(ministr_api::client::DaemonClient::new());
    let corpus_id = match client.register_corpus(corpus_paths).await {
        Ok(resp) => {
            eprintln!(
                "ministr: primary corpus {} registered (indexing_started={})",
                resp.corpus_id, resp.indexing_started
            );
            resp.corpus_id
        }
        Err(e) => {
            return Err(miette::miette!(
                "corpus registration failed: {e} — is the daemon running?"
            ));
        }
    };
    let session_id = match client.create_session(&corpus_id, None).await {
        Ok(resp) => {
            eprintln!("ministr: primary session {} created", resp.session_id);
            resp.session_id
        }
        Err(e) => {
            return Err(miette::miette!("session creation failed: {e}"));
        }
    };

    // Resolve each linked project's (corpus_id, session_id) so the agent
    // can target it by label via the `project: "<label>"` argument on any
    // shared MCP tool. Failures are logged but non-fatal — the primary
    // corpus stays usable.
    let mut linked_backends: std::collections::HashMap<
        String,
        std::sync::Arc<ministr_mcp::backend::DaemonBackend>,
    > = std::collections::HashMap::new();
    let mut linked_cleanup: Vec<(String, String)> = Vec::new();
    for project in linked {
        let label = project.label.clone();
        let paths: Vec<String> = project.corpus_paths.clone();
        if paths.is_empty() {
            eprintln!("ministr: warning — linked project '{label}' has no corpus paths, skipping");
            continue;
        }
        // Pass the linked-project label as display_name so the tray UI
        // shows "BurntSushi-ripgrep" rather than the basename of the
        // (possibly content-hashed) clone dir.
        match client
            .register_corpus_with_display_name(&paths, Some(label.clone()))
            .await
        {
            Ok(resp) => {
                let linked_corpus_id = resp.corpus_id;
                match client.create_session(&linked_corpus_id, None).await {
                    Ok(sresp) => {
                        eprintln!(
                            "ministr: linked '{label}' → corpus {linked_corpus_id}, session {} (indexing_started={})",
                            sresp.session_id, resp.indexing_started
                        );
                        linked_cleanup.push((linked_corpus_id.clone(), sresp.session_id.clone()));
                        let backend = ministr_mcp::backend::DaemonBackend::new(
                            std::sync::Arc::clone(&client),
                            linked_corpus_id,
                            Some(sresp.session_id),
                        );
                        linked_backends.insert(label, std::sync::Arc::new(backend));
                    }
                    Err(e) => {
                        eprintln!(
                            "ministr: warning — linked '{label}' session creation failed: {e}"
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!("ministr: warning — linked '{label}' corpus registration failed: {e}");
            }
        }
    }

    eprintln!("ministr: starting MCP proxy on stdio (daemon-backend mode)");
    let mut server = if linked_backends.is_empty() {
        ministr_mcp::server::MinistrServer::with_daemon_backend(
            std::sync::Arc::clone(&client),
            corpus_id.clone(),
            session_id.clone(),
        )
    } else {
        let default_backend = std::sync::Arc::new(ministr_mcp::backend::DaemonBackend::new(
            std::sync::Arc::clone(&client),
            corpus_id.clone(),
            Some(session_id.clone()),
        ));
        let multi = ministr_mcp::backend::DaemonMultiBackend::new(default_backend, linked_backends);
        ministr_mcp::server::MinistrServer::with_daemon_multi_backend(multi, session_id.clone())
    };

    // Prune local-only tools — fetch / clone / refresh / task all need
    // local engine state (embedder, vector index, storage, fetchers) that
    // daemon-backend mode doesn't have. `prune_tools` already gates on
    // `web_fetcher.is_none() && git_fetcher.is_none()` etc., which is
    // exactly the daemon-mode state, so this call is enough.
    let local_paths: Vec<std::path::PathBuf> =
        corpus_paths.iter().map(std::path::PathBuf::from).collect();
    server.prune_tools(&local_paths);

    let cleanup_client = std::sync::Arc::clone(&client);
    let cleanup_corpus = corpus_id;
    let cleanup_session = session_id;

    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("proxy MCP server failed")?;

    // Keep the service alive until the client disconnects.
    let _ = service.waiting().await;

    // Clean up the primary daemon session.
    if let Err(e) = cleanup_client
        .destroy_session(&cleanup_corpus, &cleanup_session)
        .await
    {
        eprintln!("ministr: warning — primary session cleanup failed: {e}");
    }
    // Clean up linked-project sessions too so the desktop UI doesn't show
    // stale entries.
    for (linked_corpus, linked_session) in &linked_cleanup {
        if let Err(e) = cleanup_client
            .destroy_session(linked_corpus, linked_session)
            .await
        {
            eprintln!("ministr: warning — linked session cleanup for {linked_corpus} failed: {e}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr index
// ---------------------------------------------------------------------------

/// `ministr index` — run ingestion synchronously and exit.
pub(crate) async fn cmd_index(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    resolved_model: &str,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<()> {
    tracing::info!(
        corpus_count = corpus_paths.len(),
        config = %config_path.display(),
        "ministr index — {} corpus path(s)",
        corpus_paths.len()
    );
    for path in corpus_paths {
        tracing::info!(path = %path, "  corpus root");
    }

    if corpus_paths.is_empty() && git_includes.is_empty() {
        tracing::warn!("no corpus paths specified, nothing to index");
        return Ok(());
    }

    let ctx = infra::init_infrastructure(
        corpus_paths,
        config,
        Some(resolved_model),
        resolved_dimension,
        rerank_depth,
    )
    .await?;

    let progress = Arc::new(ministr_core::ingestion::IngestionProgress::new());
    // Local `ministr index` keeps the PHASE3 bundle-at-end shape;
    // streaming persist is opt-in via the cloud worker path below.
    ingestion::run_corpus_ingestion(corpus_paths, git_includes, &ctx, &progress, None).await?;

    tracing::info!("indexing complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr init
// ---------------------------------------------------------------------------

/// `ministr init` — detect project structure and generate `.ministr.toml`.
pub(crate) fn cmd_init(root: &Path, force: bool) -> Result<()> {
    let detection = ministr_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .ministr.toml")?;

    // Scaffold agent config files (Claude Code hooks, Cursor rules, etc.).
    let scaffolded = ministr_core::scaffold::scaffold_agent_config(root);

    eprintln!(
        "Detected project: {} ({})",
        detection.project_name, detection.project_type
    );
    for ws in &detection.workspaces {
        eprintln!("  {} workspace ({} members)", ws.kind, ws.members.len());
    }
    if !detection.bridges.is_empty() {
        let names: Vec<_> = detection.bridges.iter().map(|b| format!("{b:?}")).collect();
        eprintln!("  Bridges: {}", names.join(", "));
    }
    let langs = detection.detected_languages();
    if !langs.is_empty() {
        let names: Vec<_> = langs.iter().map(|l| format!("{l:?}")).collect();
        eprintln!("  Languages: {}", names.join(", "));
    }
    eprintln!();
    let config_path = root.join(".ministr.toml");
    let total_paths = detection.source_paths.len() + detection.doc_paths.len();
    if config_path.exists() && !force {
        eprintln!(".ministr.toml already exists (use --force to overwrite)");
    } else {
        eprintln!("Generated .ministr.toml with {total_paths} paths");
    }

    eprintln!();
    eprintln!("MCP server configs:");
    eprintln!("  ✓ .mcp.json (Claude Code)");
    eprintln!("  ✓ .vscode/mcp.json (VS Code / GitHub Copilot)");
    eprintln!("  ✓ .cursor/mcp.json (Cursor)");
    eprintln!();
    eprintln!(
        "Agent instructions ({} created, {} healed):",
        scaffolded.created, scaffolded.healed
    );
    eprintln!("  ✓ .claude/rules/          (tool scope, playbook)");
    eprintln!("  ✓ .claude/settings.json   (PreToolUse hooks — blocks Grep/Glob/Bash search)");
    eprintln!("  ✓ .github/hooks/          (Copilot CLI + cloud agent hooks — same enforcement)");
    eprintln!("  ✓ .cursor/rules/ministr.mdc  (Cursor rules — aggressive advisory)");
    eprintln!("  ✓ .cursor/hooks.json      (Cursor hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ .windsurf/hooks.json    (Windsurf hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ windsurf/rules/ministr.md  (Windsurf rules)");
    eprintln!("  ✓ .continue/rules/ministr.md (Continue.dev rules)");
    eprintln!("  ✓ .github/copilot-instructions.md");
    eprintln!("  ✓ AGENTS.md               (universal)");
    if scaffolded.custom_rules > 0 {
        eprintln!(
            "  ✓ ministr-custom.md           ({} custom rules injected)",
            scaffolded.custom_rules
        );
    }
    if !langs.is_empty() {
        let names: Vec<_> = langs.iter().map(|l| format!("{l:?}")).collect();
        eprintln!(
            "  ✓ ministr-lang-rules.md       ({} language playbook)",
            names.join(", ")
        );
    }
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new session in your preferred agent (Claude Code, Cursor, Copilot)");
    eprintln!("  2. ministr will auto-index and semantic search tools become available");
    eprintln!("  3. Grep/Glob/Bash search are blocked — agents must use ministr tools");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr init --interactive
// ---------------------------------------------------------------------------

/// `ministr init --interactive` — guided setup with a confirmation step.
///
/// `ministr init` writes one configuration set: corpus config plus the
/// steering scaffold for every supported agent platform. The interactive
/// flow shows what will be written and asks for confirmation — it does not
/// offer per-platform or "strictness" choices, because the scaffold is a
/// single non-blocking steering design (the earlier prompts collected
/// answers that were never applied; they are gone rather than faked).
pub(crate) fn cmd_init_interactive(root: &Path, force: bool) -> Result<()> {
    use dialoguer::Confirm;

    eprintln!("ministr interactive setup\n");

    let detection = ministr_core::init::detect_project(root);
    eprintln!("  Detected project type: {}", detection.project_type);
    eprintln!();
    eprintln!("`ministr init` will write:");
    eprintln!("  - .ministr.toml (corpus paths, auto-detected)");
    eprintln!("  - MCP client configs (merged non-destructively)");
    eprintln!("  - advisory agent rules for Claude Code, Cursor, Windsurf,");
    eprintln!("    Continue, Copilot, and a platform-neutral AGENTS.md");
    eprintln!("  - PreToolUse steering hooks for the platforms that support");
    eprintln!("    them (Claude Code, Cursor, Windsurf, Copilot — not");
    eprintln!("    Continue.dev or AGENTS.md, which are advisory only)");
    eprintln!();

    let proceed = Confirm::new()
        .with_prompt("Write .ministr.toml and scaffold agent configs?")
        .default(true)
        .interact()
        .into_diagnostic()?;

    if !proceed {
        eprintln!("Aborted.");
        return Ok(());
    }

    // `write_config` skips an existing `.ministr.toml` unless `--force`,
    // so report what actually happened rather than always "Created".
    let config_existed = root.join(".ministr.toml").exists();
    ministr_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .ministr.toml")?;
    let config_action = if config_existed && !force {
        ".ministr.toml left as-is (already present; pass --force to overwrite)"
    } else {
        ".ministr.toml written"
    };

    let scaffolded = ministr_core::scaffold::scaffold_agent_config(root);

    eprintln!();
    eprintln!(
        "Done! {config_action}; scaffolded {} files ({} created, {} healed).",
        scaffolded.touched(),
        scaffolded.created,
        scaffolded.healed,
    );
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new session in your preferred agent");
    eprintln!("  2. ministr auto-indexes; its semantic search and code-nav tools become available");
    eprintln!("  3. The hooks steer (they do not wall): the built-in Grep/Glob tools are");
    eprintln!("     declined in favor of ministr; a leading shell grep/find is allowed with");
    eprintln!("     a hint; pipelines are never intercepted");

    Ok(())
}

// ---------------------------------------------------------------------------
// ministr export / import
// ---------------------------------------------------------------------------

/// `ministr export` — export the corpus index to a portable bundle.
pub(crate) async fn cmd_export(
    corpus_paths: &[String],
    config: &ministr_core::config::MinistrConfig,
    resolved_model: &str,
    output: Option<&Path>,
) -> Result<()> {
    use ministr_core::bundle::{
        self, BUNDLE_FORMAT_VERSION, BundleCorpusRoot, BundleManifest, compute_bundle_version,
    };
    use ministr_core::storage::Storage as _;

    // Resolve the corpus data directory without loading the embedding model.
    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        infra::corpus_data_dir_name(corpus_paths)
    };
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    let db_path = corpus_dir.join("content.db");

    if !db_path.exists() {
        miette::bail!(
            "no indexed corpus found at {}. Run `ministr index` first.",
            corpus_dir.display()
        );
    }

    // Open storage (no embedder needed for export).
    let storage = ministr_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    let doc_count = storage
        .document_count()
        .await
        .into_diagnostic()
        .wrap_err("failed to count documents")?;
    let roots = storage
        .list_corpus_roots()
        .await
        .into_diagnostic()
        .wrap_err("failed to list corpus roots")?;

    // Get vector count and dimension from the HNSW index.
    let index_dir = corpus_dir.join("index");
    let (vector_count, dimension) = if index_dir.exists() {
        match ministr_core::index::HnswIndex::load(&index_dir) {
            Ok(loaded) => (loaded.len(), loaded.dimension()),
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    let bundle_roots: Vec<BundleCorpusRoot> = roots
        .iter()
        .map(|r| BundleCorpusRoot {
            id: r.id.clone(),
            display_name: r.display_name.clone(),
            kind: r.kind.as_str().to_string(),
            commit_sha: r.commit_sha.clone(),
            branch: r.branch.clone(),
            repo_url: r.repo_url.clone(),
        })
        .collect();

    // Capture the source commit SHA: prefer corpus root metadata, fall back
    // to `git rev-parse HEAD` in the first corpus path.
    let source_commit = bundle_roots
        .iter()
        .find_map(|r| r.commit_sha.clone())
        .or_else(|| {
            corpus_paths
                .first()
                .and_then(|p| ministr_core::git::local_head_sha(std::path::Path::new(p)))
        });

    let bundle_version = Some(compute_bundle_version(&bundle_roots));

    let manifest = BundleManifest {
        format_version: BUNDLE_FORMAT_VERSION,
        model_name: resolved_model.to_string(),
        dimension,
        vector_count,
        document_count: doc_count,
        symbol_count: 0,
        corpus_roots: bundle_roots,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        bundle_version,
        source_commit,
    };

    let output_path = output.map_or_else(
        || {
            let filename = format!("{corpus_name}.ministr-index");
            PathBuf::from(filename)
        },
        Path::to_path_buf,
    );

    bundle::export_bundle(&corpus_dir, &output_path, &manifest)
        .into_diagnostic()
        .wrap_err("failed to export bundle")?;

    eprintln!("Exported {doc_count} documents, {vector_count} vectors ({dimension}d)");
    eprintln!("Bundle: {}", output_path.display());
    Ok(())
}

/// `ministr import` — import a `.ministr-index` bundle into local storage.
pub(crate) fn cmd_import(
    corpus_paths: &[String],
    config: &ministr_core::config::MinistrConfig,
    bundle_path: &Path,
) -> Result<()> {
    use ministr_core::bundle;

    if !bundle_path.exists() {
        miette::bail!("bundle not found: {}", bundle_path.display());
    }

    // Determine corpus directory name from the bundle filename or corpus paths.
    let corpus_name = if corpus_paths.is_empty() {
        bundle_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_owned()
    } else {
        infra::corpus_data_dir_name(corpus_paths)
    };
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);

    if corpus_dir.join("content.db").exists() {
        miette::bail!(
            "corpus '{}' already exists at {}. Remove it first or use a different name.",
            corpus_name,
            corpus_dir.display()
        );
    }

    let manifest = bundle::import_bundle(bundle_path, &corpus_dir)
        .into_diagnostic()
        .wrap_err("failed to import bundle")?;

    eprintln!(
        "Imported: {} documents, {} vectors ({}d, model: {})",
        manifest.document_count, manifest.vector_count, manifest.dimension, manifest.model_name
    );
    eprintln!("Corpus: {}", corpus_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr status / search
// ---------------------------------------------------------------------------

/// `ministr status` — show corpus stats from local storage.
///
/// Opens the `SQLite` database directly (no embedding model needed) and
/// displays document counts, corpus roots, data directory, and index info.
/// Falls back to the daemon API if available for richer live status.
#[allow(clippy::too_many_lines)]
pub(crate) async fn cmd_daemon_status() -> Result<()> {
    use ministr_core::storage::Storage as _;

    // Try daemon first for live status.
    let client = ministr_api::client::DaemonClient::new();
    if client.is_available()
        && let Ok(status) = client.status().await
    {
        eprintln!("ministr daemon v{}", status.version);
        eprintln!("  Uptime:    {}s", status.uptime_secs);
        eprintln!("  Memory:    {:.0} MB", status.memory_mb);
        eprintln!(
            "  Model:     {} ({}d)",
            status.model, status.model_dimension
        );
        eprintln!("  Corpora:   {}", status.corpora.len());
        for c in &status.corpora {
            // Show the human-readable display name; fall back to the id
            // only if the daemon predates the `display_name` field.
            let label = if c.display_name.is_empty() {
                c.id.as_str()
            } else {
                c.display_name.as_str()
            };
            eprintln!(
                "    {label} — {} files, {} sections, {} embeddings [{}]",
                c.files_indexed,
                c.sections_count,
                c.embeddings_count,
                match &c.status {
                    ministr_api::corpus::IndexingStatus::Idle => "idle".to_string(),
                    ministr_api::corpus::IndexingStatus::Indexing {
                        files_done,
                        files_total,
                    } => format!("indexing {files_done}/{files_total}"),
                    ministr_api::corpus::IndexingStatus::Error { message } =>
                        format!("error: {message}"),
                }
            );
        }
        return Ok(());
    }

    // Daemon not available — show local storage stats.
    let config_path = ministr_core::config::MinistrConfig::default_path();
    let config = ministr_core::config::MinistrConfig::load(&config_path)
        .into_diagnostic()
        .wrap_err("failed to load config")?;

    let cwd = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("failed to get current directory")?;
    let corpus_config = ministr_core::config::RepoConfig::discover(&cwd)
        .into_diagnostic()
        .wrap_err("failed to read .ministr.toml")?;

    let corpus_paths: Vec<String> = if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else {
        config.corpus_paths.clone()
    };

    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        infra::corpus_data_dir_name(&corpus_paths)
    };

    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    let db_path = corpus_dir.join("content.db");
    let index_dir = corpus_dir.join("index");

    eprintln!("ministr status (local)");
    eprintln!();
    eprintln!("  Data dir:  {}", corpus_dir.display());
    eprintln!(
        "  Database:  {}",
        if db_path.exists() {
            "exists"
        } else {
            "not found"
        }
    );
    eprintln!(
        "  Index dir: {}",
        if index_dir.exists() {
            "exists"
        } else {
            "not found"
        }
    );

    if !db_path.exists() {
        eprintln!();
        eprintln!("  No index found. Run `ministr serve` or `ministr index` to build one.");
        return Ok(());
    }

    let storage = ministr_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    let doc_count = storage.document_count().await.unwrap_or(0);
    let roots = storage.list_corpus_roots().await.unwrap_or_default();

    eprintln!("  Documents: {doc_count}");
    eprintln!("  Roots:     {}", roots.len());
    for r in &roots {
        let name = r.display_name.as_deref().unwrap_or(&r.path);
        eprintln!("    {name} ({} — {} files)", r.kind.as_str(), r.file_count);
    }

    // Show index file sizes.
    if index_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&index_dir)
    {
        let total_bytes: u64 = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let mb = total_bytes as f64 / 1_048_576.0;
        eprintln!("  Index size: {mb:.1} MB");
    }

    Ok(())
}

/// `ministr search` — search the corpus via the daemon.
pub(crate) async fn cmd_daemon_search(
    corpus_paths: &[String],
    query: &str,
    top_k: usize,
) -> Result<()> {
    let client = ministr_api::client::DaemonClient::new();
    if !client.is_available() {
        miette::bail!(
            "ministr daemon is not running (no endpoint at {})",
            client.endpoint()
        );
    }

    // Register corpus if needed.
    let resp = client
        .register_corpus(corpus_paths)
        .await
        .into_diagnostic()
        .wrap_err("failed to register corpus")?;

    let results = client
        .survey(&resp.corpus_id, query, Some(top_k))
        .await
        .into_diagnostic()
        .wrap_err("search failed")?;

    for r in &results.results {
        eprintln!("[{:8}] {:.3}  {}", r.resolution, r.score, r.content_id);
        eprintln!("  {}", r.text.lines().next().unwrap_or(""));
        eprintln!();
    }

    if results.results.is_empty() {
        eprintln!("No results found.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// ministr hooks test
// ---------------------------------------------------------------------------

/// `ministr hooks test` — validate installed hook files and simulate tool calls.
pub(crate) fn cmd_hooks_test(root: &Path) {
    use std::collections::BTreeMap;

    /// A simulated tool call for testing.
    struct TestCase {
        tool: &'static str,
        args: &'static str,
        should_block: bool,
    }

    let test_cases = &[
        TestCase {
            tool: "Grep",
            args: r#"{"pattern": "fn main"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Glob",
            args: r#"{"pattern": "**/*.rs"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "grep -r TODO ."}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "find . -name '*.rs'"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cat file.rs | grep fn"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "rg pattern src/"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cargo test"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cargo build"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "git status"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Read",
            args: r#"{"path": "src/main.rs"}"#,
            should_block: false,
        },
    ];

    // ── Check hook files ────────────────────────────────────────────────
    let hook_files: BTreeMap<&str, std::path::PathBuf> = [
        ("Claude Code", root.join(".claude/settings.json")),
        (
            "Copilot CLI",
            root.join(".github/hooks/ministr-enforce.json"),
        ),
        ("Cursor", root.join(".cursor/hooks.json")),
        ("Windsurf", root.join(".windsurf/hooks.json")),
    ]
    .into_iter()
    .collect();

    eprintln!("Hook files:");
    let mut any_missing = false;
    for (platform, path) in &hook_files {
        if path.exists() {
            // Validate JSON structure.
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(_) => eprintln!(
                        "  ✓ {platform:<14} {}",
                        path.strip_prefix(root).unwrap_or(path).display()
                    ),
                    Err(e) => eprintln!("  ✗ {platform:<14} invalid JSON: {e}"),
                },
                Err(e) => eprintln!("  ✗ {platform:<14} read error: {e}"),
            }
        } else {
            eprintln!("  ✗ {platform:<14} not found (run `ministr init`)");
            any_missing = true;
        }
    }

    // ── Check advisory files ────────────────────────────────────────────
    eprintln!();
    eprintln!("Advisory files:");
    let advisory_files: &[(&str, &str)] = &[
        ("Claude rules", ".claude/rules/ministr-scope.md"),
        ("Cursor rules", ".cursor/rules/ministr.mdc"),
        ("Windsurf rules", "windsurf/rules/ministr.md"),
        ("Continue rules", ".continue/rules/ministr.md"),
        ("Copilot instructions", ".github/copilot-instructions.md"),
        ("AGENTS.md", "AGENTS.md"),
        ("Language rules", ".claude/rules/ministr-lang-rules.md"),
        ("Custom rules", ".claude/rules/ministr-custom.md"),
    ];
    for (name, rel_path) in advisory_files {
        let path = root.join(rel_path);
        if path.exists() {
            eprintln!("  ✓ {name:<22} {rel_path}");
        } else {
            eprintln!("  · {name:<22} not present");
        }
    }

    if any_missing {
        eprintln!();
        eprintln!("⚠ Some hook files are missing. Run `ministr init` to generate them.");
    }

    // ── Simulate tool calls ─────────────────────────────────────────────
    eprintln!();
    eprintln!("Simulated tool calls:");
    let mut pass = 0;
    let mut fail = 0;
    for tc in test_cases {
        let expected = if tc.should_block { "BLOCK" } else { "ALLOW" };
        let actual_blocked = would_hook_block(tc.tool, tc.args);
        let actual = if actual_blocked { "BLOCK" } else { "ALLOW" };
        let ok = actual_blocked == tc.should_block;
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }

        // Truncate args for display.
        let cmd_display = if tc.tool == "Bash" {
            serde_json::from_str::<serde_json::Value>(tc.args)
                .ok()
                .and_then(|v| v["command"].as_str().map(String::from))
                .unwrap_or_else(|| tc.args.to_string())
        } else {
            tc.tool.to_string()
        };

        let icon = if ok { "✓" } else { "✗" };
        let expect_str = if ok {
            String::new()
        } else {
            format!(" (expected {expected})")
        };
        eprintln!("  {icon} {cmd_display:<40} → {actual}{expect_str}");
    }

    eprintln!();
    eprintln!("{pass} passed, {fail} failed");

    if fail > 0 {
        eprintln!("⚠ Some simulations did not match expected behavior.");
    }
}

// ---------------------------------------------------------------------------
// ministr setup
// ---------------------------------------------------------------------------

/// `ministr setup` — add the `ministr` binary's directory to the user's PATH.
///
/// Wraps the `onpath` crate so installer scripts (`install.sh`, the Tauri
/// first-run flow) don't have to hand-roll cross-shell rc-file edits. On
/// Unix, writes to bash / zsh / fish / nushell / `PowerShell` / tcsh / xonsh
/// rc files for shells the user actually has installed. On Windows, writes
/// the per-user `HKCU\Environment\PATH` registry entry — same surface
/// `install.ps1` and the Tauri NSIS installer hook target, so re-running is
/// idempotent regardless of how the user got here.
///
/// This is the **single source of truth** for where the ministr CLI
/// lives and what is on `PATH`. Every channel funnels through it: the
/// dev `just reinstall` scripts, the Tauri app's first-launch
/// `setup.rs`, and the NSIS installer hooks. They used to each PATH-add
/// a *different* directory (dev → `~/.ministr/bin`, packaged →
/// `%LOCALAPPDATA%\ministr`), and nothing ever removed the stale one —
/// so an old build permanently shadowed the new one on `PATH` and no
/// amount of reinstalling helped. Consolidating here fixes that
/// structurally:
///
/// 1. The canonical location is always `<daemon_data_dir>/bin`
///    (`~/.ministr/bin`), independent of where the running binary sits.
///    The legacy `--bin-dir` argument is accepted for compatibility but
///    no longer changes the target — every caller converges here.
/// 2. The running binary is staged into the canonical dir (so the
///    packaged app / NSIS, whose `ministr` lives elsewhere, still puts
///    the *current* binary on the canonical `PATH`).
/// 3. Known legacy / duplicate ministr install roots are de-PATHed and
///    their shadowing binaries refreshed with the current one, so a
///    stale copy can never win `PATH` resolution again.
///
/// `uninstall=true` removes the canonical dir from `PATH` (NSIS
/// uninstaller hook) and skips staging / legacy refresh.
pub(crate) fn cmd_setup(bin_dir: Option<&Path>, dry_run: bool, uninstall: bool) -> Result<()> {
    // Canonical, channel-independent install location. `--bin-dir` is
    // intentionally ignored for the target (kept only so existing
    // callers / NSIS hooks don't break) — the whole point of this
    // routine is that every entry point lands in the same place.
    let _ = bin_dir; // legacy arg — no longer changes the target (see above)
    let bin_dir = ministr_api::daemon_data_dir().join("bin");
    let exe_name = if cfg!(windows) {
        "ministr.exe"
    } else {
        "ministr"
    };
    let canonical_exe = bin_dir.join(exe_name);

    // Stage the running binary into the canonical dir (best-effort —
    // a locked target on Windows must not abort PATH wiring; the next
    // run heals it).
    if !uninstall
        && !dry_run
        && let Ok(current) = std::env::current_exe()
        && current != canonical_exe
    {
        if let Err(e) = std::fs::create_dir_all(&bin_dir) {
            eprintln!("warning: could not create {}: {e}", bin_dir.display());
        } else if let Err(e) = std::fs::copy(&current, &canonical_exe) {
            eprintln!(
                "warning: could not stage ministr into {}: {e}",
                canonical_exe.display()
            );
        }
    }

    let manager = onpath::PathManager::new(&bin_dir, "ministr").dry_run(dry_run);
    let (verb, report) = if uninstall {
        let r = manager
            .remove()
            .into_diagnostic()
            .wrap_err_with(|| format!("onpath failed to remove {} from PATH", bin_dir.display()))?;
        ("remove", r)
    } else {
        let r = manager
            .add()
            .into_diagnostic()
            .wrap_err_with(|| format!("onpath failed to add {} to PATH", bin_dir.display()))?;
        // De-PATH legacy/duplicate install roots and refresh any
        // shadowing binaries so a stale copy can't win resolution.
        if !dry_run {
            neutralize_legacy_ministr(&bin_dir, &canonical_exe);
        }
        ("add", r)
    };

    // Report (which shells / files were edited) goes to stdout so callers
    // like install.sh can capture it; user-facing reminders go to stderr.
    println!("{report}");

    if dry_run {
        eprintln!("(dry-run — nothing was written)");
    } else if verb == "add" {
        eprintln!();
        eprintln!(
            "Open a new shell (or `source` the modified rc file) for the change to take effect."
        );
    }

    Ok(())
}

/// De-PATH known legacy / duplicate ministr install roots and refresh
/// any shadowing `ministr` binaries with the canonical one, so a stale
/// copy can never win `PATH` resolution again. Best-effort throughout:
/// a missing dir, a locked file, or a failed `PATH` edit must not break
/// `setup` — the canonical dir is already wired by the caller.
fn neutralize_legacy_ministr(canonical_bin: &Path, canonical_exe: &Path) {
    // ministr-DEDICATED legacy dirs → safe to drop from PATH entirely.
    // These are Windows-only (the packaged-bundle `%LOCALAPPDATA%\ministr`
    // root + its `bin`, from an older installer that shadowed the dev
    // install on PATH).
    #[cfg(windows)]
    if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
        let root = std::path::PathBuf::from(lad).join("ministr");
        for dir in [root.join("bin"), root] {
            if dir.as_path() == canonical_bin || !dir.exists() {
                continue;
            }
            let _ = onpath::PathManager::new(&dir, "ministr").remove();
            refresh_shadowing_binaries(&dir, canonical_exe);
        }
    }

    // Shared dirs (hold other tools) → never de-PATH; only refresh a
    // stale `ministr` so it isn't an old build if still resolved first.
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(home) = std::env::var_os(home_var) {
        let cargo_bin = std::path::PathBuf::from(home).join(".cargo").join("bin");
        if cargo_bin.as_path() != canonical_bin {
            refresh_shadowing_binaries(&cargo_bin, canonical_exe);
        }
    }
}

/// Overwrite any CLI `ministr` executable in `dir` with the canonical
/// binary (never touches `ministr-app.exe` — a different program).
///
/// Windows blocks overwriting a *running* `.exe` (the stale copy is
/// exactly the one being executed via PATH, so it is loaded), but it
/// *does* allow renaming it. So on a plain-copy failure we move the
/// locked file aside (`<name>.stale`) and copy the fresh binary into
/// place — the rename succeeds even while the old image runs, and the
/// orphan is swept on the next pass once nothing holds it. Best-effort.
fn refresh_shadowing_binaries(dir: &Path, canonical_exe: &Path) {
    for name in ["ministr.exe", "ministr-cli.exe", "ministr"] {
        let f = dir.join(name);
        if !f.is_file() || f.as_path() == canonical_exe {
            continue;
        }
        if std::fs::copy(canonical_exe, &f).is_ok() {
            continue;
        }
        // Locked (running) target: rename aside, then copy fresh in.
        let aside = dir.join(format!("{name}.stale"));
        let _ = std::fs::remove_file(&aside);
        if std::fs::rename(&f, &aside).is_ok() {
            let _ = std::fs::copy(canonical_exe, &f);
        }
    }
    // Sweep any `.stale` orphans from a previous locked pass.
    for name in [
        "ministr.exe.stale",
        "ministr-cli.exe.stale",
        "ministr.stale",
    ] {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// Check if the Claude Code hooks would block a given tool/args combination.
///
/// This simulates the `PreToolUse` hook logic from `.claude/settings.json`.
fn would_hook_block(tool_name: &str, tool_args: &str) -> bool {
    let search_tools = ["grep", "Grep", "egrep", "fgrep", "rg", "ag", "ack"];
    let find_tools = ["find", "fd"];

    match tool_name {
        "Grep" | "Glob" => true,
        "Bash" => {
            let cmd = serde_json::from_str::<serde_json::Value>(tool_args)
                .ok()
                .and_then(|v| v["command"].as_str().map(String::from))
                .unwrap_or_default();

            // Direct search command.
            let first_word = cmd.split_whitespace().next().unwrap_or("");
            if search_tools.contains(&first_word) || find_tools.contains(&first_word) {
                return true;
            }

            // Piped to search tool.
            if cmd.contains('|') {
                for tool in &search_tools {
                    if cmd.contains(&format!("| {tool}")) || cmd.contains(&format!("|{tool}")) {
                        return true;
                    }
                }
            }

            false
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// ministr atlas — F2.6
// ---------------------------------------------------------------------------

/// `ministr atlas reindex` — F2.6 worker entrypoint.
///
/// The Azure Container Apps Job invokes this on the F4.2 weekly cron.
/// F2.6 v0 ships the orchestration with no-op step impls so the
/// command itself is stable: the cron's structured-log dashboard, the
/// dead-letter table, and the alerts all see real data from day one.
///
/// F4.2 replaces the no-op trait impls below with concrete
/// `ministr_core::git::GitFetcher` / corpus-registry / Azure Blob
/// upload paths without changing this function's signature.
pub(crate) async fn cmd_atlas_reindex() -> miette::Result<()> {
    use std::pin::Pin;
    use std::sync::Arc;

    use ministr_atlas::{
        BlobWriter, Cloner, IndexerStep, ReindexError, reindex_once,
    };

    type BoxFut<'a, T> =
        Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

    /// No-op clone step — logs the URL and returns a synthetic path.
    /// F4.2 replaces with a real `ministr_core::git::GitFetcher`.
    #[derive(Debug)]
    struct StubCloner;
    impl Cloner for StubCloner {
        fn clone_to_tmp<'a>(
            &'a self,
            clone_url: &'a str,
        ) -> BoxFut<'a, Result<std::path::PathBuf, ReindexError>> {
            Box::pin(async move {
                tracing::info!(clone_url, "atlas: would clone (stub)");
                Ok(std::path::PathBuf::from(format!(
                    "/tmp/atlas-stub-{}",
                    clone_url.len()
                )))
            })
        }
    }

    /// No-op index step — returns a placeholder bundle handle.
    #[derive(Debug)]
    struct StubIndexer;
    impl IndexerStep for StubIndexer {
        fn index_dir<'a>(
            &'a self,
            path: &'a std::path::Path,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move {
                tracing::info!(path = %path.display(), "atlas: would index (stub)");
                Ok(format!("stub-bundle:{}", path.display()))
            })
        }
    }

    /// No-op blob writer — returns the synthetic blob path the cron
    /// dashboard expects to see in the log.
    #[derive(Debug)]
    struct StubWriter;
    impl BlobWriter for StubWriter {
        fn write_blob<'a>(
            &'a self,
            slug: &'a str,
            _handle: &'a str,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move {
                let blob = format!("atlas/{slug}/latest.idx");
                tracing::info!(blob, "atlas: would write (stub)");
                Ok(blob)
            })
        }
    }

    let cloner: Arc<dyn Cloner> = Arc::new(StubCloner);
    let indexer: Arc<dyn IndexerStep> = Arc::new(StubIndexer);
    let writer: Arc<dyn BlobWriter> = Arc::new(StubWriter);
    let license: Arc<dyn ministr_atlas::LicenseFilter> =
        Arc::new(ministr_atlas::SpdxFilter);
    let optout: Arc<dyn ministr_atlas::OptOutRegistry> =
        Arc::new(ministr_atlas::InMemoryRegistry::new());

    tracing::info!(
        seed_count = ministr_atlas::ATLAS_SEED_REPOS.len(),
        "atlas reindex starting (F2.6 v0 stub orchestration)"
    );
    let outcome = reindex_once(&cloner, &indexer, &writer, &license, &optout).await;
    tracing::info!(
        indexed = outcome.indexed.len(),
        skipped = outcome.skipped.len(),
        failed = outcome.failed.len(),
        "atlas reindex complete"
    );
    if !outcome.failed.is_empty() {
        tracing::warn!("{} step failures recorded", outcome.failed.len());
    }
    Ok(())
}

/// `ministr atlas manifest` — emit the F2.6 v0 manifest as JSON on
/// stdout. The cron pipes this into the Atlas storage account so the
/// public mirror at `ministr.ai/atlas/manifest.json` stays in sync.
pub(crate) fn cmd_atlas_manifest() -> miette::Result<()> {
    let manifest = ministr_atlas::ManifestSnapshot::from_seed_list();
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| miette::miette!("serialise atlas manifest: {e}"))?;
    println!("{json}");
    Ok(())
}

/// `ministr api-keys flag-stale --threshold-days N` — F3.4c-ii
/// weekly stale-keys cron entrypoint. Scans `api_keys` for rows whose
/// `last_used_at` (or `created_at` for never-used keys) is older than
/// `threshold_days` days and emits an `api_key.stale` audit event per
/// row. Returns the flagged-count + elapsed wall-clock as structured
/// log fields so the cron's dashboard can alarm on a runaway pass.
///
/// Requires `MINISTR_PG_URL` (the cloud Postgres connection string).
/// Exits with a miette error if the env var is missing or the SELECT
/// fails; the Container Apps Job's failure-retry policy can then alert.
///
/// Idempotent: re-runs against an unchanged table emit the same
/// audit-event set. The audit table itself dedupes only at the row
/// level; downstream consumers should treat a re-emitted
/// `api_key.stale` as informational, not duplicate-detection-required.
pub(crate) async fn cmd_api_keys_flag_stale(threshold_days: u32) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "ministr api-keys flag-stale requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    let pool_arc = std::sync::Arc::new(pool);
    let sink = ministr_cloud::PostgresAuditSink::from_arc(std::sync::Arc::clone(&pool_arc));
    tracing::info!(threshold_days, "api-keys flag-stale starting");
    let outcome = ministr_cloud::flag_stale_api_keys(&pool_arc, threshold_days, &sink)
        .await
        .into_diagnostic()
        .wrap_err("flag stale api_keys")?;
    tracing::info!(
        flagged = outcome.flagged,
        elapsed_ms = u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        threshold_days = outcome.threshold_days,
        "api-keys flag-stale complete"
    );
    Ok(())
}

/// `ministr audit prune --retention-days N` — F3.7c daily-retention
/// cron entrypoint. Drops `audit_events` rows older than
/// `retention_days` days; logs the row count + elapsed wall-clock so
/// the cron's structured-log dashboard can render the operation.
///
/// Requires `MINISTR_PG_URL` to be set — the cloud Postgres connection
/// string the rest of `cmd_serve_http` already consumes. Exits with a
/// miette error (non-zero status) if the env var is missing or the
/// DELETE fails; the Container Apps Job's failure-retry policy can
/// then alert.
pub(crate) async fn cmd_audit_prune(retention_days: u32) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "ministr audit prune requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    tracing::info!(retention_days, "audit prune starting");
    let outcome = ministr_cloud::prune_audit_events(&pool, retention_days)
        .await
        .into_diagnostic()
        .wrap_err("prune audit_events")?;
    tracing::info!(
        deleted = outcome.deleted,
        elapsed_ms = u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        retention_days = outcome.retention_days,
        "audit prune complete"
    );
    Ok(())
}

/// `ministr audit ensure-partitions --lookahead-quarters N` — F5.3-c-ii
/// CLI surface that mirrors `cmd_serve_http`'s boot-time call. Useful
/// for operator-driven catch-up + cron jobs that don't want to restart
/// the serve to push the forward edge of `audit_events` partitions out.
///
/// Requires `MINISTR_PG_URL`. Idempotent — a re-run with the same
/// lookahead creates 0 new partitions.
/// `ministr audit archive --partition NAME --archive-dir DIR` —
/// F5.3-c-ii-archive-fs operator-driven cold archive. SELECTs all
/// rows from the named partition, writes them as a gzipped JSONL
/// file at `<archive_dir>/<partition>.jsonl.gz` (typically
/// `audit_events_yYYYYqN.jsonl.gz`), then `DETACH PARTITION` +
/// `DROP TABLE` it from the live database. The named file
/// becomes the authoritative copy.
///
/// Idempotency: a second invocation against an already-archived
/// partition returns a non-zero exit with a clear error
/// (`"partition is not a child of audit_events"`). The archive file
/// is OVERWRITTEN if the customer re-runs after a transient
/// failure between FS write + DETACH; this is acceptable since
/// the file contents would be identical (same partition data).
pub(crate) async fn cmd_audit_archive(
    partition: &str,
    archive_dir: &std::path::Path,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "ministr audit archive requires MINISTR_PG_URL \
             (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    tracing::info!(partition, archive_dir = %archive_dir.display(), "audit archive starting");
    let outcome = ministr_cloud::archive_audit_partition_to_dir(&pool, partition, archive_dir)
        .await
        .into_diagnostic()
        .wrap_err("archive audit partition")?;
    tracing::info!(
        partition,
        rows = outcome.rows,
        bytes_on_disk = outcome.bytes_on_disk,
        file_path = %outcome.file_path.display(),
        "audit archive complete"
    );
    Ok(())
}

pub(crate) async fn cmd_audit_ensure_partitions(
    lookahead_quarters: u32,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "ministr audit ensure-partitions requires MINISTR_PG_URL \
             (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    tracing::info!(lookahead_quarters, "audit ensure-partitions starting");
    let outcome = ministr_cloud::ensure_audit_partitions(&pool, lookahead_quarters)
        .await
        .into_diagnostic()
        .wrap_err("ensure audit_events partitions")?;
    tracing::info!(
        existing = outcome.existing,
        created = outcome.created,
        target_end_year = outcome.target_end_year,
        target_end_quarter = outcome.target_end_quarter,
        "audit ensure-partitions complete"
    );
    Ok(())
}

/// `ministr cloud mint-test-bearer --github-id N --email E [--scope S]`
/// — F-Test-1 helper. The OAuth self-issuer's DCR flow mints `client_id`
/// values that aren't UUIDs (22-char base64url strings), but the cloud
/// schema's tenant lattice (`users.id`, `org_members.user_id`, the
/// visibility-filter `$1::uuid` cast) assumes UUID subjects produced by
/// the GitHub callback. This helper bridges that gap by:
///
/// 1. Upserting a `users` row via `upsert_github_user` — same path the
///    real GitHub callback uses, so the resulting UUID has the exact
///    shape production tenants have.
/// 2. Minting a bearer token via `OAuthStore::issue_bearer_token` bound
///    to that UUID subject.
///
/// Prints `{"user_id":"...","token":"...","plan_id":"..."}` on stdout.
/// The harness scripts this once per tenant and uses the returned token
/// as `Authorization: Bearer …` for assertions.
///
/// Requires `MINISTR_PG_URL`. Idempotent: re-running with the same
/// `github_id` returns the same UUID (the `users.github_id` UNIQUE key
/// drives `ON CONFLICT`). A fresh token is minted each call — old
/// tokens remain valid until they expire.
pub(crate) async fn cmd_cloud_mint_test_bearer(
    github_id: i64,
    email: &str,
    scope: &str,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "ministr cloud mint-test-bearer requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    let identity = ministr_cloud::idp::ResolvedIdentity {
        issuer: ministr_cloud::idp::github::GITHUB_ISSUER.to_string(),
        subject: github_id.to_string(),
        email: Some(email.to_string()),
        display_name: None,
        github_id: Some(github_id),
    };
    let user = ministr_cloud::upsert_github_user(&pool, &identity)
        .await
        .into_diagnostic()
        .wrap_err("upsert test user")?;
    let store = ministr_mcp::auth::OAuthStore::postgres(
        ministr_mcp::auth::OAuthConfig::default(),
        &pg_url,
    )
    .await
    .into_diagnostic()
    .wrap_err("open OAuth store")?;
    let token = store
        .issue_bearer_token(&user.id, scope)
        .await
        .into_diagnostic()
        .wrap_err("issue bearer token")?;
    // Stdout is JSON so the harness can `jq -r .token`. Logs go to
    // stderr already (tracing default), so this doesn't pollute either.
    let out = serde_json::json!({
        "user_id": user.id,
        "token": token,
        "plan_id": user.plan_id,
    });
    println!("{}", serde_json::to_string(&out).unwrap_or_else(|_| "{}".into()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_parser_none_when_input_is_none() {
        assert!(parse_cors_allowed_origins(None).is_none());
    }

    #[test]
    fn cors_parser_single_origin_round_trips() {
        let out = parse_cors_allowed_origins(Some("https://ministr.ai"))
            .expect("single origin parses");
        assert_eq!(out, vec!["https://ministr.ai".to_string()]);
    }

    #[test]
    fn cors_parser_multiple_origins_round_trip() {
        let out = parse_cors_allowed_origins(Some(
            "https://ministr.ai, https://docs.ministr.ai,http://localhost:3000",
        ))
        .expect("three origins parse");
        assert_eq!(
            out,
            vec![
                "https://ministr.ai".to_string(),
                "https://docs.ministr.ai".to_string(),
                "http://localhost:3000".to_string(),
            ]
        );
    }

    #[test]
    fn cors_parser_drops_malformed_entries_but_keeps_valid_ones() {
        // First entry is missing the scheme; should be dropped + log.
        let out = parse_cors_allowed_origins(Some(
            "ministr.ai, https://ministr.ai",
        ))
        .expect("at least one entry parses");
        assert_eq!(out, vec!["https://ministr.ai".to_string()]);
    }

    #[test]
    fn cors_parser_returns_none_when_every_entry_is_malformed() {
        // Every entry lacks "://" → all rejected → overall None.
        assert!(parse_cors_allowed_origins(Some("foo, bar baz")).is_none());
    }

    #[test]
    fn cors_parser_returns_none_for_empty_string() {
        assert!(parse_cors_allowed_origins(Some("")).is_none());
        assert!(parse_cors_allowed_origins(Some("  ,  ")).is_none());
    }

    #[test]
    fn cors_parser_rejects_entries_containing_whitespace() {
        // Whitespace inside an origin is malformed (e.g. accidental
        // newline embedded in the env var). The outer trim handles
        // surrounding spaces; this guards the inside.
        let out = parse_cors_allowed_origins(Some("https://good.example, https://bad ex.com"));
        assert_eq!(out, Some(vec!["https://good.example".to_string()]));
    }

    #[test]
    fn cors_layer_builds_without_panicking_on_valid_origins() {
        // build_cors_layer takes already-parsed origins, so it's
        // mainly a "does the tower-http API still compile + accept"
        // smoke. The CorsLayer type is opaque so we just confirm
        // construction succeeds.
        let _layer = build_cors_layer(&[
            "https://ministr.ai".to_string(),
            "http://localhost:3000".to_string(),
        ]);
    }
}
