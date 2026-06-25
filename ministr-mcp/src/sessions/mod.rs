//! Session bundles and export surface.
//!
//! The bundle layer adapts a live in-memory [`SessionEntry`] into a
//! deterministic, portable archive an agent dev can download for
//! replay / inspection. The current scope is the export side; import,
//! signed-URL delivery, and a Tauri inspector are deferred.
//!
//! Lives in `ministr-mcp` because the `SessionRegistry` (the source
//! of truth for live session state) is held by `MinistrServer` here.
//! `cmd_serve_http` mounts the route by passing the shared
//! `Arc<Mutex<SessionRegistry>>` into [`export::session_export_routes`].
//!
//! [`SessionEntry`]: ministr_core::session::SessionEntry

pub mod export;

pub use export::{
    SessionBundleManifest, SessionExportState, SessionSummary, build_session_bundle,
    session_export_routes,
};
