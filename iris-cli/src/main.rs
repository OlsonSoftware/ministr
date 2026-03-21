//! iris-cli — binary entry point for the iris MCP server.
//!
//! Parses command-line arguments, loads configuration, initializes tracing,
//! constructs the query service with real storage/embedding/index backends,
//! and starts the MCP server over stdio transport.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt;

use iris_core::index::VectorIndexLoad as _;

/// iris — a context cache controller for LLM agents.
///
/// Runs an MCP server over stdio that provides intelligent context retrieval
/// tools (survey, read, extract) for a local document corpus.
#[derive(Parser, Debug)]
#[command(name = "iris", version, about)]
struct Cli {
    /// Path to the corpus directory to serve.
    #[arg(short, long)]
    corpus: Option<PathBuf>,

    /// Path to config file (default: ~/.iris/config.toml).
    #[arg(short = 'C', long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments before initializing anything else.
    let cli = Cli::parse();

    // Set up miette error reporting.
    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().build())
    }))
    .expect("miette hook should be set once");

    // Initialize tracing (writes to stderr so stdout is free for MCP).
    iris_core::tracing::init_tracing();

    // Load configuration.
    let config_path = cli
        .config
        .unwrap_or_else(iris_core::config::IrisConfig::default_path);
    let config = iris_core::config::IrisConfig::load(&config_path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to load config from {}", config_path.display()))?;

    tracing::info!(
        corpus = ?cli.corpus,
        config = %config_path.display(),
        "iris starting"
    );

    // Determine corpus data directory.
    let corpus_name = cli
        .corpus
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let corpus_dir = config.data_dir.join("corpora").join(corpus_name);
    let db_path = corpus_dir.join("content.db");

    // Create corpus directory if it doesn't exist.
    std::fs::create_dir_all(&corpus_dir)
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "failed to create corpus directory: {}",
                corpus_dir.display()
            )
        })?;

    // Initialize storage.
    let storage = iris_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    // Initialize embedder.
    let embedder: Arc<dyn iris_core::embedding::Embedder> = Arc::new(
        iris_core::embedding::FastEmbedder::new(&config.default_model, None)
            .into_diagnostic()
            .wrap_err("failed to initialize embedding model")?,
    );

    // Initialize vector index.
    let dim = embedder.dimension();
    let index_dir = corpus_dir.join("index");
    let index: Arc<dyn iris_core::index::VectorIndex> = if index_dir.exists() {
        Arc::new(
            iris_core::index::HnswIndex::load(&index_dir)
                .into_diagnostic()
                .wrap_err("failed to load vector index")?,
        )
    } else {
        Arc::new(
            iris_core::index::HnswIndex::new(dim, 100_000)
                .into_diagnostic()
                .wrap_err("failed to create vector index")?,
        )
    };

    // Build the query service.
    let service = Arc::new(iris_core::service::QueryService::new(
        storage, embedder, index,
    ));

    // Create the MCP server and serve over stdio.
    let server = iris_mcp::server::IrisServer::new(service);
    let mcp_service = server
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("failed to start MCP stdio transport")?;

    // Wait for the service to shut down.
    mcp_service
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("MCP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}
