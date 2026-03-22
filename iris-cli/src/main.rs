//! iris-cli — binary entry point for the iris MCP server.
//!
//! Parses command-line arguments, loads configuration, initializes tracing,
//! constructs the query service with real storage/embedding/index backends,
//! and starts the MCP server over stdio transport.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt;
use sha2::{Digest, Sha256};

use iris_core::coherence::{CoherenceEngine, FileWatcher};
use iris_core::index::VectorIndexLoad as _;
use iris_core::session::BudgetConfig;

/// iris — a context cache controller for LLM agents.
///
/// Runs an MCP server over stdio that provides intelligent context retrieval
/// tools (survey, read, extract) for a local document corpus.
#[derive(Parser, Debug)]
#[command(name = "iris", version, about)]
struct Cli {
    /// Paths to corpus directories, individual files, or glob patterns.
    ///
    /// Accepts multiple values via repeated flags:
    /// `iris --corpus ./docs --corpus ./DESIGN.md --corpus ./CHANGELOG.md`
    #[arg(short, long)]
    corpus: Vec<PathBuf>,

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

    // Merge CLI corpus paths with config corpus_paths (CLI takes precedence).
    let corpus_paths = if cli.corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli.corpus.clone()
    };

    tracing::info!(
        corpus = ?corpus_paths,
        config = %config_path.display(),
        "iris starting"
    );

    // Determine corpus data directory from a hash of all paths.
    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        corpus_data_dir_name(&corpus_paths)
    };

    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
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

    // Run ingestion if corpus paths were provided.
    if !corpus_paths.is_empty() {
        run_ingestion(&corpus_paths, &storage, &*embedder, &*index, &index_dir).await?;
    }

    // Build the query service and start the MCP server.
    let storage = Arc::new(storage);
    let service = Arc::new(iris_core::service::QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder),
        Arc::clone(&index),
    ));

    let session_id = corpus_session_id(&corpus_paths);
    let budget_config = BudgetConfig {
        max_context_tokens: config.default_context_budget,
        ..BudgetConfig::default()
    };

    let server = iris_mcp::server::IrisServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&storage),
        session_id,
    )
    .await;

    // Enable web fetching for iris_fetch tool.
    let server = enable_web_fetcher(server, &corpus_dir, &embedder, &index)?;

    // Enable git cloning for iris_clone tool.
    let server = enable_git_fetcher(server, &embedder, &index);

    // Spawn coherence file watcher if corpus paths were provided.
    let _coherence_handle = if corpus_paths.is_empty() {
        None
    } else {
        spawn_coherence(&corpus_paths, &server, &storage, &embedder, &index)?
    };

    let mcp_service = server
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("failed to start MCP stdio transport")?;

    mcp_service
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("MCP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}

/// Spawn the coherence file watcher and background processing task.
///
/// Watches all corpus paths for file changes, re-indexes affected files
/// (including embeddings and vector index), and propagates coherence alerts
/// to the active session.
fn spawn_coherence(
    corpus_paths: &[PathBuf],
    server: &iris_mcp::server::IrisServer,
    storage: &Arc<iris_core::storage::SqliteStorage>,
    embedder: &Arc<dyn iris_core::embedding::Embedder>,
    index: &Arc<dyn iris_core::index::VectorIndex>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    // Collect watch paths: directories directly, individual files via their parent.
    let watch_paths: Vec<PathBuf> = corpus_paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().unwrap_or(p).to_path_buf()
            }
        })
        .collect();

    let watcher = FileWatcher::new(&watch_paths)
        .into_diagnostic()
        .wrap_err("failed to start file watcher for coherence")?;

    // Use the first directory path as the primary corpus_dir for the coherence engine.
    let primary_dir = corpus_paths
        .iter()
        .find(|p| p.is_dir())
        .cloned()
        .or_else(|| {
            corpus_paths
                .first()
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(|| PathBuf::from("."));

    let engine = Arc::new(CoherenceEngine::with_embeddings(
        primary_dir,
        Arc::clone(embedder),
        Arc::clone(index),
    ));

    let session = server.session_arc();

    let handle =
        iris_core::coherence::spawn_coherence_task(watcher, engine, Arc::clone(storage), session);

    tracing::info!(
        corpus = ?corpus_paths,
        "coherence file watcher started"
    );

    Ok(Some(handle))
}

/// Run the ingestion pipeline against all corpus paths, then persist the index.
async fn run_ingestion(
    corpus_paths: &[PathBuf],
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
    index_dir: &std::path::Path,
) -> Result<()> {
    let start = std::time::Instant::now();
    let pipeline = iris_core::ingestion::IngestionPipeline::new();
    let stats = pipeline
        .ingest_paths_with_embeddings(corpus_paths, storage, embedder, index)
        .await
        .into_diagnostic()
        .wrap_err("ingestion failed")?;

    index
        .persist(index_dir)
        .into_diagnostic()
        .wrap_err("failed to persist vector index")?;

    let elapsed_ms = elapsed_millis(start);
    tracing::info!(
        files_discovered = stats.files_discovered,
        files_indexed = stats.files_indexed,
        files_skipped = stats.files_skipped,
        files_removed = stats.files_removed,
        files_failed = stats.files_failed,
        sections = stats.total_sections,
        claims = stats.total_claims,
        embeddings = stats.total_embeddings,
        elapsed_ms,
        "ingestion complete"
    );
    Ok(())
}

/// Derive a stable session ID from the corpus paths so sessions persist across restarts.
fn corpus_session_id(corpus_paths: &[PathBuf]) -> Option<String> {
    if corpus_paths.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for p in corpus_paths {
        hasher.update(p.to_string_lossy().as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.finalize();
    Some(format!(
        "iris-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]
    ))
}

/// Derive a stable data directory name from corpus paths.
fn corpus_data_dir_name(corpus_paths: &[PathBuf]) -> String {
    if corpus_paths.len() == 1 {
        // Single path: use the file/directory name for human readability.
        if let Some(name) = corpus_paths[0].file_name().and_then(|n| n.to_str()) {
            return name.to_owned();
        }
    }
    // Multiple paths or no filename: hash all paths.
    let mut hasher = Sha256::new();
    for p in corpus_paths {
        hasher.update(p.to_string_lossy().as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.finalize();
    format!(
        "multi-{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3]
    )
}

/// Convert elapsed duration to milliseconds, saturating at `u64::MAX`.
fn elapsed_millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Enable web fetching on the server by constructing an `HttpClient` and `WebFetcher`.
fn enable_web_fetcher(
    server: iris_mcp::server::IrisServer,
    corpus_dir: &Path,
    embedder: &Arc<dyn iris_core::embedding::Embedder>,
    index: &Arc<dyn iris_core::index::VectorIndex>,
) -> Result<iris_mcp::server::IrisServer> {
    let web_cache_dir = corpus_dir.join("web");
    let http_client = iris_core::web::HttpClient::with_defaults()
        .into_diagnostic()
        .wrap_err("failed to create HTTP client for web fetcher")?;
    let web_fetcher = iris_core::web::fetcher::WebFetcher::new(
        http_client,
        &web_cache_dir,
        iris_core::web::fetcher::WebFetcherConfig::default(),
    );
    Ok(server.with_web_fetcher(web_fetcher, Arc::clone(embedder), Arc::clone(index)))
}

/// Enable git cloning on the server by constructing a `GitFetcher`.
fn enable_git_fetcher(
    server: iris_mcp::server::IrisServer,
    embedder: &Arc<dyn iris_core::embedding::Embedder>,
    index: &Arc<dyn iris_core::index::VectorIndex>,
) -> iris_mcp::server::IrisServer {
    let git_fetcher = iris_core::git::GitFetcher::with_defaults();
    server.with_git_fetcher(git_fetcher, Arc::clone(embedder), Arc::clone(index))
}
