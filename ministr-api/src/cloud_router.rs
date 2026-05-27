//! F31.2b — `CloudRouterMounter` MIT seam between `ministr-cli` and the
//! proprietary `ministr-cloud` crate.
//!
//! `ministr-cli`'s `cmd_serve_http` accepts `Option<&dyn CloudRouterMounter>`.
//! When `None`, the function runs the self-hosted code path only —
//! identical to the pre-F31 behaviour when no `MINISTR_PG_URL` is set.
//! When `Some`, it calls [`CloudRouterMounter::setup`] before binding the
//! HTTP listener and applies the returned adapter slots / cloud Router
//! onto the locally-built state.
//!
//! The trait's concrete implementation lives in
//! `ministr_cloud::cli::ClassicCloudMounter` (proprietary). The
//! `ministr-cloud-tools` binary constructs it for `ministr-cloud-tools serve`.
//! ministr-cli stays MIT and never references either crate at compile time.
//!
//! ## Why the adapter-bag shape
//!
//! The cloud overlay touches ~36 sites in `cmd_serve_http` today — mostly
//! wiring `Arc<dyn Trait>` cloud adapters (`PostgresUsageSink`,
//! `PostgresAuditSink`, `BlobBackendSink`, …) into daemon / OAuth / server
//! state slots that already accept those traits. The trait surface
//! therefore mirrors those existing seams: a single async `setup` returns
//! optional `Arc<dyn>` slots grouped by destination plus a cloud
//! [`axum::Router`] to merge with the base router. The cloud impl owns
//! its Postgres pool, blob backend, license validation, and background
//! tasks internally.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use crate::api_key::ApiKeyResolver;
use crate::audit::AuditSink;
use crate::blob_sink::BlobSink;
use crate::corpora_repo::CorporaRepo;
use crate::corpus_restorer::CorpusRestorer;
use crate::drops_ledger::DropsLedger;
use crate::github_app::InstallationTokenMinter;
use crate::index_job_sink::IndexJobSink;
use crate::plan_resolver::PlanResolver;
use crate::session_bundle_store::SessionBundleStore;
use crate::session_storage::SessionStorage;
use crate::sla_window_store::SlaWindowStore;
use crate::tenant_filter::{TenantCorpusFilter, TenantCorpusVisibility};
use crate::usage::UsageSink;
use crate::ApiError;

/// Mounts cloud-mode HTTP routes and adapters onto a locally-built serve.
///
/// Implementations own the full cloud-side lifecycle: validating any
/// Enterprise license, opening Postgres / blob / Stripe / GitHub-App
/// clients, building cloud-only axum routers, populating the
/// returned adapter slots, and spawning whatever background tasks the
/// cloud needs (latency flush, blob upload reactor, indexer worker
/// loop, license-revocation refresh).
///
/// The MIT serve path in `ministr-cli` calls [`setup`] **once** at boot;
/// the returned [`CloudMountOutput`] is then applied to the in-progress
/// daemon / OAuth / server state and merged onto the axum app before
/// the listener binds. There is no per-request callback.
///
/// [`setup`]: CloudRouterMounter::setup
pub trait CloudRouterMounter: Send + Sync + std::fmt::Debug {
    /// Set up cloud-mode resources and return adapters to wire into the
    /// local serve. Called once during `cmd_serve_http`.
    ///
    /// # Errors
    ///
    /// Returns an [`ApiError`] if license validation refuses boot, the
    /// Postgres pool cannot open, migrations fail, or any other cloud
    /// resource refuses to come up. Self-hosted callers never invoke
    /// this method (they pass `mounter = None`).
    fn setup<'a>(
        &'a self,
        input: &'a CloudMountInput,
    ) -> Pin<Box<dyn Future<Output = Result<CloudMountOutput, ApiError>> + Send + 'a>>;
}

/// Thin inputs the cloud mounter needs from the local serve.
///
/// Kept deliberately small — anything the cloud impl can derive itself
/// (env vars, Postgres pool, blob backend, license) is NOT passed here.
#[derive(Debug, Clone)]
pub struct CloudMountInput {
    /// The embedding model name the local serve resolved
    /// (e.g. `"BAAI/bge-small-en-v1.5"`). Mirrored into blob sink
    /// manifests + bundle-route URLs.
    pub resolved_model: String,
    /// Count of `corpus_paths` the local serve was started with —
    /// surfaced into `AdminState::set_corpus_count` for the `/healthz`
    /// payload.
    pub corpus_count: usize,
    /// Optional cloud data dir (the cloud impl reads
    /// `MINISTR_CLOUD_DATA_DIR` itself; this is just a hint so the
    /// caller can preserve compatibility with its own persistence
    /// directory if needed).
    pub data_dir_hint: Option<PathBuf>,
}

/// What [`CloudRouterMounter::setup`] returns — adapter slots + cloud
/// Router + optional revocation shutdown signal.
///
/// Every adapter field is `Option<Arc<dyn _>>` so the local serve only
/// applies what the mounter actually provided. Missing adapters leave
/// the corresponding daemon / OAuth / server slot in its self-hosted
/// default (typically `None`, which collapses to a no-op inside the
/// adapter trait's call-site).
#[derive(Default)]
pub struct CloudMountOutput {
    /// Cloud-only axum router (orgs, billing, webhooks, SAML/OIDC RP,
    /// atlas, GitHub sign-in, …). Merged onto the base app before the
    /// listener binds.
    pub router: Router,
    /// Adapters wired into `ministr_daemon::state::AppState`.
    pub daemon_adapters: CloudDaemonAdapters,
    /// Adapters wired into `MinistrServer` / `CorpusRegistry`.
    pub server_adapters: CloudServerAdapters,
    /// Adapters wired into `ministr_mcp::auth::OAuthStore`.
    pub oauth_adapters: CloudOAuthAdapters,
    /// Adapters wired into `ministr_mcp::admin::AdminState`.
    pub admin_adapters: CloudAdminAdapters,
    /// Optional license-revocation shutdown handle. When set, the
    /// local serve calls [`RevocationHandle::shutdown_future`] inside
    /// `axum::serve(…).with_graceful_shutdown(…)` and exits 1 on
    /// post-serve [`RevocationHandle::is_revoked`].
    pub shutdown: Option<Arc<dyn RevocationHandle>>,
}

impl std::fmt::Debug for CloudMountOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudMountOutput")
            .field("router", &"axum::Router(..)")
            .field("daemon_adapters", &self.daemon_adapters)
            .field("server_adapters", &self.server_adapters)
            .field("oauth_adapters", &self.oauth_adapters)
            .field("admin_adapters", &self.admin_adapters)
            .field("shutdown_set", &self.shutdown.is_some())
            .finish()
    }
}

/// Adapters merged into `AppState` via its `with_*` setters.
#[derive(Default)]
pub struct CloudDaemonAdapters {
    pub usage_sink: Option<Arc<dyn UsageSink>>,
    pub audit_sink: Option<Arc<dyn AuditSink>>,
    pub index_job_sink: Option<Arc<dyn IndexJobSink>>,
    pub blob_sink: Option<Arc<dyn BlobSink>>,
    pub installation_minter: Option<Arc<dyn InstallationTokenMinter>>,
    pub corpus_visibility: Option<Arc<dyn TenantCorpusVisibility>>,
}

impl std::fmt::Debug for CloudDaemonAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudDaemonAdapters")
            .field("usage_sink", &self.usage_sink.is_some())
            .field("audit_sink", &self.audit_sink.is_some())
            .field("index_job_sink", &self.index_job_sink.is_some())
            .field("blob_sink", &self.blob_sink.is_some())
            .field("installation_minter", &self.installation_minter.is_some())
            .field("corpus_visibility", &self.corpus_visibility.is_some())
            .finish()
    }
}

/// Adapters merged into `MinistrServer` / `CorpusRegistry`.
#[derive(Default)]
pub struct CloudServerAdapters {
    pub tenant_filter: Option<Arc<dyn TenantCorpusFilter>>,
    pub session_storage: Option<Arc<dyn SessionStorage>>,
    pub drops_ledger: Option<Arc<dyn DropsLedger>>,
    pub corpora_repo: Option<Arc<dyn CorporaRepo>>,
    pub corpus_restorer: Option<Arc<dyn CorpusRestorer>>,
    pub session_bundle_store: Option<Arc<dyn SessionBundleStore>>,
}

impl std::fmt::Debug for CloudServerAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudServerAdapters")
            .field("tenant_filter", &self.tenant_filter.is_some())
            .field("session_storage", &self.session_storage.is_some())
            .field("drops_ledger", &self.drops_ledger.is_some())
            .field("corpora_repo", &self.corpora_repo.is_some())
            .field("corpus_restorer", &self.corpus_restorer.is_some())
            .field("session_bundle_store", &self.session_bundle_store.is_some())
            .finish()
    }
}

/// Adapters merged into `ministr_mcp::auth::OAuthStore`.
#[derive(Default)]
pub struct CloudOAuthAdapters {
    pub api_key_resolver: Option<Arc<dyn ApiKeyResolver>>,
    pub plan_resolver: Option<Arc<dyn PlanResolver>>,
}

impl std::fmt::Debug for CloudOAuthAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudOAuthAdapters")
            .field("api_key_resolver", &self.api_key_resolver.is_some())
            .field("plan_resolver", &self.plan_resolver.is_some())
            .finish()
    }
}

/// Adapters merged into `ministr_mcp::admin::AdminState`.
#[derive(Default)]
pub struct CloudAdminAdapters {
    pub sla_window_store: Option<Arc<dyn SlaWindowStore>>,
}

impl std::fmt::Debug for CloudAdminAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudAdminAdapters")
            .field("sla_window_store", &self.sla_window_store.is_some())
            .finish()
    }
}

/// License-revocation shutdown signal returned by the cloud mounter.
///
/// The cloud's URL-based revocation refresh task uses this to signal
/// the serve loop when a license is mid-flight revoked; the local
/// serve calls [`shutdown_future`] inside
/// `axum::serve(..).with_graceful_shutdown(..)` and then checks
/// [`is_revoked`] after the listener exits to decide whether to
/// `process::exit(1)` (so the orchestrator restarts and the boot
/// validator refuses the now-revoked license).
///
/// [`shutdown_future`]: RevocationHandle::shutdown_future
/// [`is_revoked`]: RevocationHandle::is_revoked
pub trait RevocationHandle: Send + Sync + std::fmt::Debug {
    /// Future that resolves when the handle has been signalled. The
    /// serve loop awaits this inside `with_graceful_shutdown`.
    fn shutdown_future(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// `true` iff the revocation refresh task signalled this handle
    /// due to a confirmed mid-flight revocation. The serve loop
    /// `process::exit(1)`s when true so the orchestrator restarts.
    fn is_revoked(&self) -> bool;
}
