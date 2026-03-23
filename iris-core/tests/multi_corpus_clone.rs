//! Integration test: sequential multi-corpus clone with root registration.
//!
//! Verifies that cloning 3+ repositories sequentially results in:
//! - All repos remaining searchable (documents from each root exist).
//! - Corpus root count growing monotonically.
//! - Distinct root IDs for each cloned repository.

use std::collections::HashSet;
use std::path::Path;

use iris_core::index::{HnswIndex, VectorIndex};
use iris_core::ingestion::{IngestionPipeline, compute_root_id};
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::types::{CorpusRoot, RootKind};

/// Deterministic mock embedder for integration tests.
struct MockEmbedder {
    dim: usize,
}

impl iris_core::embedding::Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, iris_core::error::IndexError> {
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

/// Create a directory with a minimal Rust source file.
fn create_mock_repo(dir: &Path, name: &str) {
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        format!(
            r#"//! {name} library.

/// A struct unique to {name}.
pub struct {name}Config {{
    pub value: String,
}}

impl {name}Config {{
    /// Create a new {name} config.
    pub fn new() -> Self {{
        Self {{
            value: String::from("{name}"),
        }}
    }}
}}
"#,
        ),
    )
    .unwrap();

    // Add a second file so there's meaningful content.
    std::fs::write(
        src_dir.join("util.rs"),
        format!(
            r#"//! {name} utilities.

/// Helper function for {name}.
pub fn {lower}_helper() -> &'static str {{
    "{name}_helper"
}}
"#,
            lower = name.to_lowercase(),
        ),
    )
    .unwrap();
}

/// Clone and ingest a mock repo directory as if it were a cloned repository,
/// registering a corpus root with git-style metadata.
async fn clone_and_ingest_mock(
    dir: &Path,
    repo_url: &str,
    pipeline: &IngestionPipeline,
    storage: &SqliteStorage,
    embedder: &MockEmbedder,
    index: &HnswIndex,
) -> String {
    let root_id = compute_root_id(dir);

    // Register corpus root (mirroring clone_and_ingest in the MCP server).
    let clone_root = CorpusRoot {
        id: root_id.clone(),
        path: dir.to_string_lossy().to_string(),
        kind: RootKind::Git,
        display_name: Some(repo_url.to_string()),
        file_count: 0,
        language_stats: std::collections::HashMap::new(),
        repo_url: Some(repo_url.to_string()),
        branch: Some("main".to_string()),
        commit_sha: Some("abc123".to_string()),
        clone_timestamp: Some("1711036800".to_string()),
        sparse_paths: Vec::new(),
    };
    storage.upsert_corpus_root(&clone_root).await.unwrap();

    // Ingest with root-scoped pipeline.
    let stats = pipeline
        .ingest_directory_with_embeddings_rooted(dir, storage, embedder, index, Some(&root_id))
        .await
        .unwrap();

    // Update root with file count (indexed + skipped = all discovered files).
    let updated_root = CorpusRoot {
        file_count: stats.files_indexed + stats.files_skipped,
        ..clone_root
    };
    storage.upsert_corpus_root(&updated_root).await.unwrap();

    assert!(
        stats.files_discovered > 0,
        "should discover files from {repo_url}"
    );

    root_id
}

#[tokio::test]
async fn sequential_clone_all_remain_searchable_and_roots_grow_monotonically() {
    let tmp = tempfile::tempdir().unwrap();

    // Create 3 separate mock repositories.
    let repo_names = ["Alpha", "Beta", "Gamma"];
    let mut repo_dirs = Vec::new();
    for name in &repo_names {
        let repo_dir = tmp.path().join(name.to_lowercase());
        create_mock_repo(&repo_dir, name);
        repo_dirs.push(repo_dir);
    }

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();

    let mut root_ids = Vec::new();
    let mut prev_root_count = 0;

    // Clone and ingest each repo sequentially.
    for (i, (repo_dir, name)) in repo_dirs.iter().zip(repo_names.iter()).enumerate() {
        let fake_url = format!("https://github.com/test/{}", name.to_lowercase());
        let root_id =
            clone_and_ingest_mock(repo_dir, &fake_url, &pipeline, &storage, &embedder, &index)
                .await;
        root_ids.push(root_id);

        // Root count must grow monotonically.
        let roots = storage.list_corpus_roots().await.unwrap();
        assert!(
            roots.len() > prev_root_count,
            "root count should grow monotonically: was {prev_root_count}, now {} after adding {name}",
            roots.len(),
        );
        prev_root_count = roots.len();

        // All previously-ingested roots should still be present.
        assert_eq!(
            roots.len(),
            i + 1,
            "expected {} roots after cloning {name}",
            i + 1,
        );
    }

    // All root IDs should be distinct.
    let unique_ids: HashSet<&String> = root_ids.iter().collect();
    assert_eq!(
        unique_ids.len(),
        repo_names.len(),
        "all root IDs should be distinct"
    );

    // Verify all 3 repos are still searchable — each should have documents.
    let all_roots = storage.list_corpus_roots().await.unwrap();
    assert_eq!(all_roots.len(), 3, "should have exactly 3 corpus roots");

    for root in &all_roots {
        assert_eq!(root.kind, RootKind::Git, "all roots should be git kind");
        assert!(
            root.file_count > 0,
            "root {} should have indexed files",
            root.id
        );
        assert!(
            root.repo_url.is_some(),
            "root {} should have repo_url",
            root.id
        );
    }

    // Verify documents from each root exist in storage.
    for root_id in &root_ids {
        let docs = storage.list_documents_by_root(root_id).await.unwrap();
        assert!(
            !docs.is_empty(),
            "root {root_id} should have documents in storage"
        );
    }

    // Vector index should contain embeddings from all 3 repos.
    assert!(
        index.len() > 0,
        "vector index should contain embeddings from all repos"
    );
}

#[tokio::test]
async fn re_ingesting_same_repo_does_not_duplicate_root() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().join("repo");
    create_mock_repo(&repo_dir, "Repeat");

    let storage = SqliteStorage::open_in_memory().unwrap();
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).unwrap();
    let pipeline = IngestionPipeline::new();

    let fake_url = "https://github.com/test/repeat";

    // Ingest the same repo twice.
    let root_id_1 =
        clone_and_ingest_mock(&repo_dir, fake_url, &pipeline, &storage, &embedder, &index).await;
    let root_id_2 =
        clone_and_ingest_mock(&repo_dir, fake_url, &pipeline, &storage, &embedder, &index).await;

    assert_eq!(
        root_id_1, root_id_2,
        "same repo should produce same root ID"
    );

    let roots = storage.list_corpus_roots().await.unwrap();
    assert_eq!(
        roots.len(),
        1,
        "re-ingesting should upsert, not duplicate the root"
    );
}
