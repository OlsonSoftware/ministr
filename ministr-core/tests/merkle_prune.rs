//! F35 — dirty-subtree merkle pruning integration tests.
//!
//! Exercises the multi-path ingest surface (the one the daemon and CLI
//! multi-path drive): after a first index persists the per-directory
//! stat-fingerprint tree, a reindex with one changed file must prune every
//! file in unchanged directories BEFORE the per-file hash-lookup path
//! (`files_pruned` counter), while the changed file's directory still goes
//! through the per-file mtime+hash backstop. The tree must auto-heal when
//! absent and be rebuilt after every successful ingest.

use std::path::{Path, PathBuf};

use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::{SqliteStorage, Storage};

/// Deterministic mock embedder for integration tests.
struct MockEmbedder {
    dim: usize,
}

impl ministr_core::embedding::Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, ministr_core::error::IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.bytes().enumerate() {
                    v[i % self.dim] += f32::from(b) / 255.0;
                }
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// Five markdown files across three directories: `a/`, `b/`, and the root.
fn make_corpus(root: &Path) {
    write(&root.join("a/f1.md"), "# A1\n\nAlpha one body.\n");
    write(&root.join("a/f2.md"), "# A2\n\nAlpha two body.\n");
    write(&root.join("b/f3.md"), "# B3\n\nBravo three body.\n");
    write(&root.join("b/f4.md"), "# B4\n\nBravo four body.\n");
    write(&root.join("top.md"), "# Top\n\nRoot-level body.\n");
}

struct Harness {
    storage: SqliteStorage,
    embedder: MockEmbedder,
    index: HnswIndex,
    pipeline: IngestionPipeline,
    paths: Vec<PathBuf>,
}

impl Harness {
    fn new(corpus: &Path) -> Self {
        let dim = 8;
        Self {
            storage: SqliteStorage::open_in_memory().unwrap(),
            embedder: MockEmbedder { dim },
            index: HnswIndex::new(dim, 10_000).unwrap(),
            pipeline: IngestionPipeline::new(),
            paths: vec![corpus.to_path_buf()],
        }
    }

    async fn ingest(&self) -> ministr_core::ingestion::IngestionStats {
        self.pipeline
            .ingest_paths_with_embeddings(&self.paths, &self.storage, &self.embedder, &self.index)
            .await
            .unwrap()
    }

    async fn root_id(&self) -> String {
        let roots = self.storage.list_corpus_roots().await.unwrap();
        assert_eq!(roots.len(), 1, "expected exactly one registered root");
        roots[0].id.clone()
    }
}

#[tokio::test]
async fn one_changed_file_prunes_unchanged_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    make_corpus(&corpus);
    let h = Harness::new(&corpus);

    // First index: no stored tree yet — auto-heal path, nothing pruned.
    let stats = h.ingest().await;
    assert_eq!(stats.files_discovered, 5);
    assert_eq!(stats.files_indexed, 5);
    assert_eq!(
        stats.files_pruned, 0,
        "first index has no tree to prune with"
    );

    // The tree must have been persisted for the next run.
    let rid = h.root_id().await;
    let nodes = h.storage.get_corpus_merkle_nodes(&rid).await.unwrap();
    assert_eq!(nodes.len(), 3, "expected nodes for '', 'a', 'b'");

    // Change ONE file (different size so the stat fingerprint must move
    // regardless of filesystem mtime granularity).
    write(
        &corpus.join("a/f1.md"),
        "# A1\n\nAlpha one body, now substantially revised and longer.\n",
    );

    // Reindex: dirs `b/` and the root are provably unchanged → their 3
    // files are pruned before the per-file hash path. Dir `a/` is dirty:
    // f1 re-indexes, f2 goes through the per-file backstop and hash-skips.
    let stats = h.ingest().await;
    assert_eq!(stats.files_pruned, 3, "b/f3, b/f4, top.md must be pruned");
    assert_eq!(stats.files_indexed, 1, "only the changed file re-indexes");
    assert_eq!(
        stats.files_skipped, 4,
        "3 pruned + 1 hash-skipped sibling in the dirty directory"
    );
    assert_eq!(stats.files_failed, 0);
}

#[tokio::test]
async fn absent_tree_auto_heals_and_rebuilds() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    make_corpus(&corpus);
    let h = Harness::new(&corpus);

    h.ingest().await;
    let rid = h.root_id().await;

    // Wipe the tree (simulates a pre-F35 database or a cleared repair
    // state) and change a file so the mtime fast-skip doesn't trigger.
    h.storage
        .replace_corpus_merkle_nodes(&rid, &[])
        .await
        .unwrap();
    write(
        &corpus.join("b/f3.md"),
        "# B3\n\nBravo three body, revised after the tree was wiped.\n",
    );

    // No tree → pruning silently disabled; ingest stays correct via the
    // per-file backstop...
    let stats = h.ingest().await;
    assert_eq!(stats.files_pruned, 0, "absent tree must disable pruning");
    assert_eq!(stats.files_indexed, 1);
    assert_eq!(stats.files_failed, 0);

    // ...and the tree rebuilt itself without any manual wipe/rebuild step.
    let nodes = h.storage.get_corpus_merkle_nodes(&rid).await.unwrap();
    assert_eq!(
        nodes.len(),
        3,
        "tree must auto-heal after a successful ingest"
    );
}

#[tokio::test]
async fn unchanged_corpus_reindex_skips_all_files() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    make_corpus(&corpus);
    let h = Harness::new(&corpus);

    h.ingest().await;

    // Nothing changed: the whole reindex short-circuits (manifest-level
    // fast-skip) — zero files re-indexed, zero content re-hashes.
    let stats = h.ingest().await;
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_skipped, 5);
    assert_eq!(stats.files_failed, 0);
}
