//! Request → bucket-key extractors.
//!
//! Two ship with F2.2: [`ip_key`] (per-source-IP) and [`tenant_key`]
//! (per-Tenant). Both are plain functions, not a trait — the
//! middleware accepts a `fn(&Request) -> Option<String>` so adding a
//! new strategy (e.g. `(ip, tenant)` composite) is a one-line addition
//! here without touching the layer.
//!
//! Returning `Option<String>` lets the middleware fall through to "no
//! limit" when the request lacks the key — e.g. an unauthenticated
//! probe has no Tenant; an `X-Forwarded-For`-less direct call has no
//! reliable IP. Per ROADMAP §3 the public surface always sits behind
//! Azure Container Apps' ingress so XFF is present in production.

use axum::extract::ConnectInfo;
use axum::http::Request;
use ministr_mcp::auth::Tenant;
use std::net::SocketAddr;

/// Header set by Azure Container Apps' ingress and most reverse
/// proxies, holding the original client IP.
const XFF_HEADER: &str = "x-forwarded-for";

/// Extract the client IP. Honours `X-Forwarded-For` first (production
/// path) and falls back to the TCP peer address via `ConnectInfo`
/// (local dev / direct connections).
///
/// `X-Forwarded-For` can carry a list — `client, proxy1, proxy2`.
/// The left-most entry is the original client; that's what we key on.
#[must_use]
pub fn ip_key<B>(req: &Request<B>) -> Option<String> {
    if let Some(xff) = req.headers().get(XFF_HEADER)
        && let Ok(s) = xff.to_str()
        && let Some(client) = s.split(',').next()
    {
        let trimmed = client.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
}

/// Extract the tenant subject from request extensions populated by the
/// auth middleware. Returns `None` for routes mounted outside the
/// `Tenant`-injecting middleware (in which case the per-tenant limit
/// is a no-op — the per-IP limit still applies on the same route).
#[must_use]
pub fn tenant_key<B>(req: &Request<B>) -> Option<String> {
    req.extensions()
        .get::<Tenant>()
        .map(|t| t.subject.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use ministr_mcp::auth::Plan;

    fn empty_request() -> Request<()> {
        Request::builder().uri("/").body(()).unwrap()
    }

    #[test]
    fn ip_key_prefers_xff_over_connectinfo() {
        let mut req = empty_request();
        req.headers_mut()
            .insert("x-forwarded-for", "203.0.113.5, 10.0.0.1".parse().unwrap());
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(
            "127.0.0.1:443".parse().unwrap(),
        ));
        assert_eq!(ip_key(&req).as_deref(), Some("203.0.113.5"));
    }

    #[test]
    fn ip_key_falls_back_to_connectinfo() {
        let mut req = empty_request();
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(
            "192.0.2.7:54321".parse().unwrap(),
        ));
        assert_eq!(ip_key(&req).as_deref(), Some("192.0.2.7"));
    }

    #[test]
    fn ip_key_returns_none_when_no_signal_present() {
        let req = empty_request();
        assert!(ip_key(&req).is_none());
    }

    #[test]
    fn tenant_key_reads_from_extension() {
        let mut req = empty_request();
        req.extensions_mut().insert(Tenant {
            subject: "user-42".into(),
            org_id: None,
            plan: Plan::Pro,
        });
        assert_eq!(tenant_key(&req).as_deref(), Some("user-42"));
    }

    #[test]
    fn tenant_key_returns_none_without_extension() {
        let req = empty_request();
        assert!(tenant_key(&req).is_none());
    }

    #[test]
    fn ip_key_trims_empty_xff_entries() {
        let mut req = empty_request();
        req.headers_mut()
            .insert("x-forwarded-for", "   , 10.0.0.1".parse().unwrap());
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(
            "127.0.0.1:443".parse().unwrap(),
        ));
        // Falls back to ConnectInfo when XFF's first entry is blank.
        assert_eq!(ip_key(&req).as_deref(), Some("127.0.0.1"));
    }
}
