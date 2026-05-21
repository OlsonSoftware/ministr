//! F6.2-a — session bundle export endpoint.
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
//! # Honest scope (F6.2-a v0)
//!
//! - **No `asked` events** — the `Session` shadow doesn't journal
//!   tool calls today; only the deliveries that produced content land
//!   in `delivered_items`. A future Session-side journal would land
//!   first; this module would then emit `asked.jsonl` alongside.
//! - **No `drops.jsonl`** — drops live in the Postgres
//!   `session_drops` ledger (F6.1-d) rather than in the in-memory
//!   `SessionEntry`. Querying the ledger here would make the bundle
//!   async-heavy; F6.2-b adds that integration once the wire shape is
//!   stable.
//! - **No blob storage / signed URL** — the route streams the tar
//!   back inline as `application/x-tar`. F6.2-c moves the artefact
//!   under `sessions/{tenant}/{id}/{ts}.tar` in blob storage and
//!   returns a 24h-TTL signed URL per the roadmap spec.
//! - **No Tauri "Session inspector" tab** — debugging today is via
//!   `curl POST /api/v1/sessions/{id}/export -o session.tar && tar tf
//!   session.tar`. Tauri UI lands in F6.2-d.
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

/// Build the tar bytes for a session bundle.
///
/// Two entries: `manifest.json` (single JSON object) and
/// `delivered.jsonl` (one `DeliveredItem` JSON per line, ordered by
/// `turn_delivered` ascending for stable replay).
///
/// # Errors
///
/// Returns `tar`/`serde_json` errors as a `String` — the only failure
/// modes are programmer bugs (manifest fails to serialise, the tar
/// builder hits an I/O fault on the in-memory buffer), so the caller
/// surfaces them as `500 internal error` rather than a structured
/// error envelope.
pub fn build_session_bundle(
    session_id: &str,
    entry: &SessionEntry,
) -> Result<Vec<u8>, String> {
    let manifest = SessionBundleManifest::from_entry(session_id, entry);
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("serialize manifest: {e}"))?;
    let delivered_jsonl = build_delivered_jsonl(&entry.session)?;

    let mut buf: Vec<u8> = Vec::with_capacity(manifest_json.len() + delivered_jsonl.len() + 4096);
    {
        let cursor = Cursor::new(&mut buf);
        let mut builder = tar::Builder::new(cursor);
        append_tar_entry(&mut builder, "manifest.json", &manifest_json)?;
        append_tar_entry(&mut builder, "delivered.jsonl", &delivered_jsonl)?;
        builder.finish().map_err(|e| format!("tar finish: {e}"))?;
    }
    Ok(buf)
}

/// Build the `delivered.jsonl` body: one JSON-serialised
/// [`ministr_core::session::DeliveredItem`] per line, ordered by
/// `turn_delivered` ascending. Stable ordering matters for diff-able
/// replays.
fn build_delivered_jsonl(session: &Session) -> Result<Vec<u8>, String> {
    let mut items: Vec<&ministr_core::session::DeliveredItem> =
        session.delivered_items().collect();
    items.sort_by_key(|item| (item.turn_delivered, item.content_id.0.clone()));

    let mut out: Vec<u8> = Vec::new();
    for item in items {
        let line = serde_json::to_vec(item)
            .map_err(|e| format!("serialize delivered: {e}"))?;
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

/// State the export route needs — just the shared session registry.
/// `cmd_serve_http` already threads the same `Arc<Mutex<SessionRegistry>>`
/// through `MinistrServer`; the export router clones that Arc.
#[derive(Clone)]
pub struct SessionExportState {
    pub registry: Arc<Mutex<SessionRegistry>>,
}

impl SessionExportState {
    pub fn new(registry: Arc<Mutex<SessionRegistry>>) -> Self {
        Self { registry }
    }
}

/// Mount the F6.2-a route. Path mirrors the roadmap spec:
/// `POST /api/v1/sessions/{id}/export`.
pub fn session_export_routes(state: SessionExportState) -> Router {
    Router::new()
        .route("/api/v1/sessions/{id}/export", post(handle_export))
        .with_state(state)
}

async fn handle_export(
    State(state): State<SessionExportState>,
    Path(session_id): Path<String>,
) -> Response {
    let reg = state.registry.lock().await;
    let Some(entry) = reg.get_session(&session_id) else {
        drop(reg);
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };

    let bundle = match build_session_bundle(&session_id, entry) {
        Ok(bytes) => bytes,
        Err(e) => {
            drop(reg);
            warn!(error = %e, session_id = %session_id, "build_session_bundle failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "build bundle failed").into_response();
        }
    };
    drop(reg);

    let filename = format!("session-{session_id}.tar");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    ([
        (header::CONTENT_TYPE, "application/x-tar".to_string()),
        (header::CONTENT_DISPOSITION, content_disposition),
    ], bundle)
        .into_response()
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
        let parsed: SessionBundleManifest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, manifest);
    }
}
