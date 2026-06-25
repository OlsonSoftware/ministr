//! Session bundle export endpoint.
//!
//! Builds a deterministic `.tar` archive from a live [`SessionEntry`]
//! and ships it over `POST /api/v1/sessions/{id}/export`. Archive
//! layout:
//!
//! ```text
//! session-bundle.tar
//! ├── manifest.json      (session id, timings, counts — single JSON object)
//! └── delivered.jsonl    (one DeliveredItem per line, ordered by turn_delivered)
//! ```
//!
//! # Honest scope (v0)
//!
//! - **No `asked` events** — the `Session` shadow doesn't journal
//!   tool calls today; only the deliveries that produced content land
//!   in `delivered_items`. A future Session-side journal would land
//!   first; this module would then emit `asked.jsonl` alongside.
//! - **No `drops.jsonl`** — drops live in the Postgres
//!   `session_drops` ledger rather than in the in-memory `SessionEntry`.
//!   Querying the ledger here would make the bundle async-heavy; a
//!   future pass adds that integration once the wire shape is stable.
//! - **No blob storage / signed URL** — the route streams the tar
//!   back inline as `application/x-tar`. A later pass moves the
//!   artefact under `sessions/{tenant}/{id}/{ts}.tar` in blob storage
//!   and returns a 24h-TTL signed URL per the roadmap spec.
//! - **No Tauri "Session inspector" tab** — debugging today is via
//!   `curl POST /api/v1/sessions/{id}/export -o session.tar && tar tf
//!   session.tar`.
//!
//! Why `tar`, not `zip`: matches the existing `.ministr-index`
//! corpus-bundle convention (see `ministr-core/src/bundle.rs`); no
//! new compression dependency. The roadmap's `.session.zip` extension
//! becomes `.session.tar` in v0.

use std::io::Cursor;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ministr_api::{DropsLedger, SessionBundleStore, SessionBundleStoreError};
use ministr_core::session::{Session, SessionEntry, SessionRegistry, UsageTracker};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Snapshot of a [`SessionEntry`] as exported via
/// [`build_session_bundle`]. Field order is stable so cross-version
/// consumers can deserialise newer bundles with older code (future
/// fields land on the end). Mirrors the
/// [`ministr_api::SessionSnapshot`] shape but adds counts the
/// inspector UI will want at a glance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBundleManifest {
    /// Bundle schema version. v0 = 1.
    pub schema_version: u32,
    /// Session id as the agent presented it.
    pub session_id: String,
    /// ISO-8601 UTC timestamp the session was opened.
    pub opened_at: String,
    /// ISO-8601 UTC timestamp the bundle was generated.
    pub exported_at: String,
    /// Tokens this session has consumed (matches `entry.budget.usage_status().tokens_used`).
    pub budget_used: usize,
    /// Number of distinct content items delivered to the agent.
    pub delivered_count: usize,
    /// Sum of tokens across all delivered items.
    pub total_delivered_tokens: usize,
    /// Current pressure level — `"normal"`, `"elevated"`, or `"critical"`.
    pub pressure_level: String,
}

impl SessionBundleManifest {
    /// Build a manifest from the live entry. Pure — no I/O.
    #[must_use]
    pub fn from_entry(session_id: &str, entry: &SessionEntry) -> Self {
        let status = entry.budget.usage_status();
        Self {
            schema_version: 1,
            session_id: session_id.to_owned(),
            opened_at: iso8601_from_session(&entry.session),
            exported_at: crate::task::iso8601_now(),
            budget_used: status.tokens_used,
            delivered_count: entry.session.delivered_count(),
            total_delivered_tokens: entry.session.total_delivered_tokens(),
            pressure_level: pressure_label(status.level),
        }
    }
}

/// Derive an ISO-8601 string for the session's open time. The
/// `Session::created_at` is a [`std::time::Instant`] (monotonic, not
/// wall-clock) so we approximate by subtracting `elapsed` from `now`.
/// Good enough for the manifest — a future Session refactor that
/// captures a real wall-clock can swap this body without touching the
/// schema.
fn iso8601_from_session(session: &Session) -> String {
    let elapsed = session.elapsed();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs().saturating_sub(elapsed.as_secs());
    crate::task::iso8601_from_secs(secs)
}

fn pressure_label(level: ministr_core::session::UsageLevel) -> String {
    match level {
        ministr_core::session::UsageLevel::Normal => "normal".into(),
        ministr_core::session::UsageLevel::Elevated => "elevated".into(),
        ministr_core::session::UsageLevel::Critical => "critical".into(),
    }
}

/// Build the tar bytes for a session bundle (sync, drops-less).
///
/// Two entries: `manifest.json` (single JSON object) and
/// `delivered.jsonl` (one `DeliveredItem` JSON per line, ordered by
/// `turn_delivered` ascending for stable replay).
///
/// Use [`build_session_bundle_with_drops`] when a drops ledger is
/// wired and the bundle should also carry `drops.jsonl`.
///
/// # Errors
///
/// Returns `tar`/`serde_json` errors as a `String` — the only failure
/// modes are programmer bugs (manifest fails to serialise, the tar
/// builder hits an I/O fault on the in-memory buffer), so the caller
/// surfaces them as `500 internal error` rather than a structured
/// error envelope.
pub fn build_session_bundle(session_id: &str, entry: &SessionEntry) -> Result<Vec<u8>, String> {
    assemble_bundle_tar(session_id, entry, None)
}

/// Async wrapper that augments the bundle with `drops.jsonl` when a
/// [`DropsLedger`] backend is wired and a tenant is in scope.
///
/// Behaviour matrix:
/// - `ledger=Some` + `tenant_id=Some` → queries the ledger via
///   `list_for_session`; on success, appends `drops.jsonl` to the
///   tar (one [`ministr_api::DropEntry`] JSON per line, ledger's
///   own oldest-first order). On ledger error, logs at warn and
///   falls through to the no-drops shape so a Postgres hiccup
///   doesn't fail the export entirely.
/// - `ledger=None` or `tenant_id=None` → no `drops.jsonl` entry;
///   bundle shape matches [`build_session_bundle`].
///
/// # Errors
///
/// Same shape as [`build_session_bundle`] — only tar/serde failures
/// surface; ledger storage errors are swallowed (with a warn log).
pub async fn build_session_bundle_with_drops(
    session_id: &str,
    entry: &SessionEntry,
    ledger: Option<&dyn DropsLedger>,
    tenant_id: Option<&str>,
) -> Result<Vec<u8>, String> {
    let drops_bytes = match (ledger, tenant_id) {
        (Some(l), Some(tid)) => match l.list_for_session(tid, session_id).await {
            Ok(entries) => Some(serialize_drops_jsonl(&entries)?),
            Err(e) => {
                warn!(
                    error = ?e,
                    session_id = %session_id,
                    tenant_id = %tid,
                    "drops ledger list_for_session failed — exporting bundle without drops",
                );
                None
            }
        },
        _ => None,
    };
    assemble_bundle_tar(session_id, entry, drops_bytes.as_deref())
}

/// Internal — assemble the tar from already-serialised pieces. Shared
/// between the sync and async public APIs so the entry order +
/// header policy stay identical.
fn assemble_bundle_tar(
    session_id: &str,
    entry: &SessionEntry,
    drops_jsonl: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let manifest = SessionBundleManifest::from_entry(session_id, entry);
    let manifest_json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| format!("serialize manifest: {e}"))?;
    let delivered_jsonl = build_delivered_jsonl(&entry.session)?;

    let drops_len = drops_jsonl.map_or(0, <[u8]>::len);
    let mut buf: Vec<u8> =
        Vec::with_capacity(manifest_json.len() + delivered_jsonl.len() + drops_len + 4096);
    {
        let cursor = Cursor::new(&mut buf);
        let mut builder = tar::Builder::new(cursor);
        append_tar_entry(&mut builder, "manifest.json", &manifest_json)?;
        append_tar_entry(&mut builder, "delivered.jsonl", &delivered_jsonl)?;
        if let Some(bytes) = drops_jsonl {
            append_tar_entry(&mut builder, "drops.jsonl", bytes)?;
        }
        builder.finish().map_err(|e| format!("tar finish: {e}"))?;
    }
    Ok(buf)
}

/// Serialise a slice of [`ministr_api::DropEntry`] into `drops.jsonl`
/// bytes. Preserves the ledger's iteration order (oldest-first per
/// the trait contract).
fn serialize_drops_jsonl(entries: &[ministr_api::DropEntry]) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    for entry in entries {
        let line = serde_json::to_vec(entry).map_err(|e| format!("serialize drop: {e}"))?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

/// Build the `delivered.jsonl` body: one JSON-serialised
/// [`ministr_core::session::DeliveredItem`] per line, ordered by
/// `turn_delivered` ascending. Stable ordering matters for diff-able
/// replays.
fn build_delivered_jsonl(session: &Session) -> Result<Vec<u8>, String> {
    let mut items: Vec<&ministr_core::session::DeliveredItem> = session.delivered_items().collect();
    items.sort_by_key(|item| (item.turn_delivered, item.content_id.0.clone()));

    let mut out: Vec<u8> = Vec::new();
    for item in items {
        let line = serde_json::to_vec(item).map_err(|e| format!("serialize delivered: {e}"))?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

/// Append one in-memory blob to the tar builder. Single-purpose helper
/// so the bundle builder reads like a sequence of named entries.
fn append_tar_entry<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    path: &str,
    body: &[u8],
) -> Result<(), String> {
    let mut header = tar::Header::new_gnu();
    header
        .set_path(path)
        .map_err(|e| format!("tar header set_path {path}: {e}"))?;
    header.set_size(body.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append(&header, body)
        .map_err(|e| format!("tar append {path}: {e}"))?;
    Ok(())
}

/// State the export route needs — the shared session registry, an
/// optional drops ledger, and an optional bundle store. `cmd_serve_http`
/// threads the same `Arc<Mutex<SessionRegistry>>` through
/// `MinistrServer`; the export router clones that Arc. The ledger +
/// store are `None` on self-hosted serve (no Postgres / no blob
/// backend) and the bundle falls back to the inline-tar shape.
#[derive(Clone)]
pub struct SessionExportState {
    pub registry: Arc<Mutex<SessionRegistry>>,
    pub ledger: Option<Arc<dyn DropsLedger>>,
    pub bundle_store: Option<Arc<dyn SessionBundleStore>>,
}

impl SessionExportState {
    #[must_use]
    pub fn new(registry: Arc<Mutex<SessionRegistry>>) -> Self {
        Self {
            registry,
            ledger: None,
            bundle_store: None,
        }
    }

    /// Attach a drops ledger so `handle_export` augments the bundle with
    /// `drops.jsonl`. Cloud `cmd_serve_http` wires `PostgresDropsLedger`
    /// here when `cloud_pool` `is_some`; self-hosted leaves the field
    /// `None`.
    #[must_use]
    pub fn with_drops_ledger(mut self, ledger: Arc<dyn DropsLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Attach a bundle store so `handle_export` uploads the tar to blob
    /// storage and returns a signed URL (JSON) instead of streaming
    /// inline. Cloud `cmd_serve_http` wires `CloudSessionBundleStore`
    /// when the Azure account + signing secret are configured; otherwise
    /// the field stays `None` and the inline-tar shape continues to ship.
    #[must_use]
    pub fn with_bundle_store(mut self, store: Arc<dyn SessionBundleStore>) -> Self {
        self.bundle_store = Some(store);
        self
    }
}

/// Mount the session export routes. Paths mirror the roadmap spec:
/// - `POST /api/v1/sessions/{id}/export`: tar bundle, or JSON
///   `{url, expires_at}` when a bundle store is wired.
/// - `GET /api/v1/sessions`: list in-memory session summaries.
/// - `GET /api/v1/sessions/bundles/{*path}`: download a signed bundle.
///   Mounted unconditionally; returns 404 when no store is wired (the
///   route only resolves for clients that received a signed URL, which
///   only the upload path mints).
pub fn session_export_routes(state: SessionExportState) -> Router {
    Router::new()
        .route("/api/v1/sessions/{id}/export", post(handle_export))
        .route("/api/v1/sessions", axum::routing::get(handle_list))
        .route(
            "/api/v1/sessions/bundles/{*path}",
            axum::routing::get(handle_bundle_download),
        )
        .with_state(state)
}

/// One in-memory session summary returned by `GET /api/v1/sessions`.
/// Subset of [`SessionBundleManifest`] minus the export-time fields
/// (`exported_at`, `schema_version`) since this is a live snapshot, not
/// a packaged artefact. Future cross-pod listing will merge against
/// the `agent_sessions` Postgres table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub opened_at: String,
    pub budget_used: usize,
    pub delivered_count: usize,
    pub total_delivered_tokens: usize,
    pub pressure_level: String,
}

impl SessionSummary {
    /// Build a summary from a live entry. Mirrors [`SessionBundleManifest::from_entry`]
    /// but drops the export-time / schema-version fields.
    #[must_use]
    pub fn from_entry(session_id: &str, entry: &SessionEntry) -> Self {
        let status = entry.budget.usage_status();
        Self {
            session_id: session_id.to_owned(),
            opened_at: iso8601_from_session(&entry.session),
            budget_used: status.tokens_used,
            delivered_count: entry.session.delivered_count(),
            total_delivered_tokens: entry.session.total_delivered_tokens(),
            pressure_level: pressure_label(status.level),
        }
    }
}

/// Admit predicate keyed on the tenant scope and the entry's own
/// `tenant_id`. Shared between `handle_list` (filter the listing) and
/// `handle_export` (gate access to a specific id).
///
/// Behaviour matrix:
/// - `scope = None` (self-hosted / stdio): admit all entries.
/// - `scope = Some(t)`, `entry_tenant = Some(t)`: admit (match).
/// - `scope = Some(t)`, `entry_tenant = Some(other)`: deny.
/// - `scope = Some(t)`, `entry_tenant = None`: deny (pre-stamping
///   legacy entries are invisible to scoped callers — safe-by-default).
#[must_use]
fn admit_session_for_scope(scope: Option<&str>, entry_tenant: Option<&str>) -> bool {
    match scope {
        None => true,
        Some(s) => entry_tenant == Some(s),
    }
}

async fn handle_list(State(state): State<SessionExportState>) -> Response {
    // Read the tenant scope before locking so the filter is computed
    // once per request.
    let scope = crate::tenant_scope::current();
    let reg = state.registry.lock().await;
    let mut summaries: Vec<SessionSummary> = reg
        .session_ids()
        .into_iter()
        .filter_map(|id| {
            let entry = reg.get_session(&id)?;
            if admit_session_for_scope(scope.as_deref(), entry.tenant_id.as_deref()) {
                Some(SessionSummary::from_entry(&id, entry))
            } else {
                None
            }
        })
        .collect();
    drop(reg);
    // Stable order: most-recently-opened first. opened_at strings are
    // ISO-8601 UTC so lexical sort matches chronological order.
    summaries.sort_by(|a, b| {
        b.opened_at
            .cmp(&a.opened_at)
            .then(a.session_id.cmp(&b.session_id))
    });
    axum::Json(summaries).into_response()
}

async fn handle_export(
    State(state): State<SessionExportState>,
    Path(session_id): Path<String>,
) -> Response {
    // Read the tenant scope BEFORE locking the registry so we don't
    // hold the mutex across a `tenant_scope::current` call (cheap today
    // but keeps the lock window minimal).
    let tenant_id = crate::tenant_scope::current();

    let reg = state.registry.lock().await;
    let Some(entry) = reg.get_session(&session_id) else {
        drop(reg);
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };

    // Gate access by tenant. Cross-tenant fetches return 404
    // (existence-leak-resistant — mirrors the "session not found" shape
    // so an attacker can't enumerate id space across tenants).
    // Self-hosted (no scope) admits all; cloud admits only the calling
    // tenant's sessions. Pre-stamping legacy entries are invisible to
    // scoped callers.
    if !admit_session_for_scope(tenant_id.as_deref(), entry.tenant_id.as_deref()) {
        drop(reg);
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    }

    let bundle = match build_session_bundle_with_drops(
        &session_id,
        entry,
        state.ledger.as_deref().map(|l| l as &dyn DropsLedger),
        tenant_id.as_deref(),
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(e) => {
            drop(reg);
            warn!(error = %e, session_id = %session_id, "build_session_bundle_with_drops failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "build bundle failed").into_response();
        }
    };
    drop(reg);

    // When a bundle store is wired AND a tenant is in scope, upload to
    // blob and return JSON `{url, expires_at}`. Otherwise (self-hosted,
    // dev, or no signing secret configured) use the inline-tar shape.
    if let (Some(store), Some(tid)) = (state.bundle_store.as_deref(), tenant_id.as_deref()) {
        match store.put_and_sign(tid, &session_id, bundle).await {
            Ok(signed) => {
                return axum::Json(signed).into_response();
            }
            Err(e) => {
                warn!(
                    error = %e,
                    session_id = %session_id,
                    "bundle store upload failed — falling back to inline tar"
                );
                // Fall through: but we've moved `bundle` into
                // `put_and_sign`. Rebuild it for the inline path.
                let scope = tenant_id.clone();
                let reg = state.registry.lock().await;
                let Some(entry) = reg.get_session(&session_id) else {
                    drop(reg);
                    return (StatusCode::NOT_FOUND, "session not found").into_response();
                };
                let bundle = match build_session_bundle_with_drops(
                    &session_id,
                    entry,
                    state.ledger.as_deref().map(|l| l as &dyn DropsLedger),
                    scope.as_deref(),
                )
                .await
                {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        drop(reg);
                        warn!(error = %e, session_id = %session_id, "rebuild after store-fail");
                        return (StatusCode::INTERNAL_SERVER_ERROR, "build bundle failed")
                            .into_response();
                    }
                };
                drop(reg);
                return inline_tar_response(&session_id, bundle);
            }
        }
    }

    inline_tar_response(&session_id, bundle)
}

fn inline_tar_response(session_id: &str, bundle: Vec<u8>) -> Response {
    let filename = format!("session-{session_id}.tar");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    (
        [
            (header::CONTENT_TYPE, "application/x-tar".to_string()),
            (header::CONTENT_DISPOSITION, content_disposition),
        ],
        bundle,
    )
        .into_response()
}

/// `GET /api/v1/sessions/bundles/{*path}?expires=...&sig=...`.
///
/// The signed URL minted by `put_and_sign` points back at this handler.
/// We extract the path (everything after `/bundles/`) and the raw
/// query string (which carries `expires=…&sig=…`) and hand both to the
/// store's `verify_and_get`. The handler is intentionally tiny — all
/// the trust logic lives in the store.
async fn handle_bundle_download(
    State(state): State<SessionExportState>,
    Path(blob_path): Path<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let Some(store) = state.bundle_store.as_deref() else {
        // Self-hosted / no-store deployments don't mint signed URLs,
        // so a request landing here is malformed — surface 404 rather
        // than leaking the "store is None" mode shape.
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let token = query.unwrap_or_default();
    match store.verify_and_get(&blob_path, &token).await {
        Ok(bytes) => {
            let filename = blob_path
                .rsplit('/')
                .next()
                .unwrap_or("session.tar")
                .to_owned();
            let content_disposition = format!("attachment; filename=\"{filename}\"");
            (
                [
                    (header::CONTENT_TYPE, "application/x-tar".to_string()),
                    (header::CONTENT_DISPOSITION, content_disposition),
                ],
                bytes,
            )
                .into_response()
        }
        Err(SessionBundleStoreError::InvalidToken) => {
            (StatusCode::FORBIDDEN, "invalid or expired token").into_response()
        }
        Err(SessionBundleStoreError::NotFound) => {
            (StatusCode::NOT_FOUND, "bundle not found").into_response()
        }
        Err(SessionBundleStoreError::Storage(msg)) => {
            warn!(error = %msg, path = %blob_path, "bundle store download error");
            (StatusCode::INTERNAL_SERVER_ERROR, "bundle download failed").into_response()
        }
    }
}

// `UsageTracker` is imported into scope only so doc references resolve;
// the tracker is read indirectly via `SessionEntry::budget`.
#[allow(dead_code)]
fn _silence_unused(_: &UsageTracker) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_core::session::{AccessMode, UsageConfig};
    use ministr_core::types::{ContentId, Resolution};
    use std::io::Read;

    fn fresh_registry() -> SessionRegistry {
        SessionRegistry::new(UsageConfig::default())
    }

    fn extract_tar_entries(bytes: &[u8]) -> std::collections::HashMap<String, Vec<u8>> {
        let cursor = Cursor::new(bytes);
        let mut archive = tar::Archive::new(cursor);
        let mut map = std::collections::HashMap::new();
        for entry_result in archive.entries().expect("entries iter") {
            let mut entry = entry_result.expect("read entry");
            let path = entry
                .path()
                .expect("entry path")
                .to_string_lossy()
                .into_owned();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).expect("read body");
            map.insert(path, buf);
        }
        map
    }

    #[test]
    fn empty_session_bundle_has_manifest_and_empty_delivered() {
        let mut reg = fresh_registry();
        reg.create_session("agent-empty", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-empty").expect("entry");
        let bytes = build_session_bundle("agent-empty", entry).expect("build");
        let entries = extract_tar_entries(&bytes);
        assert!(
            entries.contains_key("manifest.json"),
            "tar should carry manifest.json",
        );
        assert!(
            entries.contains_key("delivered.jsonl"),
            "tar should carry delivered.jsonl even when empty",
        );
        assert!(
            entries["delivered.jsonl"].is_empty(),
            "empty session ⇒ zero-line delivered.jsonl",
        );

        let manifest: SessionBundleManifest =
            serde_json::from_slice(&entries["manifest.json"]).expect("parse manifest");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.session_id, "agent-empty");
        assert_eq!(manifest.delivered_count, 0);
        assert_eq!(manifest.total_delivered_tokens, 0);
        assert_eq!(manifest.budget_used, 0);
        assert_eq!(manifest.pressure_level, "normal");
        assert!(!manifest.opened_at.is_empty());
        assert!(!manifest.exported_at.is_empty());
    }

    #[test]
    fn session_with_two_deliveries_emits_two_jsonl_lines_in_turn_order() {
        let mut reg = fresh_registry();
        let entry = reg.create_session("agent-two", None, AccessMode::ReadWrite);

        // Two deliveries, on different turns; record in reverse turn
        // order so we can assert the bundle sorts by `turn_delivered`.
        entry.session.record_delivery(
            &ContentId("docs/b.md#y".to_string()),
            Resolution::Section,
            200,
            2,
            "hash-b".to_string(),
        );
        entry.session.record_delivery(
            &ContentId("docs/a.md#x".to_string()),
            Resolution::Section,
            100,
            1,
            "hash-a".to_string(),
        );
        let _ = entry.budget.record_tokens("docs/a.md#x", 100);
        let _ = entry.budget.record_tokens("docs/b.md#y", 200);

        let entry_ref = reg.get_session("agent-two").expect("entry");
        let bytes = build_session_bundle("agent-two", entry_ref).expect("build");
        let entries = extract_tar_entries(&bytes);
        let manifest: SessionBundleManifest =
            serde_json::from_slice(&entries["manifest.json"]).expect("parse manifest");

        assert_eq!(manifest.delivered_count, 2);
        assert_eq!(manifest.total_delivered_tokens, 300);
        assert_eq!(manifest.budget_used, 300);

        let body = String::from_utf8(entries["delivered.jsonl"].clone()).expect("utf8");
        let mut lines = body.lines();
        let first = lines.next().expect("line 1");
        let second = lines.next().expect("line 2");
        assert!(lines.next().is_none(), "exactly two lines");
        // turn_delivered=1 ⇒ docs/a.md#x sorts first.
        assert!(
            first.contains("\"content_id\":\"docs/a.md#x\""),
            "first line should be turn 1; got: {first}",
        );
        assert!(
            second.contains("\"content_id\":\"docs/b.md#y\""),
            "second line should be turn 2; got: {second}",
        );
    }

    #[test]
    fn manifest_schema_version_is_one_and_field_order_stable() {
        // Wire-shape regression guard: the inspector + future
        // import-side code key on `schema_version` to detect
        // breaking schema changes. v0 must stay at 1 until we
        // ship a v2 export with extra fields.
        let mut reg = fresh_registry();
        reg.create_session("agent-shape", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-shape").expect("entry");
        let manifest = SessionBundleManifest::from_entry("agent-shape", entry);
        assert_eq!(manifest.schema_version, 1);
        // Round-trip JSON to ensure derive(Serialize, Deserialize) stays
        // symmetric. Future field additions land on the end of the
        // struct so older crates keep parsing newer payloads.
        let json = serde_json::to_string(&manifest).expect("serialize");
        let parsed: SessionBundleManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, manifest);
    }

    // ── Drops integration tests ─────────────────────────────────────────

    use ministr_api::{AppendDropFuture, DropEntry, DropsLedgerError, ListDropsFuture};
    use std::sync::Mutex as StdMutex;

    /// Test-only ledger that returns a fixed slice of drops on
    /// `list_for_session`. Mirrors the `StubLedger` pattern from
    /// `registry.rs::tests` but exposes a `push` setter so the
    /// test can pre-seed the response.
    #[derive(Debug, Default)]
    struct StubDropsLedger {
        rows: StdMutex<Vec<DropEntry>>,
    }

    impl StubDropsLedger {
        fn push(&self, entry: DropEntry) {
            self.rows
                .lock()
                .expect("stub drops ledger mutex never poisoned")
                .push(entry);
        }
    }

    impl DropsLedger for StubDropsLedger {
        fn append<'a>(&'a self, entry: &'a DropEntry) -> AppendDropFuture<'a> {
            let owned = entry.clone();
            Box::pin(async move {
                self.rows
                    .lock()
                    .expect("stub drops ledger mutex never poisoned")
                    .push(owned);
                Ok::<(), DropsLedgerError>(())
            })
        }

        fn list_for_session<'a>(
            &'a self,
            tenant_id: &'a str,
            session_id: &'a str,
        ) -> ListDropsFuture<'a> {
            Box::pin(async move {
                let rows = self
                    .rows
                    .lock()
                    .expect("stub drops ledger mutex never poisoned");
                Ok(rows
                    .iter()
                    .filter(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                    .cloned()
                    .collect())
            })
        }
    }

    /// When both a ledger and a tenant scope are present, the bundle
    /// gains a `drops.jsonl` with one line per ledger entry.
    #[tokio::test]
    async fn bundle_includes_drops_when_ledger_wired_and_tenant_scoped() {
        let mut reg = fresh_registry();
        reg.create_session("agent-d", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-d").expect("entry");

        let ledger = Arc::new(StubDropsLedger::default());
        ledger.push(DropEntry {
            session_id: "agent-d".into(),
            tenant_id: "tenant-x".into(),
            claim_id: "docs/a.md#x".into(),
            evicted_at: "2026-05-21T00:00:00Z".into(),
        });
        ledger.push(DropEntry {
            session_id: "agent-d".into(),
            tenant_id: "tenant-x".into(),
            claim_id: "docs/b.md#y".into(),
            evicted_at: "2026-05-21T00:01:00Z".into(),
        });

        let bytes = build_session_bundle_with_drops(
            "agent-d",
            entry,
            Some(ledger.as_ref() as &dyn DropsLedger),
            Some("tenant-x"),
        )
        .await
        .expect("build with drops");
        let entries = extract_tar_entries(&bytes);
        assert!(
            entries.contains_key("drops.jsonl"),
            "tar must carry drops.jsonl when ledger+tenant present",
        );
        let body = String::from_utf8(entries["drops.jsonl"].clone()).expect("utf8");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "exactly two drops");
        assert!(lines[0].contains("\"claim_id\":\"docs/a.md#x\""));
        assert!(lines[1].contains("\"claim_id\":\"docs/b.md#y\""));
    }

    /// No ledger wired ⇒ bundle has no `drops.jsonl` entry.
    #[tokio::test]
    async fn bundle_omits_drops_when_no_ledger() {
        let mut reg = fresh_registry();
        reg.create_session("agent-no-ledger", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-no-ledger").expect("entry");

        let bytes =
            build_session_bundle_with_drops("agent-no-ledger", entry, None, Some("tenant-x"))
                .await
                .expect("build");
        let entries = extract_tar_entries(&bytes);
        assert!(entries.contains_key("manifest.json"));
        assert!(entries.contains_key("delivered.jsonl"));
        assert!(
            !entries.contains_key("drops.jsonl"),
            "no ledger ⇒ no drops.jsonl",
        );
    }

    /// Ledger wired but no tenant scope ⇒ bundle omits drops (can't
    /// look up by PK without a tenant id). Self-hosted serve lands here.
    #[tokio::test]
    async fn bundle_omits_drops_when_no_tenant_scope() {
        let mut reg = fresh_registry();
        reg.create_session("agent-no-tenant", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-no-tenant").expect("entry");
        let ledger = Arc::new(StubDropsLedger::default());

        let bytes = build_session_bundle_with_drops(
            "agent-no-tenant",
            entry,
            Some(ledger.as_ref() as &dyn DropsLedger),
            None,
        )
        .await
        .expect("build");
        let entries = extract_tar_entries(&bytes);
        assert!(
            !entries.contains_key("drops.jsonl"),
            "no tenant ⇒ no drops.jsonl",
        );
    }

    /// Ledger returns zero entries for the session ⇒ bundle still ships
    /// a `drops.jsonl` entry (just empty). Matches the shape of the
    /// `delivered.jsonl` empty case so the inspector can branch on
    /// file-present rather than file-content.
    #[tokio::test]
    async fn bundle_includes_empty_drops_jsonl_when_ledger_returns_zero() {
        let mut reg = fresh_registry();
        reg.create_session("agent-zero", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-zero").expect("entry");
        let ledger = Arc::new(StubDropsLedger::default());
        // No `push` — ledger returns Ok(vec![]).

        let bytes = build_session_bundle_with_drops(
            "agent-zero",
            entry,
            Some(ledger.as_ref() as &dyn DropsLedger),
            Some("tenant-x"),
        )
        .await
        .expect("build");
        let entries = extract_tar_entries(&bytes);
        assert!(
            entries.contains_key("drops.jsonl"),
            "ledger queried ⇒ drops.jsonl entry always present (even when empty)",
        );
        assert!(
            entries["drops.jsonl"].is_empty(),
            "no rows ⇒ zero-line drops.jsonl",
        );
    }

    // ── Session-list tests ──────────────────────────────────────────────

    #[test]
    fn session_summary_from_entry_carries_live_fields() {
        let mut reg = fresh_registry();
        let entry = reg.create_session("agent-s", None, AccessMode::ReadWrite);
        entry.session.record_delivery(
            &ContentId("docs/a.md#x".to_string()),
            Resolution::Section,
            150,
            1,
            "hash".to_string(),
        );
        let _ = entry.budget.record_tokens("docs/a.md#x", 150);

        let entry_ref = reg.get_session("agent-s").expect("entry");
        let summary = SessionSummary::from_entry("agent-s", entry_ref);
        assert_eq!(summary.session_id, "agent-s");
        assert_eq!(summary.budget_used, 150);
        assert_eq!(summary.delivered_count, 1);
        assert_eq!(summary.total_delivered_tokens, 150);
        assert_eq!(summary.pressure_level, "normal");
        assert!(!summary.opened_at.is_empty());
    }

    #[test]
    fn session_entry_default_tenant_id_is_none() {
        let mut reg = fresh_registry();
        reg.create_session("agent-default", None, AccessMode::ReadWrite);
        let entry = reg.get_session("agent-default").expect("entry");
        assert!(
            entry.tenant_id.is_none(),
            "create_session must default tenant_id to None — cloud stamps it post-create",
        );
    }

    /// `handle_list` filters entries by `tenant_scope::current`.
    /// `handle_list` + `handle_export` both delegate to
    /// [`admit_session_for_scope`]; this test exercises the predicate
    /// directly across the 4-corner matrix so both endpoints benefit
    /// from the same regression guard.
    #[test]
    fn admit_session_for_scope_matrix() {
        // Self-hosted (no scope) admits all entries regardless of stamp.
        assert!(admit_session_for_scope(None, None));
        assert!(admit_session_for_scope(None, Some("tenant-x")));
        // Scoped caller + matching entry tenant ⇒ admit.
        assert!(admit_session_for_scope(Some("tenant-x"), Some("tenant-x")));
        // Scoped caller + different tenant ⇒ deny.
        assert!(!admit_session_for_scope(Some("tenant-x"), Some("tenant-y")));
        // Scoped caller + unstamped entry ⇒ deny (safe-by-default).
        assert!(!admit_session_for_scope(Some("tenant-x"), None));
    }

    /// Confirms the `admit_session_for_scope` predicate produces the
    /// right session subset on a 3-entry registry.
    #[test]
    fn list_endpoint_filter_subsets_via_admit_helper() {
        let mut reg = fresh_registry();
        reg.create_session("sess-x", None, AccessMode::ReadWrite);
        reg.get_session_mut("sess-x").unwrap().tenant_id = Some("tenant-x".into());
        reg.create_session("sess-y", None, AccessMode::ReadWrite);
        reg.get_session_mut("sess-y").unwrap().tenant_id = Some("tenant-y".into());
        reg.create_session("sess-z", None, AccessMode::ReadWrite);
        // sess-z tenant_id stays None.

        let collect_under = |scope: Option<&str>| -> Vec<String> {
            reg.session_ids()
                .into_iter()
                .filter(|id| {
                    let tid = reg.get_session(id).unwrap().tenant_id.clone();
                    admit_session_for_scope(scope, tid.as_deref())
                })
                .collect()
        };

        assert_eq!(collect_under(None).len(), 3, "self-hosted ⇒ all visible");
        assert_eq!(collect_under(Some("tenant-x")), vec!["sess-x".to_string()]);
        assert_eq!(collect_under(Some("tenant-y")), vec!["sess-y".to_string()]);
        assert!(
            collect_under(Some("tenant-z")).is_empty(),
            "unknown tenant ⇒ no visibility, including unstamped legacy entries",
        );
    }

    #[test]
    fn session_summary_serde_round_trip() {
        // Wire-shape guard: the React inspector keys on these field
        // names. Renames must update the TS mirror in cloudClient.ts.
        let s = SessionSummary {
            session_id: "agent-1".into(),
            opened_at: "2026-05-21T00:00:00Z".into(),
            budget_used: 42,
            delivered_count: 3,
            total_delivered_tokens: 99,
            pressure_level: "normal".into(),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let parsed: SessionSummary = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, parsed);
    }

    // ── Bundle store tests ──────────────────────────────────────────────

    use ministr_api::{
        PutAndSignFuture, SessionBundleStore, SessionBundleStoreError, SignedBundleUrl,
        VerifyAndGetFuture,
    };

    /// Test-only bundle store. Captures uploads in-memory and round-
    /// trips a fixed signed URL. Mirrors the `StubStore` shape from
    /// `ministr_api::session_bundle_store::tests` but exposes the
    /// captured bytes for assertions.
    #[derive(Debug, Default)]
    struct StubBundleStore {
        captured: StdMutex<Vec<(String, String, Vec<u8>)>>, // (tenant, session, bytes)
        force_error: StdMutex<bool>,
    }

    impl StubBundleStore {
        fn force_error_on_put(&self) {
            *self.force_error.lock().expect("mutex") = true;
        }
        fn captures(&self) -> Vec<(String, String, Vec<u8>)> {
            self.captured.lock().expect("mutex").clone()
        }
    }

    impl SessionBundleStore for StubBundleStore {
        fn put_and_sign<'a>(
            &'a self,
            tenant_id: &'a str,
            session_id: &'a str,
            bytes: Vec<u8>,
        ) -> PutAndSignFuture<'a> {
            Box::pin(async move {
                if *self.force_error.lock().expect("mutex") {
                    return Err(SessionBundleStoreError::Storage("forced".into()));
                }
                self.captured.lock().expect("mutex").push((
                    tenant_id.to_owned(),
                    session_id.to_owned(),
                    bytes,
                ));
                Ok(SignedBundleUrl {
                    url: format!(
                        "https://example.test/api/v1/sessions/bundles/sessions/{tenant_id}/{session_id}/stub.tar?expires=1&sig=stub"
                    ),
                    expires_at: "2026-05-22T00:00:00Z".into(),
                })
            })
        }
        fn verify_and_get<'a>(
            &'a self,
            blob_path: &'a str,
            token: &'a str,
        ) -> VerifyAndGetFuture<'a> {
            Box::pin(async move {
                if token != "expires=1&sig=stub" {
                    return Err(SessionBundleStoreError::InvalidToken);
                }
                let cap = self.captured.lock().expect("mutex");
                cap.iter()
                    .find(|(t, s, _)| blob_path == format!("sessions/{t}/{s}/stub.tar"))
                    .map(|(_, _, b)| b.clone())
                    .ok_or(SessionBundleStoreError::NotFound)
            })
        }
    }

    /// Bundle-store wired + tenant scoped ⇒ `handle_export` uploads to
    /// the store rather than streaming inline. We exercise the upload
    /// via `put_and_sign` directly because spinning up axum +
    /// `scope_for_test` against a real request is heavier than the
    /// helper-call exercise itself; the dispatch-mode regression guard
    /// is the integration via `state.bundle_store.is_some()`.
    #[tokio::test]
    async fn put_and_sign_round_trips_through_dyn_store() {
        let store = Arc::new(StubBundleStore::default());
        let dyn_store: Arc<dyn SessionBundleStore> = Arc::clone(&store) as _;
        let bundle = vec![0u8, 1, 2, 3, 4];
        let signed = dyn_store
            .put_and_sign("tenant-x", "agent-x", bundle.clone())
            .await
            .expect("put_and_sign succeeds");
        assert!(signed.url.contains("sessions/tenant-x/agent-x/stub.tar"));
        assert!(signed.url.contains("expires="));
        assert!(signed.url.contains("sig="));
        let captures = store.captures();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].0, "tenant-x");
        assert_eq!(captures[0].1, "agent-x");
        assert_eq!(captures[0].2, bundle);
    }

    /// Store error ⇒ the upload path falls back to the inline shape.
    /// Exercises the `force_error` branch in the helper rather than the
    /// route; the route's else-arm just calls `inline_tar_response`.
    #[tokio::test]
    async fn put_and_sign_error_surfaces_to_caller() {
        let store = Arc::new(StubBundleStore::default());
        store.force_error_on_put();
        let dyn_store: Arc<dyn SessionBundleStore> = Arc::clone(&store) as _;
        let err = dyn_store
            .put_and_sign("tenant", "agent", vec![1, 2, 3])
            .await
            .expect_err("should fail");
        assert!(matches!(err, SessionBundleStoreError::Storage(_)));
    }

    /// `SessionExportState::with_bundle_store` attaches the store and
    /// `bundle_store.is_some()` flips. Regression guard for the builder
    /// chain.
    #[test]
    fn with_bundle_store_attaches_backend() {
        let reg = Arc::new(Mutex::new(fresh_registry()));
        let state = SessionExportState::new(reg);
        assert!(state.bundle_store.is_none());
        let store: Arc<dyn SessionBundleStore> = Arc::new(StubBundleStore::default()) as _;
        let state = state.with_bundle_store(store);
        assert!(state.bundle_store.is_some());
    }

    /// Verify path: a valid token round-trips the captured bytes; an
    /// invalid token surfaces `InvalidToken`.
    #[tokio::test]
    async fn verify_and_get_validates_token() {
        let store = Arc::new(StubBundleStore::default());
        let dyn_store: Arc<dyn SessionBundleStore> = Arc::clone(&store) as _;
        dyn_store
            .put_and_sign("t1", "s1", vec![9, 9, 9])
            .await
            .expect("put");
        let bytes = dyn_store
            .verify_and_get("sessions/t1/s1/stub.tar", "expires=1&sig=stub")
            .await
            .expect("verify ok");
        assert_eq!(bytes, vec![9, 9, 9]);
        let err = dyn_store
            .verify_and_get("sessions/t1/s1/stub.tar", "expires=1&sig=wrong")
            .await
            .expect_err("verify rejects");
        assert!(matches!(err, SessionBundleStoreError::InvalidToken));
    }
}
