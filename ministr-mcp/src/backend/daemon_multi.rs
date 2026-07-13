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

use super::{BackendError, DaemonBackend};

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
    /// Unknown labels are rejected so a routed query can never silently
    /// return results from the primary corpus.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::UnknownProject`] for an unregistered label.
    pub fn for_project(&self, project: Option<&str>) -> Result<&Arc<DaemonBackend>, BackendError> {
        match project {
            None => Ok(&self.default),
            Some(label) => self
                .linked
                .get(label)
                .or_else(|| {
                    (self.default.corpus_id() == label)
                        .then_some(&self.default)
                        .or_else(|| {
                            self.linked
                                .values()
                                .find(|backend| backend.corpus_id() == label)
                        })
                })
                .ok_or_else(|| BackendError::UnknownProject(label.to_string())),
        }
    }

    /// The configured linked-project labels in deterministic lexical order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<_> = self.linked.keys().cloned().collect();
        labels.sort();
        labels
    }

    /// Borrow the default backend (for `ministr_clone` and other
    /// operations that always target the session's primary corpus).
    #[must_use]
    pub fn default_backend(&self) -> &Arc<DaemonBackend> {
        &self.default
    }

    /// Every routed backend, with the primary first and linked corpora in
    /// deterministic corpus-id order.
    #[must_use]
    pub fn all_backends(&self) -> Vec<&Arc<DaemonBackend>> {
        let mut linked: Vec<_> = self.linked.values().collect();
        linked.sort_by(|a, b| a.corpus_id().cmp(b.corpus_id()));
        std::iter::once(&self.default).chain(linked).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_api::client::DaemonClient;

    fn backend(corpus_id: &str) -> Arc<DaemonBackend> {
        Arc::new(DaemonBackend::new(
            Arc::new(DaemonClient::new()),
            corpus_id.to_string(),
            None,
        ))
    }

    #[test]
    fn routes_by_linked_label_or_durable_corpus_id() {
        let primary = backend("corpus-primary-uuid");
        let linked = backend("corpus-linked-uuid");
        let router = DaemonMultiBackend::new(
            Arc::clone(&primary),
            HashMap::from([("ministr".to_string(), Arc::clone(&linked))]),
        );

        assert!(Arc::ptr_eq(router.for_project(None).unwrap(), &primary));
        assert!(Arc::ptr_eq(
            router.for_project(Some("corpus-primary-uuid")).unwrap(),
            &primary
        ));
        assert!(Arc::ptr_eq(
            router.for_project(Some("ministr")).unwrap(),
            &linked
        ));
        assert!(Arc::ptr_eq(
            router.for_project(Some("corpus-linked-uuid")).unwrap(),
            &linked
        ));
        assert!(matches!(
            router.for_project(Some("unknown")),
            Err(BackendError::UnknownProject(_))
        ));
    }

    #[test]
    fn labels_are_deterministically_sorted() {
        let router = DaemonMultiBackend::new(
            backend("primary"),
            HashMap::from([
                ("zeta".to_string(), backend("z")),
                ("alpha".to_string(), backend("a")),
                ("middle".to_string(), backend("m")),
            ]),
        );
        assert_eq!(router.labels(), ["alpha", "middle", "zeta"]);
    }
}
