//! `restore()` manifest discipline (daemon-orphan-index-adoption):
//! a manifest entry whose source paths still exist must survive a
//! restart untouched, and a pruned dead-path entry must leave a
//! tombstone in `corpora.tombstones.json` — the manifest never loses
//! an entry silently.

use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_daemon::registry::CorpusRegistry;

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

#[tokio::test]
async fn live_entries_survive_restore_and_dead_entries_leave_tombstones() {
    let data_dir = tempfile::TempDir::new().unwrap();

    // A live source tree, stored under its canonical identity so restore
    // takes the plain path (no id migration).
    let source = tempfile::TempDir::new().unwrap();
    std::fs::write(source.path().join("readme.md"), "# live corpus\n\ntext").unwrap();
    let raw = vec![source.path().display().to_string()];
    let canonical = ministr_core::corpus_id::canonical_corpus_paths(&raw).unwrap();
    let live_id = ministr_core::corpus_id::corpus_id_from_paths(&canonical).unwrap();

    let manifest = serde_json::json!([
        { "id": live_id, "paths": canonical },
        { "id": "dead-zzz", "paths": ["/nonexistent/ministr-tombstone-test"] },
    ]);
    std::fs::write(
        data_dir.path().join("corpora.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let config = ministr_core::config::MinistrConfig {
        data_dir: data_dir.path().to_path_buf(),
        ..ministr_core::config::MinistrConfig::default()
    };
    let registry = Arc::new(CorpusRegistry::new(
        embedder,
        "mock-model:test".to_string(),
        config,
    ));
    registry.restore().await;

    // The live corpus is loaded — restore never drops a live entry.
    assert!(
        registry.corpora().read().await.contains_key(&live_id),
        "live corpus must be restored"
    );

    // Registration wrote the identity sidecar — if this dir is ever
    // orphaned, adoption can recover its source paths.
    let meta = data_dir
        .path()
        .join("corpora")
        .join(&live_id)
        .join("meta.toml");
    assert!(meta.exists(), "registered corpus dir must carry meta.toml");

    // The rewritten manifest keeps the live entry and drops the dead one.
    let manifest_after = std::fs::read_to_string(data_dir.path().join("corpora.json")).unwrap();
    assert!(
        manifest_after.contains(&live_id),
        "manifest must keep the live corpus"
    );
    assert!(
        !manifest_after.contains("dead-zzz"),
        "manifest must drop the dead entry"
    );

    // The prune left a tombstone — never a silent drop.
    let stones = std::fs::read_to_string(data_dir.path().join("corpora.tombstones.json")).unwrap();
    let stones: serde_json::Value = serde_json::from_str(&stones).unwrap();
    let ids: Vec<&str> = stones
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"dead-zzz"), "pruned entry must be tombstoned");
}
