//! High-level façade handlers code against.
//!
//! `OAuthStore` bundles configuration with a backend and exposes the small
//! set of operations the OAuth handlers actually need. Handlers depend on
//! `OAuthStore`, not on the storage trait directly — that keeps generic
//! plumbing out of axum and centralises backend-selection logic.

use std::io;
use std::path::Path;
use std::sync::Arc;

use ministr_api::{ApiKeyResolver, PlanResolver};
use tracing::warn;

use super::OAuthConfig;
use super::storage::{InMemoryStorage, OAuthBackend, PostgresStorage, SqliteStorage, StorageResult};
use super::tenant::{Plan, Tenant};
use super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::util::{epoch_now, generate_id};

/// Configured OAuth state plus the chosen storage backend.
///
/// `Clone` is cheap: the backend variants hold either `Arc`-wrapped state
/// (`InMemory`) or `Arc`-wrapped clients (future Cosmos backend). This makes
/// it safe to use directly as axum `State<OAuthStore>`.
#[derive(Debug, Clone)]
pub struct OAuthStore {
    config: OAuthConfig,
    backend: OAuthBackend,
    /// F3.4a — optional fall-through resolver for service-account API
    /// keys. When `Some`, [`Self::resolve_tenant`] tries OAuth first
    /// and falls back to this resolver on miss. Self-hosted serve
    /// leaves it `None`; only OAuth tokens authenticate there.
    api_key_resolver: Option<Arc<dyn ApiKeyResolver>>,
    /// F5.5-a-plan-lookup — optional plan resolver. When `Some`, the
    /// OAuth-path [`Self::resolve_tenant`] looks up the validated
    /// subject's `users.plan_id` so the constructed `Tenant.plan`
    /// reflects the real billing tier instead of `Tenant::local()`'s
    /// `Plan::Pro` default. Self-hosted serve leaves it `None` and the
    /// existing local-tenant shape is preserved.
    plan_resolver: Option<Arc<dyn PlanResolver>>,
}

impl OAuthStore {
    /// Construct a store backed by the in-memory backend (default).
    #[must_use]
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            backend: OAuthBackend::InMemory(InMemoryStorage::new()),
            api_key_resolver: None,
            plan_resolver: None,
        }
    }

    /// F3.4a — install an [`ApiKeyResolver`] so service-account API
    /// keys authenticate alongside OAuth tokens. Cloud deployments call
    /// this with a `PostgresApiKeyResolver` from `ministr-cloud`;
    /// self-hosted serve leaves the field `None`.
    #[must_use]
    pub fn with_api_key_resolver(mut self, resolver: Arc<dyn ApiKeyResolver>) -> Self {
        self.api_key_resolver = Some(resolver);
        self
    }

    /// F5.5-a-plan-lookup — install a [`PlanResolver`] so the OAuth
    /// path resolves the real `users.plan_id` instead of defaulting
    /// every OAuth-authenticated request to `Plan::Pro`. Cloud
    /// deployments call this with a `PostgresPlanResolver` from
    /// `ministr-cloud`; self-hosted serve leaves the field `None` and
    /// the existing `Tenant::local()` shape is preserved (the
    /// resolver isn't useful there — `validate_token` returns OAuth
    /// `client_ids`, not user UUIDs).
    #[must_use]
    pub fn with_plan_resolver(mut self, resolver: Arc<dyn PlanResolver>) -> Self {
        self.plan_resolver = Some(resolver);
        self
    }

    /// Construct a store backed by `SQLite` at `db_path`. The file is
    /// created if missing and survives process restarts — meant for ACA
    /// deployments where the path is on the Azure Files mount.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the database file cannot be opened or
    /// the schema cannot be initialised.
    pub fn persistent(config: OAuthConfig, db_path: &Path) -> io::Result<Self> {
        let backend = SqliteStorage::open(db_path)
            .map(OAuthBackend::Sqlite)
            .map_err(io::Error::other)?;
        Ok(Self {
            config,
            backend,
            api_key_resolver: None,
            plan_resolver: None,
        })
    }

    /// Construct a store backed by Postgres at `url` — the cloud default
    /// for `mcp.ministr.ai` (F1.2 sub-bullet 4). The OAuth schema is
    /// created idempotently on first use; multiple pods sharing the same
    /// database all participate in the same OAuth state without any
    /// coordination beyond the connection string.
    ///
    /// `url` is a standard libpq connection string
    /// (`postgres://user:pw@host/db?sslmode=require`). Azure Postgres
    /// Flex requires TLS server-side; the backend wires rustls + the
    /// Mozilla CA bundle unconditionally.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the pool cannot be opened or the
    /// schema cannot be ensured.
    pub async fn postgres(config: OAuthConfig, url: &str) -> io::Result<Self> {
        let backend = PostgresStorage::open(url)
            .await
            .map(OAuthBackend::Postgres)
            .map_err(io::Error::other)?;
        Ok(Self {
            config,
            backend,
            api_key_resolver: None,
            plan_resolver: None,
        })
    }

    /// Read-only view of the configuration.
    #[must_use]
    pub(crate) fn config(&self) -> &OAuthConfig {
        &self.config
    }

    // ── Client lifecycle ───────────────────────────────────────────────────

    pub(crate) async fn save_client(&self, client: RegisteredClient) -> StorageResult<()> {
        self.backend.save_client(client).await
    }

    pub(crate) async fn get_client(
        &self,
        client_id: &str,
    ) -> StorageResult<Option<RegisteredClient>> {
        self.backend.get_client(client_id).await
    }

    // ── Authorization codes ────────────────────────────────────────────────

    pub(crate) async fn save_code(&self, code: AuthorizationCode) -> StorageResult<()> {
        self.backend.save_code(code).await
    }

    pub(crate) async fn take_code(
        &self,
        code: &str,
    ) -> StorageResult<Option<AuthorizationCode>> {
        self.backend.take_code(code).await
    }

    // ── Tokens ─────────────────────────────────────────────────────────────

    pub(crate) async fn save_token(&self, token: AccessToken) -> StorageResult<()> {
        self.backend.save_token(token).await
    }

    /// Mint a fresh bearer token bound to `client_id` + `scope` and persist
    /// it through the configured storage backend.
    ///
    /// The lifetime is the store's [`OAuthConfig::token_ttl`]. The returned
    /// string is the opaque token value — clients use it as the `Bearer`
    /// header on subsequent requests.
    ///
    /// Cloud-side federation flows (F1.3 GitHub `IdP`) call this after
    /// resolving the user's identity to deliver a token without going
    /// through the RFC 6749 §4.1 code-grant dance. The local-stack OAuth
    /// handlers continue to use the existing private path.
    ///
    /// # Errors
    ///
    /// Returns [`OAuthIssueError::Storage`] when the backend rejects the
    /// write (network outage, schema drift, etc.). Matches the closed-fail
    /// posture of [`Self::validate_token`].
    pub async fn issue_bearer_token(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<String, OAuthIssueError> {
        let token = generate_id();
        let access = AccessToken {
            token: token.clone(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
            expires_at: epoch_now() + self.config.token_ttl.as_secs(),
        };
        self.backend
            .save_token(access)
            .await
            .map_err(|e| OAuthIssueError::Storage(e.to_string()))?;
        Ok(token)
    }

    /// Validate a bearer token. Returns the `client_id` if the token exists
    /// and has not expired.
    ///
    /// Storage backend failures are logged and treated as invalid — we
    /// degrade closed: a transient Cosmos blip rejects the request rather
    /// than letting an unauthenticated caller through.
    pub(crate) async fn validate_token(&self, token: &str) -> Option<String> {
        match self.backend.get_token(token).await {
            Ok(Some(access)) if epoch_now() <= access.expires_at => Some(access.client_id),
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "oauth storage error during token validation; rejecting");
                None
            }
        }
    }

    /// Resolve the [`Tenant`] for a bearer token.
    ///
    /// Order: try the OAuth path first (the hot path — short-lived
    /// access tokens minted via the OAuth code grant or the F1.3 GitHub
    /// federation). On miss, fall through to the F3.4a service-account
    /// API key resolver if one is installed. The fall-through never
    /// runs for self-hosted serve (which leaves `api_key_resolver` as
    /// `None`) so the existing `Tenant::local` shape is preserved
    /// there.
    pub(crate) async fn resolve_tenant(&self, token: &str) -> Option<Tenant> {
        if let Some(client_id) = self.validate_token(token).await {
            return Some(self.tenant_from_oauth_subject(client_id).await);
        }
        self.resolve_api_key(token, None).await
    }

    /// Resolve the [`Tenant`] for a bearer token **and** require a
    /// specific scope claim. Same OAuth-then-api-key fallback as
    /// [`Self::resolve_tenant`].
    pub(crate) async fn resolve_tenant_with_scope(
        &self,
        token: &str,
        required_scope: &str,
    ) -> Option<Tenant> {
        if let Some(client_id) = self.validate_token_with_scope(token, required_scope).await {
            return Some(self.tenant_from_oauth_subject(client_id).await);
        }
        self.resolve_api_key(token, Some(required_scope)).await
    }

    /// F5.5-a-plan-lookup — build a [`Tenant`] from an OAuth-validated
    /// subject. When a [`PlanResolver`] is wired (cloud mode), looks up
    /// the subject's `users.plan_id` and constructs a `Tenant` whose
    /// `plan` reflects the real billing tier; otherwise falls back to
    /// [`Tenant::local`] (`Plan::Pro` default — self-hosted shape).
    ///
    /// Resolver errors are logged at warn and treated as "no plan
    /// known" → fall back to Pro. Returning Pro on a transient
    /// backend blip is the right closed-loop posture: the worst case
    /// is admitting a higher tier at a lower quota lane, not letting
    /// an unauthenticated request through (the OAuth validation
    /// already passed).
    async fn tenant_from_oauth_subject(&self, subject: String) -> Tenant {
        let Some(resolver) = self.plan_resolver.as_ref() else {
            return Tenant::local(subject);
        };
        match resolver.resolve(&subject).await {
            Ok(Some(plan_id)) => Tenant {
                subject,
                org_id: None,
                plan: parse_plan_id(&plan_id),
            },
            Ok(None) => Tenant::local(subject),
            Err(e) => {
                warn!(error = %e, "plan resolver storage error; falling back to Plan::Pro");
                Tenant::local(subject)
            }
        }
    }

    /// F3.4a — fall-through resolver: hash the candidate token, look it
    /// up in `api_keys`, optionally check the required scope, and
    /// fire-and-forget a `last_used_at` touch on success. Returns
    /// `None` when no resolver is installed, the lookup misses, the
    /// scope check fails, or the storage layer errors out (fail
    /// closed, mirroring the OAuth-path posture).
    async fn resolve_api_key(
        &self,
        raw_token: &str,
        required_scope: Option<&str>,
    ) -> Option<Tenant> {
        let Some(api_resolver) = self.api_key_resolver.as_ref() else {
            tracing::debug!("api-key resolver not installed; bare OAuth-only validation");
            return None;
        };
        let key_data = match api_resolver.resolve(raw_token).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                tracing::debug!(
                    token_prefix = %raw_token.chars().take(16).collect::<String>(),
                    "api-key resolver: token not found in api_keys",
                );
                return None;
            }
            Err(e) => {
                warn!(error = %e, "api-key resolver storage error; rejecting");
                return None;
            }
        };
        if let Some(needed) = required_scope
            && !key_data.scopes.split_whitespace().any(|s| s == needed)
        {
            tracing::debug!(
                needed,
                scopes = %key_data.scopes,
                "api-key resolver: scope check failed",
            );
            return None;
        }
        let plan = parse_plan_id(&key_data.plan_id);
        // Fire-and-forget last_used touch — the request hot path doesn't
        // wait on the write. A failed touch logs but is otherwise
        // invisible (the user's request still succeeds).
        let toucher = Arc::clone(api_resolver);
        let key_id = key_data.key_id.clone();
        tokio::spawn(async move {
            if let Err(e) = toucher.touch_last_used(&key_id).await {
                warn!(error = %e, key_id, "api-key last_used touch failed");
            }
        });
        Some(Tenant {
            subject: key_data.subject,
            org_id: key_data.org_id,
            plan,
        })
    }

    /// Validate a bearer token **and** require that its scope claim contains
    /// `required_scope` as a whitespace-separated entry.
    pub(crate) async fn validate_token_with_scope(
        &self,
        token: &str,
        required_scope: &str,
    ) -> Option<String> {
        match self.backend.get_token(token).await {
            Ok(Some(access)) if epoch_now() <= access.expires_at => {
                if access.scope.split_whitespace().any(|s| s == required_scope) {
                    Some(access.client_id)
                } else {
                    None
                }
            }
            Ok(_) => None,
            Err(e) => {
                warn!(error = %e, "oauth storage error during scoped token validation; rejecting");
                None
            }
        }
    }
}

/// Parse a wire-shape plan id (`"pro" | "team" | "enterprise"`) into
/// the [`Plan`] enum. Unknown / malformed values fall through to the
/// default ([`Plan::Pro`]) — fail-open here is fine because the worst
/// case is admitting a paying tier at a lower quota lane.
fn parse_plan_id(plan_id: &str) -> Plan {
    match plan_id.trim().to_ascii_lowercase().as_str() {
        "team" => Plan::Team,
        "enterprise" => Plan::Enterprise,
        // "pro" or anything else: default to Pro.
        _ => Plan::Pro,
    }
}

/// Public error surface for [`OAuthStore::issue_bearer_token`]. Internal
/// storage variants are collapsed into a single opaque string so callers
/// outside `ministr-mcp` (e.g. cloud federation) don't depend on the
/// backend taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum OAuthIssueError {
    /// Backend rejected the write. The inner string is human-readable and
    /// safe to log; do not surface it directly in HTTP responses.
    #[error("oauth storage error: {0}")]
    Storage(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ministr_api::{PlanResolverError, ResolvePlanFuture};

    fn store() -> OAuthStore {
        OAuthStore::new(OAuthConfig::default())
    }

    /// F5.5-a-plan-lookup — stub resolver that returns a fixed value
    /// for one subject and Ok(None) for everything else, plus a synth
    /// error path so the resolver-error fallback can be exercised.
    #[derive(Debug)]
    struct StubPlanResolver {
        known: (String, String),
        boom_subject: Option<String>,
    }

    impl PlanResolver for StubPlanResolver {
        fn resolve<'a>(&'a self, subject: &'a str) -> ResolvePlanFuture<'a> {
            let known = self.known.clone();
            let boom = self.boom_subject.clone();
            Box::pin(async move {
                if Some(subject) == boom.as_deref() {
                    return Err(PlanResolverError::Storage("synthetic".into()));
                }
                if subject == known.0 {
                    return Ok(Some(known.1));
                }
                Ok(None)
            })
        }
    }

    #[tokio::test]
    async fn tenant_from_oauth_subject_without_resolver_is_local() {
        let store = store();
        let tenant = store.tenant_from_oauth_subject("alice".into()).await;
        assert_eq!(tenant.subject, "alice");
        assert!(tenant.org_id.is_none());
        assert_eq!(tenant.plan, Plan::Pro);
    }

    #[tokio::test]
    async fn tenant_from_oauth_subject_with_resolver_yields_real_plan() {
        let resolver = Arc::new(StubPlanResolver {
            known: ("enterprise-user".into(), "enterprise".into()),
            boom_subject: None,
        });
        let store = store().with_plan_resolver(resolver);
        let tenant = store
            .tenant_from_oauth_subject("enterprise-user".into())
            .await;
        assert_eq!(tenant.subject, "enterprise-user");
        assert_eq!(tenant.plan, Plan::Enterprise);
    }

    #[tokio::test]
    async fn tenant_from_oauth_subject_falls_back_when_resolver_misses() {
        let resolver = Arc::new(StubPlanResolver {
            known: ("known".into(), "team".into()),
            boom_subject: None,
        });
        let store = store().with_plan_resolver(resolver);
        // Subject the resolver doesn't recognise (e.g. a self-hosted
        // client_id) round-trips to the Tenant::local default.
        let tenant = store.tenant_from_oauth_subject("nobody".into()).await;
        assert_eq!(tenant.plan, Plan::Pro);
    }

    #[tokio::test]
    async fn tenant_from_oauth_subject_swallows_resolver_error() {
        let resolver = Arc::new(StubPlanResolver {
            known: (String::new(), String::new()),
            boom_subject: Some("blowup".into()),
        });
        let store = store().with_plan_resolver(resolver);
        // Resolver error must not reject the request — already-validated
        // OAuth token stays good; tenant falls back to Pro.
        let tenant = store.tenant_from_oauth_subject("blowup".into()).await;
        assert_eq!(tenant.plan, Plan::Pro);
    }

    fn token(name: &str, scope: &str, ttl_secs: u64, expired: bool) -> AccessToken {
        let expires_at = if expired {
            epoch_now().saturating_sub(ttl_secs)
        } else {
            epoch_now() + ttl_secs
        };
        AccessToken {
            token: name.into(),
            client_id: "client-1".into(),
            scope: scope.into(),
            expires_at,
        }
    }

    #[tokio::test]
    async fn validates_fresh_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(store.validate_token("t1").await, Some("client-1".into()));
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 100, true))
            .await
            .unwrap();
        assert_eq!(store.validate_token("t1").await, None);
    }

    #[tokio::test]
    async fn rejects_unknown_token() {
        assert_eq!(store().validate_token("never-issued").await, None);
    }

    #[tokio::test]
    async fn scope_matching_succeeds_for_both_listed_scopes() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read ministr:bundle:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            Some("client-1".into())
        );
        assert_eq!(
            store.validate_token_with_scope("t1", "ministr:read").await,
            Some("client-1".into())
        );
    }

    #[tokio::test]
    async fn scope_missing_returns_none() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn scope_present_but_expired_returns_none() {
        let store = store();
        store
            .save_token(token("t1", "ministr:bundle:read", 100, true))
            .await
            .unwrap();
        assert_eq!(
            store
                .validate_token_with_scope("t1", "ministr:bundle:read")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn resolves_tenant_for_valid_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        let tenant = store.resolve_tenant("t1").await.expect("tenant resolves");
        assert_eq!(tenant.subject, "client-1");
        assert!(tenant.org_id.is_none());
        assert_eq!(tenant.plan, super::super::tenant::Plan::Pro);
    }

    #[tokio::test]
    async fn resolves_none_for_unknown_token() {
        assert!(store().resolve_tenant("never-issued").await.is_none());
    }

    #[tokio::test]
    async fn resolves_none_for_expired_token() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 100, true))
            .await
            .unwrap();
        assert!(store.resolve_tenant("t1").await.is_none());
    }

    #[tokio::test]
    async fn scoped_tenant_requires_matching_scope() {
        let store = store();
        store
            .save_token(token("t1", "ministr:read", 3600, false))
            .await
            .unwrap();
        assert!(
            store
                .resolve_tenant_with_scope("t1", "ministr:read")
                .await
                .is_some()
        );
        assert!(
            store
                .resolve_tenant_with_scope("t1", "ministr:bundle:write")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn postgres_backed_store_round_trips_a_token() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let store = OAuthStore::postgres(OAuthConfig::default(), &url)
            .await
            .expect("open postgres oauth store");
        let tok = token("pg-t1", "ministr:read", 3600, false);
        store.save_token(tok.clone()).await.unwrap();
        assert_eq!(
            store.validate_token(&tok.token).await,
            Some("client-1".into())
        );
        let tenant = store
            .resolve_tenant(&tok.token)
            .await
            .expect("tenant resolves through postgres backend");
        assert_eq!(tenant.subject, "client-1");
    }

    #[tokio::test]
    async fn issue_bearer_token_round_trips_through_validate() {
        let store = store();
        let issued = store
            .issue_bearer_token("github:42", "ministr:read ministr:write")
            .await
            .expect("issue succeeds against in-memory store");
        assert_eq!(
            store.validate_token(&issued).await,
            Some("github:42".into())
        );
        assert_eq!(
            store
                .resolve_tenant_with_scope(&issued, "ministr:read")
                .await
                .map(|t| t.subject),
            Some("github:42".into())
        );
    }

    #[tokio::test]
    async fn issue_bearer_token_returns_distinct_tokens() {
        let store = store();
        let a = store.issue_bearer_token("c", "ministr:read").await.unwrap();
        let b = store.issue_bearer_token("c", "ministr:read").await.unwrap();
        assert_ne!(a, b, "successive issues must not collide");
    }

    #[tokio::test]
    async fn config_round_trips() {
        let config = OAuthConfig {
            issuer: "https://test.example".into(),
            scopes_supported: vec!["ministr:read".into()],
            token_ttl: Duration::from_secs(60),
            code_ttl: Duration::from_secs(30),
        };
        let store = OAuthStore::new(config.clone());
        assert_eq!(store.config().issuer, config.issuer);
    }
}
