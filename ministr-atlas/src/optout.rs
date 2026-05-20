//! Repo opt-out registry.
//!
//! Any repo owner can opt out of Atlas inclusion. F2.6 v0 ships an
//! in-memory registry seeded from a (currently empty) static list;
//! F4.1 swaps in a Postgres-backed impl reading from a manual-
//! submission form at `ministr.ai/atlas/opt-out` plus the
//! `ai.robots.txt`-style check (`User-agent: ministr-atlas` /
//! `Disallow: /` in the repo's `robots.txt`).

use parking_lot::Mutex;
use std::collections::HashSet;

/// Decide whether `clone_url` is currently opted out of Atlas
/// inclusion. Implementations MUST be cheap to call — the weekly cron
/// queries this once per repo.
pub trait OptOutRegistry: Send + Sync + std::fmt::Debug {
    /// Returns `true` when the repo identified by `clone_url` has
    /// opted out. The cron skips opted-out repos and the cloud routes
    /// return 404 for them.
    fn is_opted_out(&self, clone_url: &str) -> bool;
}

/// In-memory opt-out registry. F2.6 v0 ships empty; the deployment
/// adds entries via the `add` method (e.g. an admin CLI subcommand
/// that pushes into the in-memory set, or a hot-reload of a TOML
/// file).
#[derive(Debug, Default)]
pub struct InMemoryRegistry {
    /// Lower-cased clone URLs the registry currently rejects.
    rejected: Mutex<HashSet<String>>,
}

impl InMemoryRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of clone URLs — useful for test
    /// fixtures or a TOML-loaded production registry.
    pub fn from_urls<I, S>(urls: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let set: HashSet<String> = urls
            .into_iter()
            .map(|s| s.into().to_lowercase())
            .collect();
        Self {
            rejected: Mutex::new(set),
        }
    }

    /// Add an opt-out entry. Idempotent — adding a URL twice is a
    /// no-op, matching the public registry's semantics.
    pub fn add(&self, clone_url: impl Into<String>) {
        self.rejected.lock().insert(clone_url.into().to_lowercase());
    }

    /// Remove an opt-out entry. The cron will pick the repo back up
    /// on the next run.
    pub fn remove(&self, clone_url: &str) {
        self.rejected.lock().remove(&clone_url.to_lowercase());
    }
}

impl OptOutRegistry for InMemoryRegistry {
    fn is_opted_out(&self, clone_url: &str) -> bool {
        self.rejected.lock().contains(&clone_url.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn empty_registry_admits_everything() {
        let r = InMemoryRegistry::new();
        assert!(!r.is_opted_out("https://github.com/anyone/anything"));
    }

    #[test]
    fn add_then_query_rejects() {
        let r = InMemoryRegistry::new();
        r.add("https://github.com/owner/repo");
        assert!(r.is_opted_out("https://github.com/owner/repo"));
    }

    #[test]
    fn case_insensitive_lookup() {
        let r = InMemoryRegistry::new();
        r.add("https://github.com/Owner/Repo");
        assert!(r.is_opted_out("https://github.com/owner/repo"));
    }

    #[test]
    fn remove_re_admits() {
        let r = InMemoryRegistry::new();
        r.add("https://github.com/owner/repo");
        r.remove("https://github.com/owner/repo");
        assert!(!r.is_opted_out("https://github.com/owner/repo"));
    }

    #[test]
    fn from_urls_seeds_initial_set() {
        let r = InMemoryRegistry::from_urls([
            "https://github.com/a/b",
            "https://github.com/c/d",
        ]);
        assert!(r.is_opted_out("https://github.com/a/b"));
        assert!(r.is_opted_out("https://github.com/c/d"));
        assert!(!r.is_opted_out("https://github.com/x/y"));
    }

    #[test]
    fn trait_is_dyn_compatible() {
        let r: Arc<dyn OptOutRegistry> = Arc::new(InMemoryRegistry::new());
        assert!(!r.is_opted_out("anything"));
    }
}
