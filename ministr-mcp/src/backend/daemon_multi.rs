//! [`DaemonMultiBackend`] — daemon-forwarding backend with per-call
//! project routing.
//!
//! Wraps a default [`DaemonBackend`] (the session's primary corpus) plus
//! a label-keyed map of linked-project backends. Every shared MCP tool
//! call optionally carries a `project: Option<&str>` argument; the
//! [`Backend`](super::Backend) enum's inherent methods resolve that
//! label via [`Self::for_project`] and dispatch to the right
//! single-corpus backend.

use std::collections::HashMap;
use std::sync::Arc;

use super::DaemonBackend;

/// A daemon-forwarding backend that knows about multiple corpora.
///
/// `default` is the session's primary corpus (the one resolved when no
/// `project` argument is passed). `linked` maps each `[[linked]] label =
/// "…"` from `.ministr.toml` to its own resolved `(corpus_id,
/// session_id)` bound into a separate [`DaemonBackend`].
pub struct DaemonMultiBackend {
    default: Arc<DaemonBackend>,
    linked: HashMap<String, Arc<DaemonBackend>>,
}

impl DaemonMultiBackend {
    #[must_use]
    pub fn new(default: Arc<DaemonBackend>, linked: HashMap<String, Arc<DaemonBackend>>) -> Self {
        Self { default, linked }
    }

    /// Return the sub-backend for `project`, or the default when `None`.
    ///
    /// Unknown labels also fall back to the default so an agent typo or
    /// stale tool argument doesn't make the call fail — the agent simply
    /// sees results from the primary corpus and can re-call with the
    /// correct label after consulting [`Self::labels`].
    #[must_use]
    pub fn for_project(&self, project: Option<&str>) -> &Arc<DaemonBackend> {
        match project {
            None => &self.default,
            Some(label) => self.linked.get(label).unwrap_or(&self.default),
        }
    }

    /// The configured linked-project labels, in declaration order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.linked.keys().cloned().collect()
    }

    /// Borrow the default backend (for `ministr_clone` and other
    /// operations that always target the session's primary corpus).
    #[must_use]
    pub fn default_backend(&self) -> &Arc<DaemonBackend> {
        &self.default
    }
}
