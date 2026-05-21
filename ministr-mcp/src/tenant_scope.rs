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

use crate::auth::tenant::Tenant;

tokio::task_local! {
    /// Task-local subject of the resolved tenant for the duration of a
    /// single HTTP request. Set by [`scope_tenant`], read by [`current`].
    static TENANT_SUBJECT: Option<String>;
}

/// Return the current request's tenant subject, or `None` when called
/// outside a [`scope_tenant`] scope (self-hosted serve, in-process MCP
/// tests, stdio transport).
#[must_use]
pub fn current() -> Option<String> {
    TENANT_SUBJECT
        .try_with(Clone::clone)
        .ok()
        .flatten()
}

/// Axum middleware: pulls the [`Tenant`] from the request extensions and
/// scopes the rest of the request handling in a [`TENANT_SUBJECT`]
/// task-local. Mount AFTER [`crate::auth::middleware::validate_token_middleware`]
/// (which populates the extension) and BEFORE rmcp's
/// `StreamableHttpService`.
pub async fn scope_tenant(req: Request, next: Next) -> Response {
    let subject = req.extensions().get::<Tenant>().map(|t| t.subject.clone());
    TENANT_SUBJECT.scope(subject, next.run(req)).await
}

/// Test helper: run `fut` with [`TENANT_SUBJECT`] bound to `subject`.
///
/// Mirrors what [`scope_tenant`] does for live requests so unit tests can
/// exercise code paths that read [`current`] without standing up an axum
/// middleware stack.
#[cfg(test)]
pub(crate) async fn scope_for_test<F, T>(subject: Option<String>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    TENANT_SUBJECT.scope(subject, fut).await
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
}
