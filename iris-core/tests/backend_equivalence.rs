//! Backend equivalence tests — verify Candle and FastEmbed produce
//! compatible vectors for the same model.
//!
//! These tests require model downloads (~160MB) and run in --release mode
//! for acceptable performance. Run with:
//!
//!     cargo test --test backend_equivalence -p iris-core --features candle --release -- --ignored --nocapture

#[cfg(all(feature = "candle", target_os = "macos"))]
mod candle_vs_onnx {
    use iris_core::embedding::{CandleEmbedder, Embedder, FastEmbedder, candle_supported_models};

    /// Cosine similarity between two vectors.
    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len());
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        dot / (norm_a * norm_b)
    }

    /// Test corpus — varied text lengths and styles.
    const TEST_TEXTS: &[&str] = &[
        "The quick brown fox jumps over the lazy dog.",
        "Rust's ownership model prevents data races at compile time.",
        "fn main() { println!(\"Hello, world!\"); }",
        "HNSW provides approximate nearest neighbor search with logarithmic query time.",
        "The embedding cache uses content-addressable storage with SHA-256 hashes.",
        "pub struct PrefetchEngine { cache: PrefetchCache, topic_tracker: TopicTracker }",
        "Machine learning models can be quantized to INT8 for faster inference.",
        "The Matryoshka representation learning technique trains embeddings that are useful at multiple truncation levels.",
        "SQLite WAL mode enables concurrent readers with a single writer.",
        "async fn survey(&self, query: &str, top_k: usize) -> Result<Vec<SurveyResult>>",
        "Cross-encoder reranking improves retrieval quality by jointly attending to query and document tokens.",
        "The session shadow tracks what content has been delivered to the agent, enabling deduplication.",
        "use tokio::sync::RwLock; // Prefer RwLock over Mutex when reads dominate.",
        "Speculative prefetching predicts what the agent will need next based on sequential, topical, and structural locality.",
        "Binary quantization reduces vector storage by 32x while maintaining reasonable recall.",
        "The coherence engine watches for file changes and re-indexes affected sections.",
        "impl Embedder for CandleEmbedder { fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> }",
        "ONNX Runtime provides cross-platform inference with hardware-specific optimizations.",
        "The budget manager tracks token utilization and recommends evictions when context pressure rises.",
        "A claim is an atomic factual statement extracted from a section of documentation.",
    ];

    /// Verify that Candle and FastEmbed produce compatible vectors for a given model.
    ///
    /// "Compatible" means cosine similarity >= threshold for all text pairs.
    /// We use 0.95 as the threshold — both backends load the same model weights
    /// but use different numerical implementations (ONNX vs Candle Metal/CPU),
    /// so small floating-point differences are expected.
    #[tokio::test]
    #[ignore = "requires model downloads (~160MB). Run with: just test-backend-equiv"]
    async fn candle_and_fastembed_produce_equivalent_vectors() {
        let data_dir = tempfile::tempdir().unwrap();
        let threshold = 0.95;

        for model_info in candle_supported_models() {
            let model_name = model_info.name;
            eprintln!("\n--- Testing model: {model_name} ({}d) ---", model_info.dimension);

            let candle = match CandleEmbedder::with_data_dir(model_name, data_dir.path()) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("  SKIP: CandleEmbedder failed to load: {e}");
                    continue;
                }
            };
            let onnx = match FastEmbedder::with_data_dir(model_name, data_dir.path()) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("  SKIP: FastEmbedder failed to load: {e}");
                    continue;
                }
            };

            assert_eq!(candle.dimension(), onnx.dimension(),
                "dimension mismatch for {model_name}");

            let candle_vecs = candle.embed(TEST_TEXTS).expect("candle embed failed");
            let onnx_vecs = onnx.embed(TEST_TEXTS).expect("onnx embed failed");

            assert_eq!(candle_vecs.len(), TEST_TEXTS.len());
            assert_eq!(onnx_vecs.len(), TEST_TEXTS.len());

            let mut min_sim = f32::MAX;
            let mut max_sim = f32::MIN;
            let mut sum_sim = 0.0f64;

            for (i, (cv, ov)) in candle_vecs.iter().zip(onnx_vecs.iter()).enumerate() {
                let sim = cosine_similarity(cv, ov);
                min_sim = min_sim.min(sim);
                max_sim = max_sim.max(sim);
                sum_sim += f64::from(sim);

                assert!(
                    sim >= threshold,
                    "model {model_name}, text[{i}]: cosine similarity {sim:.6} < {threshold} \
                     text: {:?}",
                    &TEST_TEXTS[i][..TEST_TEXTS[i].len().min(60)]
                );
            }

            let mean_sim = sum_sim / TEST_TEXTS.len() as f64;
            eprintln!(
                "  cosine similarity — min: {min_sim:.6}, mean: {mean_sim:.6}, max: {max_sim:.6}"
            );
        }
    }
}
