//! Integration test: multi-path code ingestion with `.rs` files.
//!
//! Verifies that the ingestion pipeline discovers and indexes Rust source
//! files from multiple corpus paths, producing non-zero document and section
//! counts with embeddings in the vector index.

use std::path::PathBuf;

use ministr_core::index::{HnswIndex, VectorIndex};
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

#[tokio::test]
async fn multi_path_code_ingestion_indexes_rs_files() {
    let tmp = tempfile::tempdir().unwrap();

    // Create two separate "crate" directories with .rs files
    let crate_a = tmp.path().join("crate-a/src");
    let crate_b = tmp.path().join("crate-b/src");
    std::fs::create_dir_all(&crate_a).unwrap();
    std::fs::create_dir_all(&crate_b).unwrap();

    std::fs::write(
        crate_a.join("lib.rs"),
        r#"//! Crate A library.

/// A public struct.
pub struct Config {
    pub name: String,
    pub port: u16,
}

impl Config {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self {
            name: String::from("default"),
            port: 8080,
        }
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        crate_b.join("main.rs"),
        r#"//! Crate B binary.

fn main() {
    let config = crate_a::Config::new();
    println!("Starting {} on {}", config.name, config.port);
}
"#,
    )
    .unwrap();

    // Also add a markdown doc to verify mixed corpus types work
    let docs_dir = tmp.path().join("docs");
    std::fs::create_dir_all(&docs_dir).unwrap();
    std::fs::write(
        docs_dir.join("guide.md"),
        "# Getting Started\n\nInstall the tool and run it.\n\n## Configuration\n\nEdit config.toml.\n",
    )
    .unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();

    let pipeline = IngestionPipeline::new();
    let paths: Vec<PathBuf> = vec![crate_a, crate_b, docs_dir];

    let stats = pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();

    // Should discover: lib.rs, main.rs, guide.md = 3 files
    assert_eq!(
        stats.files_discovered, 3,
        "expected 3 files (2 .rs + 1 .md)"
    );
    assert_eq!(stats.files_indexed, 3, "all 3 files should be indexed");
    assert_eq!(stats.files_failed, 0, "no files should fail");
    assert!(
        stats.total_sections > 0,
        "should produce sections from code and docs"
    );
    assert!(index.len() > 0, "vector index should contain embeddings");
}

#[tokio::test]
async fn individual_file_and_directory_mixed_paths() {
    let tmp = tempfile::tempdir().unwrap();

    // A directory with code
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "//! A module.\n\npub fn hello() -> &'static str { \"hello\" }\n",
    )
    .unwrap();

    // An individual markdown file (not in a directory)
    let design_file = tmp.path().join("DESIGN.md");
    std::fs::write(&design_file, "# Design\n\nThe architecture is layered.\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();

    let pipeline = IngestionPipeline::new();
    let paths: Vec<PathBuf> = vec![src_dir, design_file];

    let stats = pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();

    assert_eq!(
        stats.files_discovered, 2,
        "expected 2 files (1 .rs dir + 1 .md file)"
    );
    assert_eq!(stats.files_indexed, 2);
    assert_eq!(stats.files_failed, 0);
}

/// Orphan GC: a file deleted out-of-band (mirrors a `git rm`, branch
/// switch, or a crate moved to another repo — exactly the F31 split) must
/// have its document, sections, vectors and `file_hashes` row pruned on
/// the next reindex, not survive forever.
#[tokio::test]
async fn reindex_prunes_orphaned_document_after_file_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("keep.rs"), "//! keep\npub fn keep() {}\n").unwrap();
    std::fs::write(src.join("gone.rs"), "//! gone\npub fn gone() {}\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();
    let paths: Vec<PathBuf> = vec![src.clone()];

    pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();
    assert_eq!(
        storage.document_count().await.unwrap(),
        2,
        "both files should be indexed initially"
    );

    // Delete one file out-of-band — no watcher event fires for this.
    std::fs::remove_file(src.join("gone.rs")).unwrap();

    let stats = pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();

    assert_eq!(
        stats.files_removed, 1,
        "the deleted file's document should be pruned on reindex"
    );
    assert_eq!(
        storage.document_count().await.unwrap(),
        1,
        "only keep.rs should remain"
    );

    let docs = storage.list_documents().await.unwrap();
    assert!(
        docs.iter().all(|d| !d.source_path.contains("gone.rs")),
        "the orphaned document for gone.rs must be deleted"
    );
    assert!(
        docs.iter().any(|d| d.source_path.contains("keep.rs")),
        "keep.rs must survive the sweep"
    );

    let hashes = storage.list_file_hashes().await.unwrap();
    assert!(
        hashes.iter().all(|h| !h.path.contains("gone.rs")),
        "the orphan's file_hashes row must be cleared so it can't suppress a future re-ingest"
    );
}

/// Safety guard: an empty discovery (a transient unreadable / unmounted
/// root) must NOT be mistaken for an emptied corpus. Without the guard the
/// cleanup loop deletes every document in the index.
#[tokio::test]
async fn empty_discovery_does_not_wipe_the_index() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.rs"), "//! a\npub fn a() {}\n").unwrap();
    std::fs::write(src.join("b.rs"), "//! b\npub fn b() {}\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();

    pipeline
        .ingest_paths_with_embeddings(std::slice::from_ref(&src), &storage, &embedder, &index)
        .await
        .unwrap();
    assert_eq!(storage.document_count().await.unwrap(), 2);

    // Reindex pointing at a root that currently yields 0 files — the stand-in
    // for an unreadable / unmounted root. The existing index must survive.
    let empty = tmp.path().join("empty-root");
    std::fs::create_dir_all(&empty).unwrap();
    let stats = pipeline
        .ingest_paths_with_embeddings(&[empty], &storage, &embedder, &index)
        .await
        .unwrap();

    assert_eq!(
        stats.files_removed, 0,
        "an empty discovery must not prune any documents"
    );
    assert_eq!(
        storage.document_count().await.unwrap(),
        2,
        "the index must survive a reindex whose discovery returned 0 files"
    );
}

/// F32.1 — a NULL-root document (ingested through the unrooted entry point,
/// by the CLI/worker, or before roots existed) must be reclaimed when its
/// file is deleted and the corpus is later reindexed through the *rooted*
/// path. The old per-root `list_documents_by_root(rid)` scan never saw
/// NULL-root docs, so they orphaned forever.
#[tokio::test]
async fn rooted_reindex_prunes_null_root_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("keep.rs"), "//! keep\npub fn keep() {}\n").unwrap();
    std::fs::write(src.join("gone.rs"), "//! gone\npub fn gone() {}\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();

    // Initial ingest through the *unrooted* entry point → documents are
    // stored with root_id = NULL and bare-relative source paths.
    pipeline
        .ingest_directory_with_embeddings(&src, &storage, &embedder, &index)
        .await
        .unwrap();
    let docs = storage.list_documents().await.unwrap();
    assert_eq!(docs.len(), 2, "both files indexed initially");
    assert!(
        docs.iter().all(|d| d.root_id.is_none()),
        "the unrooted entry point must leave root_id NULL"
    );

    // Delete one file out-of-band, then reindex through the ROOTED path.
    std::fs::remove_file(src.join("gone.rs")).unwrap();
    let rid = "root-000000000000000a";
    let stats = pipeline
        .ingest_directory_with_embeddings_rooted(&src, &storage, &embedder, &index, Some(rid), None)
        .await
        .unwrap();

    assert_eq!(
        stats.files_removed, 1,
        "the deleted file's NULL-root document must be pruned by the rooted sweep"
    );
    let docs = storage.list_documents().await.unwrap();
    assert!(
        docs.iter().all(|d| !d.source_path.contains("gone.rs")),
        "the orphaned NULL-root document for gone.rs must be deleted"
    );
    assert!(
        docs.iter().any(|d| d.source_path.contains("keep.rs")),
        "keep.rs must survive the sweep"
    );
}

/// F32.1 — the global sweep scopes deletion by ownership: reindexing one
/// root may prune that root's own orphan but must NEVER delete a sibling
/// root's documents sharing the same (multi-root) index. Without the scope
/// guard a naive global diff would wipe every other root on each reindex.
#[tokio::test]
async fn rooted_reindex_preserves_sibling_root_documents() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("a");
    let dir_b = tmp.path().join("b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(dir_a.join("keep_a.rs"), "//! a\npub fn keep_a() {}\n").unwrap();
    std::fs::write(dir_a.join("gone_a.rs"), "//! a\npub fn gone_a() {}\n").unwrap();
    std::fs::write(dir_b.join("b.rs"), "//! b\npub fn b() {}\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();

    let rid_a = "root-000000000000000a";
    let rid_b = "root-000000000000000b";
    pipeline
        .ingest_directory_with_embeddings_rooted(
            &dir_a,
            &storage,
            &embedder,
            &index,
            Some(rid_a),
            None,
        )
        .await
        .unwrap();
    pipeline
        .ingest_directory_with_embeddings_rooted(
            &dir_b,
            &storage,
            &embedder,
            &index,
            Some(rid_b),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        storage.document_count().await.unwrap(),
        3,
        "two root-A files + one root-B file indexed across two roots"
    );

    // Delete one of root A's files, then reindex ONLY root A.
    std::fs::remove_file(dir_a.join("gone_a.rs")).unwrap();
    let stats = pipeline
        .ingest_directory_with_embeddings_rooted(
            &dir_a,
            &storage,
            &embedder,
            &index,
            Some(rid_a),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        stats.files_removed, 1,
        "root A's deleted file must be pruned by its own reindex"
    );
    let docs = storage.list_documents().await.unwrap();
    assert!(
        docs.iter()
            .any(|d| d.source_path.ends_with("b.rs") && d.source_path.starts_with(rid_b)),
        "sibling root B's document (namespaced under root B) must survive a root-A reindex"
    );
    assert!(
        docs.iter().all(|d| !d.source_path.contains("gone_a.rs")),
        "root A's orphan must be pruned"
    );
    assert!(
        docs.iter().any(|d| d.source_path.contains("keep_a.rs")),
        "root A's surviving file must remain"
    );
    assert_eq!(
        storage.document_count().await.unwrap(),
        2,
        "gone_a pruned; keep_a + sibling b remain"
    );
}

/// Orphan symbols: code symbols live in their own table (NOT cascaded by
/// `delete_document`), so a deleted file's symbols used to keep surfacing in
/// symbol search forever. The orphan sweep must prune them too.
/// (`symbol_refs` cascade off symbols via FK — proven in `storage_integration`.)
#[tokio::test]
async fn reindex_prunes_orphaned_symbols_after_file_deletion() {
    use ministr_core::storage::SymbolFilter;

    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("keep.rs"), "//! keep\npub fn keep() -> u32 { 1 }\n").unwrap();
    std::fs::write(
        src.join("gone.rs"),
        "//! gone\npub fn helper() -> u32 { 42 }\npub fn caller() -> u32 { helper() }\n",
    )
    .unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();
    let paths: Vec<PathBuf> = vec![src.clone()];

    pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();

    let before = storage.list_symbols(&SymbolFilter::default()).await.unwrap();
    assert!(
        before.iter().any(|s| s.file_path.contains("gone.rs")),
        "gone.rs symbols should be indexed initially"
    );
    assert!(
        before.iter().any(|s| s.file_path.contains("keep.rs")),
        "keep.rs symbols should be indexed initially"
    );

    // Delete the file out-of-band, then reindex.
    std::fs::remove_file(src.join("gone.rs")).unwrap();
    pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .unwrap();

    let after = storage.list_symbols(&SymbolFilter::default()).await.unwrap();
    assert!(
        after.iter().all(|s| !s.file_path.contains("gone.rs")),
        "deleted file's symbols must be pruned, but these survived: {:?}",
        after
            .iter()
            .filter(|s| s.file_path.contains("gone.rs"))
            .map(|s| (&s.file_path, &s.name))
            .collect::<Vec<_>>()
    );
    assert!(
        after.iter().any(|s| s.file_path.contains("keep.rs")),
        "keep.rs symbols must survive the sweep"
    );
}
