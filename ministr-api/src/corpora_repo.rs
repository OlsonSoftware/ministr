//! Durable registry of which corpora exist.
//!
//! [`CorporaRepo`] is the trait the daemon's [`CorpusRegistry`] consults
//! to remember registrations across process restarts. The local stack
//! ships no concrete implementation — self-hosted serve persists
//! registrations in a per-data-dir `corpora.json`, which is fine because
//! the user's disk is durable. Cloud deployments wire
//! `ministr_cloud::corpora_repo::PostgresCorporaRepo`, which moves the
//! list into Postgres so every replica and every pod restart sees the
//! same set of corpora.
//!
//! # Why `BoxFuture`, not fire-and-forget
//!
//! Diverges from the [`crate::BlobSink`] / [`crate::UsageSink`]
//! sync-fire-and-forget convention because `restore()` genuinely needs
//! the result of `list()`. The trait stays `dyn`-safe by returning
//! `Pin<Box<dyn Future + Send>>` from each method — same shape several
//! other repository traits in the workspace use, no `async_trait`
//! macro dep added to this crate.
//!
//! [`CorpusRegistry`]: ../../../ministr_daemon/registry/struct.CorpusRegistry.html

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// A single registered corpus row, as persisted by [`CorporaRepo`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusRegistration {
    /// Deterministic canonical corpus id (derived from the path set).
    pub corpus_id: String,
    /// Source paths the corpus was registered against. Stored as JSONB
    /// in Postgres; round-trips a `Vec<String>` here.
    pub paths: Vec<String>,
    /// Display name (user-facing). May be empty.
    pub display_name: Option<String>,
}

/// Errors surfaced by [`CorporaRepo`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum CorporaRepoError {
    /// Underlying storage failure — connection, SQL, serialization.
    #[error("storage: {0}")]
    Storage(String),
}

/// Convenience alias for the boxed futures every [`CorporaRepo`] method
/// returns. The `'a` parameter ties the future's lifetime to the
/// borrow of `&self` (and any borrowed args), so implementations can
/// capture references without cloning into a `'static` future.
pub type RepoFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, CorporaRepoError>> + Send + 'a>>;

/// Durable backing store for the [`CorpusRegistry`]'s list of known
/// corpora.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn CorporaRepo>` inside the registry's `OnceLock`. The trait
/// is `dyn`-safe: every method returns a boxed future.
///
/// [`CorpusRegistry`]: ../../../ministr_daemon/registry/struct.CorpusRegistry.html
pub trait CorporaRepo: Send + Sync + std::fmt::Debug {
    /// Insert or update a corpus registration. Idempotent — calling
    /// twice with the same `corpus_id` updates the row in place
    /// (paths and display name may legitimately change via
    /// `update_corpus_paths`).
    fn upsert<'a>(&'a self, entry: &'a CorpusRegistration) -> RepoFuture<'a, ()>;

    /// Remove a corpus registration. Idempotent — removing a row
    /// that does not exist is not an error.
    fn remove<'a>(&'a self, corpus_id: &'a str) -> RepoFuture<'a, ()>;

    /// List every registered corpus. Used by `CorpusRegistry::restore`
    /// at boot to repopulate the in-memory map.
    fn list(&self) -> RepoFuture<'_, Vec<CorpusRegistration>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockRepo {
        rows: Mutex<Vec<CorpusRegistration>>,
    }

    impl CorporaRepo for MockRepo {
        fn upsert<'a>(&'a self, entry: &'a CorpusRegistration) -> RepoFuture<'a, ()> {
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                if let Some(slot) = rows.iter_mut().find(|r| r.corpus_id == entry.corpus_id) {
                    *slot = entry.clone();
                } else {
                    rows.push(entry.clone());
                }
                Ok(())
            })
        }

        fn remove<'a>(&'a self, corpus_id: &'a str) -> RepoFuture<'a, ()> {
            Box::pin(async move {
                self.rows
                    .lock()
                    .unwrap()
                    .retain(|r| r.corpus_id != corpus_id);
                Ok(())
            })
        }

        fn list(&self) -> RepoFuture<'_, Vec<CorpusRegistration>> {
            Box::pin(async move { Ok(self.rows.lock().unwrap().clone()) })
        }
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible() {
        let repo: std::sync::Arc<dyn CorporaRepo> = std::sync::Arc::new(MockRepo::default());
        repo.upsert(&CorpusRegistration {
            corpus_id: "c1".into(),
            paths: vec!["/tmp/a".into()],
            display_name: Some("a".into()),
        })
        .await
        .unwrap();
        let rows = repo.list().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].corpus_id, "c1");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_remove_is_idempotent() {
        let repo = MockRepo::default();
        let entry = CorpusRegistration {
            corpus_id: "c1".into(),
            paths: vec!["/tmp/a".into()],
            display_name: None,
        };
        repo.upsert(&entry).await.unwrap();
        repo.upsert(&entry).await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 1);

        repo.remove("c1").await.unwrap();
        repo.remove("c1").await.unwrap();
        assert!(repo.list().await.unwrap().is_empty());
    }
}
