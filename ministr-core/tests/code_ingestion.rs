//! Integration test: multi-path code ingestion with `.rs` files.
//!
//! Verifies that the ingestion pipeline discovers and indexes Rust source
//! files from multiple corpus paths, producing non-zero document and section
//! counts with embeddings in the vector index.

use std::path::PathBuf;

use ministr_core::index::{HnswIndex, VectorIndex};
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::SqliteStorage;

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
