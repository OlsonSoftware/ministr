//! One writer per index dir (arch-index-ownership-lease): the daemon owns
//! a registered corpus's directory through an OS-level lease, an in-process
//! engine's identical acquire is refused while the daemon holds it, and the
//! refusal reverses cleanly in both directions.

use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::storage::{IndexLease, LeaseError};
use ministr_daemon::registry::{CorpusRegistry, RegistryError};

/// Deterministic mock embedder (same shape as the shared test harness's).
struct MockEmbedder {
    dim: usize,
}

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.bytes().enumerate() {
                    v[i % self.dim] += f32::from(b) / 255.0;
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

fn test_registry(data_dir: &std::path::Path) -> Arc<CorpusRegistry> {
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let config = ministr_core::config::MinistrConfig {
        data_dir: data_dir.to_path_buf(),
        ..ministr_core::config::MinistrConfig::default()
    };
    Arc::new(CorpusRegistry::new(
        embedder,
        "mock-model:test".to_string(),
        config,
    ))
}

fn source_tree() -> (tempfile::TempDir, Vec<String>) {
    let source = tempfile::TempDir::new().unwrap();
    std::fs::write(source.path().join("readme.md"), "# corpus\n\ntext").unwrap();
    let paths = vec![source.path().display().to_string()];
    (source, paths)
}

/// The two-writer regression: with the daemon's registry holding a corpus,
/// the in-process engine's acquire (the exact call the CLI's
/// `init_infrastructure` makes) must be refused — and must name the daemon.
#[tokio::test]
async fn in_process_open_refused_while_daemon_owns_the_dir() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let registry = test_registry(data_dir.path());
    let (_source, paths) = source_tree();

    let (corpus_id, _) = registry.register(&paths).await.unwrap();
    let corpus_dir = data_dir.path().join("corpora").join(&corpus_id);

    match IndexLease::acquire(&corpus_dir, "ministr command") {
        Err(LeaseError::Held { holder, .. }) => {
            assert!(
                holder.starts_with("ministr daemon, pid "),
                "the refusal must name the daemon; got: {holder}"
            );
        }
        other => panic!("second writer must be refused; got {other:?}"),
    }

    // Unregister drops the handle — and with it the lease: the dir is
    // free again for an in-process engine.
    registry.unregister(&corpus_id).await.unwrap();
    IndexLease::acquire(&corpus_dir, "ministr command").expect("dir must be free after unregister");
}

/// The reverse direction: while an in-process engine holds the lease, the
/// daemon's register refuses instead of double-writing — and recovers once
/// the holder is gone.
#[tokio::test]
async fn daemon_register_refused_while_in_process_engine_owns_the_dir() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let registry = test_registry(data_dir.path());
    let (_source, paths) = source_tree();

    let canonical = ministr_core::corpus_id::canonical_corpus_paths(&paths).unwrap();
    let corpus_id = ministr_core::corpus_id::corpus_id_from_paths(&canonical).unwrap();
    let corpus_dir = data_dir.path().join("corpora").join(&corpus_id);

    let cli_lease = IndexLease::acquire(&corpus_dir, "ministr command").unwrap();

    match registry.register(&paths).await {
        Err(RegistryError::Lease(LeaseError::Held { holder, .. })) => {
            assert!(
                holder.starts_with("ministr command, pid "),
                "the refusal must name the in-process holder; got: {holder}"
            );
        }
        other => panic!("daemon must refuse a held dir; got {other:?}"),
    }

    // The refused register must leave no half-registered state behind.
    assert!(
        !registry.corpora().read().await.contains_key(&corpus_id),
        "a refused register must not leave the corpus in the map"
    );

    drop(cli_lease);
    let (registered_id, _) = registry
        .register(&paths)
        .await
        .expect("register must succeed once the in-process holder is gone");
    assert_eq!(registered_id, corpus_id);
}
