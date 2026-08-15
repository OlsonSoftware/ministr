//! daemon-manifest-write-guard: `corpora.json` is single-writer and
//! merge-don't-truncate.
//!
//! The 2026-08-15 incident: a test-spawned daemon pointed at the real data
//! dir rewrote the shared manifest wholesale from its own tiny loaded set,
//! silently orphaning every registration it never loaded. Two structural
//! guarantees close the class:
//!
//! - **Merge-don't-truncate** — a writer preserves every on-disk entry it
//!   was never authoritative for (never loaded, registered, unregistered,
//!   or pruned), so even a lease-holding instance cannot drop foreign
//!   registrations.
//! - **Data-dir lease** — a second live instance on the same data dir
//!   holds no manifest lease and is refused at `save_manifest`: it cannot
//!   write `corpora.json` at all.

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

fn manifest_ids(data_dir: &std::path::Path) -> Vec<String> {
    let json = std::fs::read_to_string(data_dir.join("corpora.json")).unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    entries
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

/// The incident regression: a registry booted on a data dir whose manifest
/// holds entries it NEVER loaded (no `restore()` call — the "daemon B"
/// scenario) must not drop them when it saves its own view.
#[tokio::test]
async fn foreign_manifest_entries_survive_a_writer_that_never_loaded_them() {
    let data_dir = tempfile::TempDir::new().unwrap();

    // A manifest written by "another daemon": entries this registry never
    // loads. Their source paths don't exist — exactly the state the live
    // incident truncated.
    let foreign = serde_json::json!([
        {"id": "foreign-a", "paths": ["/nonexistent/ministr-guard-a"]},
        {"id": "foreign-b", "paths": ["/nonexistent/ministr-guard-b"]},
    ]);
    std::fs::write(
        data_dir.path().join("corpora.json"),
        serde_json::to_string_pretty(&foreign).unwrap(),
    )
    .unwrap();

    let registry = test_registry(data_dir.path());
    let (_source, paths) = source_tree();
    let (corpus_id, _) = registry.register(&paths).await.unwrap();

    let ids = manifest_ids(data_dir.path());
    assert!(
        ids.contains(&corpus_id),
        "the writer's own registration must be persisted; got {ids:?}"
    );
    for foreign_id in ["foreign-a", "foreign-b"] {
        assert!(
            ids.iter().any(|id| id == foreign_id),
            "{foreign_id} was never loaded by this instance and must survive its save; got {ids:?}"
        );
    }

    // Unregistering OUR corpus drops only our entry — the foreign ones
    // still survive (the writer is authoritative for its own id only).
    registry.unregister(&corpus_id).await.unwrap();
    let ids = manifest_ids(data_dir.path());
    assert!(
        !ids.contains(&corpus_id),
        "unregister must drop the writer's own entry; got {ids:?}"
    );
    for foreign_id in ["foreign-a", "foreign-b"] {
        assert!(
            ids.iter().any(|id| id == foreign_id),
            "{foreign_id} must survive an unregister of an unrelated corpus; got {ids:?}"
        );
    }
}

/// The lease direction: while instance A owns the data dir, instance B on
/// the same dir must be unable to write the manifest at all — registration
/// works in memory, but `corpora.json` keeps A's view byte-for-byte.
#[tokio::test]
async fn second_instance_on_a_held_data_dir_cannot_write_the_manifest() {
    let data_dir = tempfile::TempDir::new().unwrap();

    let reg_a = test_registry(data_dir.path());
    let (_source_a, paths_a) = source_tree();
    let (id_a, _) = reg_a.register(&paths_a).await.unwrap();
    let manifest_before = std::fs::read_to_string(data_dir.path().join("corpora.json")).unwrap();
    assert!(manifest_before.contains(&id_a));

    // Instance B on the same live data dir: no lease → manifest read-only.
    let reg_b = test_registry(data_dir.path());
    let (_source_b, paths_b) = source_tree();
    let (id_b, _) = reg_b
        .register(&paths_b)
        .await
        .expect("register stays usable in-memory on a read-only instance");

    let manifest_after = std::fs::read_to_string(data_dir.path().join("corpora.json")).unwrap();
    assert_eq!(
        manifest_before, manifest_after,
        "a lease-less instance must not change corpora.json"
    );

    // The verbs that REQUIRE persistence surface the refusal instead of
    // silently dropping state.
    let err = reg_b
        .unregister(&id_b)
        .await
        .expect_err("unregister must refuse when the manifest is read-only");
    assert!(
        err.to_string().contains("read-only"),
        "the refusal must say the manifest is read-only here; got: {err}"
    );

    // A still owns the dir and keeps full write access.
    reg_a.unregister(&id_a).await.unwrap();
    let ids = manifest_ids(data_dir.path());
    assert!(
        !ids.contains(&id_a),
        "the lease holder's unregister must still persist; got {ids:?}"
    );
}
