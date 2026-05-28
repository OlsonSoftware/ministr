//! F2.x-b — tenant scoping for MCP tool dispatch.
//!
//! Tool handlers (`#[tool]` methods in `MinistrServer`) cannot take a
//! `RequestContext<RoleServer>` parameter without breaking the unit
//! test surface in `server::mod` — rmcp 1.5 makes `Peer::new` crate-
//! private, so tests that call `server.X(Parameters(params))` directly
//! cannot construct a `RequestContext`. Instead, the cloud HTTP entrypoint
//! mounts a thin axum middleware ([`scope_tenant`]) that pulls the
//! `Tenant` populated by `validate_token_middleware` from the request
//! extensions and wraps the inner service in a tokio task-local scope.
//! rmcp's `StreamableHttpService::call` returns a `BoxFuture` awaited
//! inside that scope, so every tool dispatch — including the eventual
//! `Backend::Registry::resolve_registry_handle` — can recover the
//! tenant via [`current`].
//!
//! Stdio / proxy transports never mount [`scope_tenant`], so [`current`]
//! returns `None` and the resolver hits its permissive fallback — the
//! self-hosted single-tenant posture is preserved.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::auth::tenant::{Plan, Tenant};

tokio::task_local! {
    /// Task-local subject of the resolved tenant for the duration of a
    /// single HTTP request. Set by [`scope_tenant`], read by [`current`].
    static TENANT_SUBJECT: Option<String>;

    /// F5.5-a-priority — task-local billing plan, set alongside
    /// `TENANT_SUBJECT` by [`scope_tenant`]. Read by [`current_plan`]
    /// so producer-side code (e.g. `PostgresIndexJobSink::create_pending`)
    /// can derive a queue priority via
    /// [`crate::auth::tenant::queue_priority`] without needing the
    /// `Tenant` extension on its own call surface.
    static TENANT_PLAN: Option<Plan>;
}

/// Return the current request's tenant subject, or `None` when called
/// outside a [`scope_tenant`] scope (self-hosted serve, in-process MCP
/// tests, stdio transport).
#[must_use]
pub fn current() -> Option<String> {
    TENANT_SUBJECT.try_with(Clone::clone).ok().flatten()
}

/// F5.5-a-priority — return the current request's billing plan, or
/// `None` when called outside a [`scope_tenant`] scope.
#[must_use]
pub fn current_plan() -> Option<Plan> {
    TENANT_PLAN.try_with(|p| *p).ok().flatten()
}

/// Axum middleware: pulls the [`Tenant`] from the request extensions and
/// scopes the rest of the request handling in the [`TENANT_SUBJECT`] +
/// [`TENANT_PLAN`] task-locals. Mount AFTER
/// [`crate::auth::middleware::validate_token_middleware`] (which
/// populates the extension) and BEFORE rmcp's `StreamableHttpService`.
pub async fn scope_tenant(req: Request, next: Next) -> Response {
    let (subject, plan) = req
        .extensions()
        .get::<Tenant>()
        .map_or((None, None), |t| (Some(t.subject.clone()), Some(t.plan)));
    // Nest both task-local scopes — entering `TENANT_PLAN` inside
    // `TENANT_SUBJECT` means both are simultaneously readable for the
    // duration of the request future.
    TENANT_SUBJECT
        .scope(subject, TENANT_PLAN.scope(plan, next.run(req)))
        .await
}

/// Test helper: run `fut` with [`TENANT_SUBJECT`] + [`TENANT_PLAN`]
/// bound. Mirrors what [`scope_tenant`] does for live requests so unit
/// tests can exercise code paths that read [`current`] / [`current_plan`]
/// without standing up an axum middleware stack.
#[cfg(test)]
pub(crate) async fn scope_for_test<F, T>(subject: Option<String>, plan: Option<Plan>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    TENANT_SUBJECT
        .scope(subject, TENANT_PLAN.scope(plan, fut))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_returns_none_outside_scope() {
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn current_returns_value_inside_scope() {
        TENANT_SUBJECT
            .scope(Some("alice".to_string()), async {
                assert_eq!(current().as_deref(), Some("alice"));
            })
            .await;
    }

    #[tokio::test]
    async fn current_returns_none_when_scope_carries_none() {
        TENANT_SUBJECT
            .scope(None, async {
                assert!(current().is_none());
            })
            .await;
    }

    #[tokio::test]
    async fn current_plan_returns_none_outside_scope() {
        assert!(current_plan().is_none());
    }

    #[tokio::test]
    async fn current_plan_round_trips_via_scope_for_test() {
        // F5.5-a-priority — the producer-side wiring needs to recover
        // the requesting tenant's plan from this task-local.
        scope_for_test(Some("alice".to_string()), Some(Plan::Enterprise), async {
            assert_eq!(current().as_deref(), Some("alice"));
            assert_eq!(current_plan(), Some(Plan::Enterprise));
        })
        .await;
    }

    #[tokio::test]
    async fn current_plan_independent_of_subject() {
        // Plan can be set without a subject (defensive — should never
        // happen in production but the scope shape mustn't panic).
        scope_for_test(None, Some(Plan::Team), async {
            assert!(current().is_none());
            assert_eq!(current_plan(), Some(Plan::Team));
        })
        .await;
    }
}
