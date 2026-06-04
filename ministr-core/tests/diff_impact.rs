//! FL7 / promote-core — `QueryService::compute_diff_impact` end-to-end over a
//! real git repo: a base commit + a head commit that edits a callee, ingested
//! (which registers the local corpus root the method resolves git against),
//! then the diff-aware blast radius. The edited callee is a seed; its caller
//! surfaces in the union impact.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::service::{CallDirection, QueryService};
use ministr_core::storage::SqliteStorage;

/// Deterministic mock embedder — the composition is symbol/AST + git driven, so
/// embedding quality is irrelevant and this avoids a model download.
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

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("run git")
        .success();
    assert!(ok, "git {args:?} failed");
}

#[tokio::test]
async fn compute_diff_impact_over_real_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Canonicalize so `git rev-parse --show-toplevel` matches the absolute
    // file_path stored at ingest time (macOS /var↔/private/var symlink).
    let repo = std::fs::canonicalize(tmp.path()).unwrap();

    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@t.test"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);

    let file = repo.join("code.rs");
    std::fs::write(
        &file,
        "pub fn callee() -> i32 {\n    let a = 1;\n    a + 1\n}\n\n\
         pub fn caller() -> i32 {\n    callee() + callee()\n}\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);

    // Head: edit callee's body only (caller is untouched, just shifted).
    std::fs::write(
        &file,
        "pub fn callee() -> i32 {\n    let a = 1;\n    let b = 2;\n    a + b\n}\n\n\
         pub fn caller() -> i32 {\n    callee() + callee()\n}\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "head"]);

    // Ingest the head tree (registers the local corpus root used for git).
    let dim = 8;
    let storage = SqliteStorage::open_in_memory().unwrap();
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 4096).unwrap();
    IngestionPipeline::new()
        .ingest_paths_with_embeddings(std::slice::from_ref(&repo), &storage, &embedder, &index)
        .await
        .expect("ingest repo");

    let service = QueryService::new(
        storage,
        Arc::new(MockEmbedder { dim }),
        Arc::new(HnswIndex::new(dim, 1).unwrap()),
    );

    let result = service
        .compute_diff_impact("HEAD~1..HEAD", 3, CallDirection::Incoming, false)
        .await
        .expect("compute_diff_impact");

    let changed: Vec<&str> = result
        .changed_symbols
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        changed.contains(&"callee"),
        "callee is a changed symbol: {changed:?}"
    );
    assert!(
        !changed.contains(&"caller"),
        "caller untouched: {changed:?}"
    );

    let impacted: Vec<&str> = result.impacted.iter().map(|c| c.name.as_str()).collect();
    assert!(
        impacted.contains(&"caller"),
        "caller is in the blast radius: {impacted:?}"
    );
    assert_eq!(result.range, "HEAD~1..HEAD");
}
