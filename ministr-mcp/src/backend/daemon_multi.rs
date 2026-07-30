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
    primary_label: super::PrimaryLabel,
}

impl DaemonMultiBackend {
    #[must_use]
    pub fn new(default: Arc<DaemonBackend>, linked: HashMap<String, Arc<DaemonBackend>>) -> Self {
        Self {
            default,
            linked,
            primary_label: super::PrimaryLabel::default(),
        }
    }

    /// The current project's own label — routed to [`Self::default`] just
    /// like `project: None`. A linked entry always wins over it, so an
    /// explicit `[[linked]] label` can never be shadowed.
    #[must_use]
    pub fn primary_label(&self) -> &super::PrimaryLabel {
        &self.primary_label
    }

    /// Whether `label` names the primary corpus (by label or corpus id).
    #[must_use]
    pub fn route_is_primary(&self, label: &str) -> bool {
        self.primary_label.matches(label) || self.default.corpus_id() == label
    }

    /// Every route this router accepts: the current project's label (when
    /// known) followed by the linked labels in lexical order.
    #[must_use]
    pub fn available_routes(&self) -> Vec<String> {
        let mut routes: Vec<String> = self
            .primary_label
            .get()
            .map(|label| vec![label.to_string()])
            .unwrap_or_default();
        routes.extend(self.labels());
        routes
    }

    /// Return the sub-backend for `project`, or the default when `None`.
    /// Unknown labels are rejected so a routed query can never silently
    /// return results from the primary corpus.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::UnknownProject`] for an unregistered label,
    /// carrying the routes that would have worked.
    pub fn for_project(&self, project: Option<&str>) -> Result<&Arc<DaemonBackend>, BackendError> {
        match project {
            None => Ok(&self.default),
            Some(label) => self
                .linked
                .get(label)
                .or_else(|| {
                    self.route_is_primary(label)
                        .then_some(&self.default)
                        .or_else(|| {
                            self.linked
                                .values()
                                .find(|backend| backend.corpus_id() == label)
                        })
                })
                .ok_or_else(|| BackendError::UnknownProject {
                    requested: label.to_string(),
                    available: self.available_routes(),
                }),
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
            Err(BackendError::UnknownProject { .. })
        ));
    }

    #[test]
    fn routes_the_current_projects_own_label_to_the_primary() {
        let primary = backend("corpus-primary-uuid");
        let linked = backend("corpus-linked-uuid");
        let router = DaemonMultiBackend::new(
            Arc::clone(&primary),
            HashMap::from([("ministr".to_string(), Arc::clone(&linked))]),
        );

        // Before the label is known, naming the project still fails —
        // strictness is preserved for label-less sessions.
        assert!(matches!(
            router.for_project(Some("kadodi")),
            Err(BackendError::UnknownProject { .. })
        ));

        router.primary_label().set("kadodi");

        assert!(Arc::ptr_eq(
            router.for_project(Some("kadodi")).unwrap(),
            &primary
        ));
        // Directory casing is not something an agent can be expected to
        // reproduce exactly.
        assert!(Arc::ptr_eq(
            router.for_project(Some("Kadodi")).unwrap(),
            &primary
        ));
        // A genuinely unknown label must still fail — otherwise an agent
        // that thinks it queried another repo silently gets this one.
        assert!(matches!(
            router.for_project(Some("ministr-private")),
            Err(BackendError::UnknownProject { .. })
        ));
    }

    #[test]
    fn an_explicit_linked_label_wins_over_the_primary_label() {
        let primary = backend("corpus-primary-uuid");
        let linked = backend("corpus-linked-uuid");
        let router = DaemonMultiBackend::new(
            Arc::clone(&primary),
            // Pathological but possible: a linked entry labelled with the
            // same name as the project doing the linking.
            HashMap::from([("kadodi".to_string(), Arc::clone(&linked))]),
        );
        router.primary_label().set("kadodi");

        assert!(Arc::ptr_eq(
            router.for_project(Some("kadodi")).unwrap(),
            &linked
        ));
    }

    #[test]
    fn unknown_route_error_carries_the_routes_that_would_have_worked() {
        let router = DaemonMultiBackend::new(
            backend("corpus-primary-uuid"),
            HashMap::from([
                ("zeta".to_string(), backend("z")),
                ("alpha".to_string(), backend("a")),
            ]),
        );
        router.primary_label().set("kadodi");

        let Err(BackendError::UnknownProject {
            requested,
            available,
        }) = router.for_project(Some("typo"))
        else {
            panic!("expected an unknown-route error");
        };
        assert_eq!(requested, "typo");
        assert_eq!(available, ["kadodi", "alpha", "zeta"]);
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
