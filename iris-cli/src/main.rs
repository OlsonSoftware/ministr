//! iris-cli — binary entry point for the iris MCP server.
//!
//! Provides two subcommands:
//!
//! - `iris serve` (default) — starts the MCP server with background ingestion.
//!   Supports `--transport stdio` (default) and `--transport http` (Streamable HTTP).
//! - `iris index` — runs ingestion synchronously and exits (no MCP server).
//!
//! When invoked without a subcommand (`iris --corpus ./docs`), defaults to `serve`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt;
use sha2::{Digest, Sha256};

use iris_core::coherence::{CoherenceEngine, FileWatcher};
use iris_core::index::VectorIndexLoad as _;
use iris_core::session::BudgetConfig;
use iris_core::storage::Storage as _;

/// iris — a context cache controller for LLM agents.
///
/// Runs an MCP server that provides intelligent context retrieval
/// tools (survey, read, extract) for a local document corpus.
/// Supports stdio and Streamable HTTP transports.
#[derive(Parser, Debug)]
#[command(name = "iris", version, about)]
struct Cli {
    /// Corpus sources: local paths, `https://` URLs, or `github://` URLs.
    ///
    /// Accepts multiple values via repeated flags:
    /// `iris --corpus ./docs --corpus https://docs.rs/serde`
    #[arg(short, long, global = true)]
    corpus: Vec<String>,

    /// Path to config file (default: ~/.iris/config.toml).
    #[arg(short = 'C', long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the MCP server (default when no subcommand is given).
    ///
    /// By default uses stdio transport. Use `--transport http` to start
    /// a Streamable HTTP server for remote/multi-client deployments.
    Serve {
        /// Transport: `stdio` (default) or `http` (Streamable HTTP).
        #[arg(short, long, default_value = "stdio")]
        transport: Transport,

        /// Host to bind the HTTP server to (only used with `--transport http`).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port for the HTTP server (only used with `--transport http`).
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },

    /// Run corpus ingestion synchronously and exit (no MCP server).
    ///
    /// Useful for pre-warming the index, debugging ingestion issues,
    /// or running in CI pipelines.
    Index,
}

/// MCP transport mode.
#[derive(Debug, Clone, clap::ValueEnum)]
enum Transport {
    /// JSON-RPC over stdin/stdout (default for local MCP clients).
    Stdio,
    /// Streamable HTTP (MCP spec 2025-03-26) for remote/multi-client deployments.
    Http,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
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
    let corpus_paths: Vec<String> = if cli.corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli.corpus.clone()
    };

    // Dispatch to the appropriate subcommand (default: serve over stdio).
    match cli.command.unwrap_or(Command::Serve {
        transport: Transport::Stdio,
        host: "127.0.0.1".to_string(),
        port: 8080,
    }) {
        Command::Serve {
            transport,
            host,
            port,
        } => match transport {
            Transport::Stdio => cmd_serve_stdio(&corpus_paths, &config_path, &config).await,
            Transport::Http => {
                cmd_serve_http(&corpus_paths, &config_path, &config, &host, port).await
            }
        },
        Command::Index => cmd_index(&corpus_paths, &config_path, &config).await,
    }
}

/// Initialize shared infrastructure: storage, embedder, and vector index.
///
/// Returns the corpus data directory, index directory, and Arc-wrapped components.
async fn init_infrastructure(
    corpus_paths: &[String],
    config: &iris_core::config::IrisConfig,
) -> Result<InfrastructureContext> {
    // Determine corpus data directory from a hash of all paths.
    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        corpus_data_dir_name(corpus_paths)
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
    // If the SQLite DB is empty (fresh migration) but a stale vector index
    // exists on disk, discard it to avoid phantom IDs from a previous run.
    let dim = embedder.dimension();
    let index_dir = corpus_dir.join("index");
    let doc_count: usize = storage
        .document_count()
        .await
        .into_diagnostic()
        .wrap_err("failed to check document count")?;
    if doc_count == 0 && index_dir.exists() {
        tracing::warn!("empty database with stale vector index — discarding old index");
        std::fs::remove_dir_all(&index_dir)
            .into_diagnostic()
            .wrap_err("failed to remove stale vector index")?;
    }

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

    Ok(InfrastructureContext {
        corpus_dir,
        index_dir,
        storage: Arc::new(storage),
        embedder,
        index,
    })
}

/// Shared infrastructure components initialized at startup.
struct InfrastructureContext {
    corpus_dir: PathBuf,
    index_dir: PathBuf,
    storage: Arc<iris_core::storage::SqliteStorage>,
    embedder: Arc<dyn iris_core::embedding::Embedder>,
    index: Arc<dyn iris_core::index::VectorIndex>,
}

/// Build a fully configured `IrisServer` with web fetcher, git fetcher, and coherence watcher.
///
/// Returns the server and a coherence handle that must be kept alive.
#[allow(clippy::too_many_lines)]
async fn build_server(
    corpus_paths: &[String],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
) -> Result<(
    iris_mcp::server::IrisServer,
    InfrastructureContext,
    Option<tokio::task::JoinHandle<()>>,
)> {
    tracing::info!(
        corpus = ?corpus_paths,
        config = %config_path.display(),
        "iris starting"
    );

    let ctx = init_infrastructure(corpus_paths, config).await?;

    let service = Arc::new(iris_core::service::QueryService::new(
        (*ctx.storage).clone(),
        Arc::clone(&ctx.embedder),
        Arc::clone(&ctx.index),
    ));

    let session_id = corpus_session_id(corpus_paths);
    let budget_config = BudgetConfig {
        max_context_tokens: config.default_context_budget,
        ..BudgetConfig::default()
    };

    let server = iris_mcp::server::IrisServer::with_persistence(
        service,
        budget_config,
        Arc::clone(&ctx.storage),
        session_id,
    )
    .await;

    let server = enable_web_fetcher(server, &ctx.corpus_dir, &ctx.embedder, &ctx.index)?;
    let server = enable_git_fetcher(server, &ctx.embedder, &ctx.index);

    // Spawn coherence file watcher.
    let local_paths: Vec<PathBuf> = corpus_paths
        .iter()
        .filter_map(|p| {
            if let iris_core::config::CorpusSource::Local(path) =
                iris_core::config::classify_corpus_path(p)
            {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    let coherence_handle = if local_paths.is_empty() {
        None
    } else {
        spawn_coherence(
            &local_paths,
            &server,
            &ctx.storage,
            &ctx.embedder,
            &ctx.index,
        )?
    };

    Ok((server, ctx, coherence_handle))
}

/// Spawn background corpus ingestion, returning when the MCP transport finishes.
fn spawn_background_ingestion(
    corpus_paths: &[String],
    ctx: &InfrastructureContext,
    ingestion_progress: &Arc<iris_core::ingestion::IngestionProgress>,
) {
    if corpus_paths.is_empty() {
        return;
    }
    let bg_corpus_paths = corpus_paths.to_vec();
    let bg_corpus_dir = ctx.corpus_dir.clone();
    let bg_storage = Arc::clone(&ctx.storage);
    let bg_embedder = Arc::clone(&ctx.embedder);
    let bg_index = Arc::clone(&ctx.index);
    let bg_index_dir = ctx.index_dir.clone();
    let bg_progress = Arc::clone(ingestion_progress);
    tokio::spawn(async move {
        match run_corpus_ingestion(
            &bg_corpus_paths,
            &bg_corpus_dir,
            &bg_storage,
            &*bg_embedder,
            &*bg_index,
            &bg_index_dir,
            &bg_progress,
        )
        .await
        {
            Ok(()) => tracing::info!("background corpus ingestion complete"),
            Err(e) => {
                tracing::error!(error = %e, "background corpus ingestion failed");
                bg_progress.complete();
            }
        }
    });
}

/// `iris serve --transport stdio` — MCP server over stdin/stdout.
async fn cmd_serve_stdio(
    corpus_paths: &[String],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
) -> Result<()> {
    let (server, ctx, _coherence_handle) = build_server(corpus_paths, config_path, config).await?;

    let ingestion_progress = server.ingestion_progress_arc();

    // Start MCP server FIRST so Claude Code doesn't time out.
    let mcp_service = server
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("failed to start MCP stdio transport")?;

    // Ingest in background AFTER the MCP server is running.
    spawn_background_ingestion(corpus_paths, &ctx, &ingestion_progress);

    mcp_service
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("MCP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}

/// `iris serve --transport http` — Streamable HTTP MCP server.
async fn cmd_serve_http(
    corpus_paths: &[String],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    host: &str,
    port: u16,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let (server, ctx, _coherence_handle) = build_server(corpus_paths, config_path, config).await?;

    let ingestion_progress = server.ingestion_progress_arc();

    // Each HTTP session gets its own IrisServer clone.
    // All clones share the same Arc'd infrastructure.
    let server_factory = move || Ok(server.clone());

    let session_manager = Arc::new(LocalSessionManager::default());
    let http_service = StreamableHttpService::new(
        server_factory,
        session_manager,
        StreamableHttpServerConfig::default(),
    );

    let app = axum::Router::new().nest_service("/mcp", http_service);

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to bind HTTP server to {bind_addr}"))?;

    tracing::info!(address = %bind_addr, "iris HTTP server listening");

    // Ingest in background AFTER the HTTP server is bound.
    spawn_background_ingestion(corpus_paths, &ctx, &ingestion_progress);

    axum::serve(listener, app)
        .await
        .into_diagnostic()
        .wrap_err("HTTP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}

/// `iris index` — run ingestion synchronously and exit.
async fn cmd_index(
    corpus_paths: &[String],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
) -> Result<()> {
    tracing::info!(
        corpus = ?corpus_paths,
        config = %config_path.display(),
        "iris starting (index mode)"
    );

    if corpus_paths.is_empty() {
        tracing::warn!("no corpus paths specified, nothing to index");
        return Ok(());
    }

    let ctx = init_infrastructure(corpus_paths, config).await?;

    let progress = Arc::new(iris_core::ingestion::IngestionProgress::new());
    run_corpus_ingestion(
        corpus_paths,
        &ctx.corpus_dir,
        &ctx.storage,
        &*ctx.embedder,
        &*ctx.index,
        &ctx.index_dir,
        &progress,
    )
    .await?;

    tracing::info!("indexing complete");
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

    // Create a channel for pushing coherence change notifications to MCP
    // resource subscribers (e.g. iris://status).
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel();
    server.set_coherence_receiver(notify_rx);

    let handle = iris_core::coherence::spawn_coherence_task(
        watcher,
        engine,
        Arc::clone(storage),
        session,
        Some(notify_tx),
    );

    tracing::info!(
        corpus = ?corpus_paths,
        "coherence file watcher started"
    );

    Ok(Some(handle))
}

/// Classify corpus paths and run the appropriate ingestion pipeline for each source type.
///
/// - Local paths are ingested via the standard file ingestion pipeline.
/// - Web URLs are fetched and ingested via `WebFetcher`.
/// - Git URLs are cloned and their content is ingested as local files.
async fn run_corpus_ingestion(
    corpus_paths: &[String],
    corpus_dir: &Path,
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
    index_dir: &std::path::Path,
    progress: &Arc<iris_core::ingestion::IngestionProgress>,
) -> Result<()> {
    use iris_core::config::{CorpusSource, classify_corpus_path};

    let mut local_paths = Vec::new();
    let mut web_urls = Vec::new();
    let mut git_urls = Vec::new();

    for raw in corpus_paths {
        match classify_corpus_path(raw) {
            CorpusSource::Local(path) => local_paths.push(path),
            CorpusSource::Web(url) => web_urls.push(url),
            CorpusSource::Git(url) => git_urls.push(url),
        }
    }

    tracing::info!(
        local = local_paths.len(),
        web = web_urls.len(),
        git = git_urls.len(),
        local_paths = ?local_paths,
        "classified corpus sources"
    );

    let start = std::time::Instant::now();
    let pipeline =
        iris_core::ingestion::IngestionPipeline::new().with_progress(Arc::clone(progress));

    // Ingest local paths.
    if !local_paths.is_empty() {
        let stats = pipeline
            .ingest_paths_with_embeddings(&local_paths, storage, embedder, index)
            .await
            .into_diagnostic()
            .wrap_err("local ingestion failed")?;

        tracing::info!(
            files_discovered = stats.files_discovered,
            files_indexed = stats.files_indexed,
            files_skipped = stats.files_skipped,
            files_removed = stats.files_removed,
            files_failed = stats.files_failed,
            sections = stats.total_sections,
            claims = stats.total_claims,
            embeddings = stats.total_embeddings,
            "local ingestion complete"
        );

        if stats.files_discovered == 0 {
            tracing::warn!(
                paths = ?local_paths,
                "no files discovered from local corpus paths — check that paths exist and contain supported files"
            );
        }
    }

    // Fetch and ingest web URLs.
    if !web_urls.is_empty() {
        ingest_web_sources(&web_urls, corpus_dir, &pipeline, storage, embedder, index).await?;
    }

    // Clone and ingest git repositories.
    if !git_urls.is_empty() {
        ingest_git_sources(&git_urls, &pipeline, storage, embedder, index).await;
    }

    index
        .persist(index_dir)
        .into_diagnostic()
        .wrap_err("failed to persist vector index")?;

    let elapsed_ms = elapsed_millis(start);
    tracing::info!(
        local = local_paths.len(),
        web = web_urls.len(),
        git = git_urls.len(),
        elapsed_ms,
        "corpus ingestion complete"
    );

    Ok(())
}

/// Fetch and ingest web URLs via `WebFetcher`.
async fn ingest_web_sources(
    urls: &[String],
    corpus_dir: &Path,
    pipeline: &iris_core::ingestion::IngestionPipeline,
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
) -> Result<()> {
    let web_cache_dir = corpus_dir.join("web");
    let http_client = iris_core::web::HttpClient::with_defaults()
        .into_diagnostic()
        .wrap_err("failed to create HTTP client for corpus web fetch")?;
    let web_fetcher = iris_core::web::fetcher::WebFetcher::new(
        http_client,
        &web_cache_dir,
        iris_core::web::fetcher::WebFetcherConfig::default(),
    );

    for url in urls {
        match web_fetcher
            .fetch_and_ingest_with_embeddings(url, pipeline, storage, embedder, index)
            .await
        {
            Ok(result) => {
                tracing::info!(
                    url = %url,
                    pages = result.pages_fetched(),
                    sections = result.sections_indexed,
                    strategy = %result.strategy,
                    "web corpus ingestion complete"
                );
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "web corpus ingestion failed");
            }
        }
    }
    Ok(())
}

/// Clone and ingest git repositories via `GitFetcher`.
async fn ingest_git_sources(
    urls: &[String],
    pipeline: &iris_core::ingestion::IngestionPipeline,
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
) {
    let git_fetcher = iris_core::git::GitFetcher::with_defaults();

    for url in urls {
        match git_fetcher.clone(url, None, None).await {
            Ok(clone_result) => {
                let clone_paths = vec![clone_result.clone_dir.clone()];
                match pipeline
                    .ingest_paths_with_embeddings(&clone_paths, storage, embedder, index)
                    .await
                {
                    Ok(stats) => {
                        tracing::info!(
                            url = %url,
                            clone_dir = %clone_result.clone_dir.display(),
                            files_indexed = stats.files_indexed,
                            sections = stats.total_sections,
                            "git corpus ingestion complete"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            url = %url,
                            error = %e,
                            "git corpus file ingestion failed"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(url = %url, error = %e, "git corpus clone failed");
            }
        }
    }
}

/// Derive a stable session ID from the corpus paths so sessions persist across restarts.
fn corpus_session_id(corpus_paths: &[String]) -> Option<String> {
    if corpus_paths.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for p in corpus_paths {
        hasher.update(p.as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.finalize();
    Some(format!(
        "iris-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]
    ))
}

/// Derive a stable data directory name from corpus paths.
fn corpus_data_dir_name(corpus_paths: &[String]) -> String {
    if corpus_paths.len() == 1 {
        // Single path: use the last component for human readability.
        let p = &corpus_paths[0];
        let name = p.rsplit('/').find(|s| !s.is_empty()).unwrap_or(p);
        // Only use the name if it looks like a simple path component (no scheme).
        if !name.contains("://") && !name.contains(':') {
            return name.to_owned();
        }
    }
    // Multiple paths or URL: hash all paths.
    let mut hasher = Sha256::new();
    for p in corpus_paths {
        hasher.update(p.as_bytes());
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
