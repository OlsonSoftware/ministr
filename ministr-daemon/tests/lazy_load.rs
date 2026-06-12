//! gd5 — `CorpusRegistry::get_or_lazy_load` warms a known-but-cold corpus
//! from the on-disk manifest, so a query that arrives before the daemon's
//! background `restore()` (or the proxy's backgrounded registration) has
//! loaded the corpus succeeds instead of 404ing.

use std::sync::Arc;

use ministr_core::config::MinistrConfig;
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_daemon::registry::CorpusRegistry;

/// Deterministic zero-vector embedder — the lazy-load path never embeds,
/// it just opens `SQLite` + loads/creates the (empty) `HNSW` index.
struct MockEmbedder {
    dim: usize,
}

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts.iter().map(|_| vec![0.0f32; self.dim]).collect())
    }
    fn dimension(&self) -> usize {
        self.dim
    }
}

#[tokio::test]
async fn get_or_lazy_load_warms_a_cold_corpus_from_the_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // A real source directory so registration has paths to record.
    let src = tmp.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("README.md"), "# hello\n").unwrap();
    let src_str = src.to_string_lossy().to_string();

    let config = MinistrConfig {
        data_dir: data_dir.clone(),
        ..MinistrConfig::default()
    };

    // Registry A registers the corpus — this writes the on-disk manifest
    // (corpora.json) + the corpus data dir.
    let embedder_a: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let reg_a = Arc::new(CorpusRegistry::new(
        embedder_a,
        "mock-model:test".to_string(),
        config.clone(),
    ));
    let (corpus_id, _started) = reg_a
        .register(std::slice::from_ref(&src_str))
        .await
        .unwrap();

    // Registry B points at the SAME data_dir but has a fresh, empty
    // in-memory map and never calls restore() — exactly the state a query
    // hits when it arrives before the corpus has been warmed.
    let embedder_b: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let reg_b = Arc::new(CorpusRegistry::new(
        embedder_b,
        "mock-model:test".to_string(),
        config,
    ));

    // A strict `get` misses — B never loaded it.
    assert!(
        reg_b.get(&corpus_id).await.is_err(),
        "cold corpus must not be in B's in-memory map"
    );

    // gd5: `get_or_lazy_load` resolves the paths from the manifest and
    // loads the corpus on demand.
    let handle = reg_b
        .get_or_lazy_load(&corpus_id)
        .await
        .expect("lazy load should warm the cold corpus");
    assert_eq!(handle.info.read().await.id, corpus_id);

    // It's now a warm fast-path hit.
    assert!(
        reg_b.get(&corpus_id).await.is_ok(),
        "corpus is warm after lazy load"
    );

    // An id that's in neither the map nor the manifest still 404s (no panic,
    // no spurious load).
    assert!(
        reg_b
            .get_or_lazy_load("sha256:deadbeefdeadbeef")
            .await
            .is_err(),
        "unknown corpus id resolves to NotFound"
    );
}

#[tokio::test]
async fn warming_clears_once_a_corpus_is_loaded() {
    // gd9 — `warming` is an in-flight signal: it's set while a corpus rebuilds
    // its index and cleared the moment the handle is in the map, so a
    // fully-registered corpus is never reported as warming and the mark never
    // leaks. (Sourcing it from the in-memory in-flight set — not the on-disk
    // manifest, which save_manifest rewrites from the loaded set mid-restore —
    // is what stops the GUI card from vanishing during the rebuild. The
    // mid-rebuild visibility itself is timing-dependent, verified empirically.)
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let src = tmp.path().join("project");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("README.md"), "# hi\n").unwrap();
    let src_str = src.to_string_lossy().to_string();

    let config = MinistrConfig {
        data_dir,
        ..MinistrConfig::default()
    };

    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let reg = Arc::new(CorpusRegistry::new(
        embedder,
        "mock-model:test".to_string(),
        config,
    ));

    // Nothing registered yet → nothing warming.
    assert!(reg.warming_placeholders().await.is_empty());

    let (corpus_id, _started) = reg.register(std::slice::from_ref(&src_str)).await.unwrap();

    // Loaded now → not warming (the in-flight mark was cleared, no leak).
    assert!(reg.get(&corpus_id).await.is_ok());
    assert!(
        reg.warming_placeholders().await.is_empty(),
        "a loaded corpus must not be reported as warming"
    );
}
