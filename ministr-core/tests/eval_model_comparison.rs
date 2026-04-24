//! Embedding model retrieval quality comparison.
//!
//! Runs the eval corpus through multiple real fastembed models and compares
//! retrieval metrics (P@5, R@5, MRR, nDCG@5) in a side-by-side table.
//!
//! These tests are `#[ignore]` because they require downloading embedding
//! models (~100-400MB each). Run with:
//!
//! ```sh
//! just bench-models                                    # all models
//! MINISTR_EVAL_MODELS="all-MiniLM-L6-v2" just bench-model # single model
//! ```

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use common::{EvalResults, GroundTruth, run_eval_with_embedder};
use ministr_core::embedding::{Embedder, FastEmbedder, TruncatingEmbedder};

/// A model configuration to benchmark.
struct ModelSpec {
    /// Display name for the results table.
    label: &'static str,
    /// Model name as recognized by `FastEmbedder::new`.
    model_name: &'static str,
    /// If set, truncate embeddings to this dimension (Matryoshka).
    truncate_dim: Option<usize>,
}

/// Default models to compare.
const DEFAULT_MODELS: &[ModelSpec] = &[
    ModelSpec {
        label: "all-MiniLM-L6-v2",
        model_name: "all-MiniLM-L6-v2",
        truncate_dim: None,
    },
    ModelSpec {
        label: "jina-embeddings-v2-base-code",
        model_name: "jina-embeddings-v2-base-code",
        truncate_dim: None,
    },
    ModelSpec {
        label: "nomic-embed-text-v1.5",
        model_name: "nomic-embed-text-v1.5",
        truncate_dim: None,
    },
    ModelSpec {
        label: "bge-small-en-v1.5",
        model_name: "bge-small-en-v1.5",
        truncate_dim: None,
    },
    ModelSpec {
        label: "nomic-embed-text-v1.5 @384",
        model_name: "nomic-embed-text-v1.5",
        truncate_dim: Some(384),
    },
];

/// Per-model benchmark result.
struct ModelResult {
    label: String,
    dim: usize,
    eval: EvalResults,
    elapsed_secs: f64,
}

// ---------------------------------------------------------------------------
// Main benchmark
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "Requires model downloads (~1GB total). Run with: just bench-models"]
async fn compare_embedding_models() {
    let Some((corpus_path, ground_truth)) = load_eval_data() else {
        return;
    };

    let specs = resolve_model_specs();
    let mut results: Vec<ModelResult> = Vec::with_capacity(specs.len());

    for spec in &specs {
        eprintln!();
        eprintln!("━━━ Loading model: {} ━━━", spec.label);

        let start = Instant::now();

        let base: Arc<dyn Embedder> = Arc::new(
            FastEmbedder::new(spec.model_name, None)
                .unwrap_or_else(|e| panic!("failed to load model '{}': {e}", spec.model_name)),
        );

        let embedder: Arc<dyn Embedder> = if let Some(target_dim) = spec.truncate_dim {
            Arc::new(
                TruncatingEmbedder::new(Arc::clone(&base), target_dim).unwrap_or_else(|e| {
                    panic!(
                        "failed to create truncating embedder for '{}' @{target_dim}: {e}",
                        spec.label
                    )
                }),
            )
        } else {
            base
        };

        let dim = embedder.dimension();
        eprintln!("  dimension: {dim}");

        let eval =
            run_eval_with_embedder(&corpus_path, &ground_truth, embedder.as_ref(), false).await;
        let elapsed_secs = start.elapsed().as_secs_f64();

        eprintln!(
            "  P@5={:.3}  R@5={:.3}  MRR={:.3}  nDCG@5={:.3}  ({elapsed_secs:.1}s)",
            eval.mean_precision, eval.mean_recall, eval.mrr, eval.mean_ndcg,
        );

        // Sanity: real models should produce non-zero retrieval
        assert!(
            eval.mean_recall > 0.0,
            "model '{}' produced zero recall — something is wrong",
            spec.label
        );

        results.push(ModelResult {
            label: spec.label.to_string(),
            dim,
            eval,
            elapsed_secs,
        });

        // Drop embedder before loading next model to limit memory usage.
        drop(embedder);
    }

    // Print comparison table
    print_comparison_table(&results);

    // Optionally write JSON output
    if let Ok(path) = std::env::var("MINISTR_EVAL_OUTPUT") {
        write_json_output(&path, &results);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load eval corpus and ground truth, returning `None` if missing.
fn load_eval_data() -> Option<(std::path::PathBuf, GroundTruth)> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus");
    let ground_truth_path = workspace_root.join("eval/ground-truth.json");

    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping: eval/ directory not found");
        return None;
    }

    let gt_json = std::fs::read_to_string(&ground_truth_path).expect("failed to read ground truth");
    let ground_truth: GroundTruth =
        serde_json::from_str(&gt_json).expect("failed to parse ground truth");

    Some((corpus_path, ground_truth))
}

/// Resolve which models to benchmark. Respects `MINISTR_EVAL_MODELS` env var
/// (comma-separated model names) or falls back to `DEFAULT_MODELS`.
fn resolve_model_specs() -> Vec<ModelSpec> {
    if let Ok(env_models) = std::env::var("MINISTR_EVAL_MODELS") {
        env_models
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|name| {
                // Check for @dim suffix for Matryoshka truncation
                if let Some((model, dim_str)) = name.rsplit_once('@') {
                    let dim: usize = dim_str
                        .parse()
                        .unwrap_or_else(|_| panic!("invalid dimension in '{name}'"));
                    ModelSpec {
                        label: Box::leak(name.to_string().into_boxed_str()),
                        model_name: Box::leak(model.to_string().into_boxed_str()),
                        truncate_dim: Some(dim),
                    }
                } else {
                    ModelSpec {
                        label: Box::leak(name.to_string().into_boxed_str()),
                        model_name: Box::leak(name.to_string().into_boxed_str()),
                        truncate_dim: None,
                    }
                }
            })
            .collect()
    } else {
        // Return references to the static default list. We can't move out of
        // a const slice, so reconstruct ModelSpec values.
        DEFAULT_MODELS
            .iter()
            .map(|s| ModelSpec {
                label: s.label,
                model_name: s.model_name,
                truncate_dim: s.truncate_dim,
            })
            .collect()
    }
}

/// Print a markdown-formatted comparison table to stderr.
fn print_comparison_table(results: &[ModelResult]) {
    eprintln!();
    eprintln!("=== Embedding Model Comparison ===");
    eprintln!();
    eprintln!(
        "| {:<32} | {:>3} | {:>5} | {:>5} | {:>5} | {:>6} | {:>8} |",
        "Model", "Dim", "P@5", "R@5", "MRR", "nDCG@5", "Time(s)"
    );
    eprintln!(
        "|{:-<34}|{:-<5}|{:-<7}|{:-<7}|{:-<7}|{:-<8}|{:-<10}|",
        "", "", "", "", "", "", ""
    );

    for r in results {
        eprintln!(
            "| {:<32} | {:>3} | {:.3} | {:.3} | {:.3} | {:>.3} | {:>7.1} |",
            r.label,
            r.dim,
            r.eval.mean_precision,
            r.eval.mean_recall,
            r.eval.mrr,
            r.eval.mean_ndcg,
            r.elapsed_secs,
        );
    }
    eprintln!();
}

/// Write results as JSON for programmatic comparison across runs.
fn write_json_output(path: &str, results: &[ModelResult]) {
    let models: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.label,
                "dimension": r.dim,
                "precision_at_5": r.eval.mean_precision,
                "recall_at_5": r.eval.mean_recall,
                "mrr": r.eval.mrr,
                "ndcg_at_5": r.eval.mean_ndcg,
                "total_seconds": r.elapsed_secs,
            })
        })
        .collect();

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs();

    let output = serde_json::json!({
        "timestamp": timestamp,
        "corpus_queries": results.first().map_or(0, |r| r.eval.query_count),
        "k": 5,
        "models": models,
    });

    let json = serde_json::to_string_pretty(&output).expect("failed to serialize JSON");
    std::fs::write(path, json).expect("failed to write JSON output");
    eprintln!("Results written to: {path}");
}
