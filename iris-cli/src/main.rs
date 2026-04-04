//! iris-cli — binary entry point for the iris MCP server.
//!
//! Provides two subcommands:
//!
//! - `iris serve` (default) — starts the MCP server with background ingestion.
//!   Supports `--transport stdio` (default) and `--transport http` (Streamable HTTP).
//! - `iris index` — runs ingestion synchronously and exits (no MCP server).
//!
//! When invoked without a subcommand (`iris --corpus ./docs`), defaults to `serve`.

mod instance;
mod proxy;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iris_core::index::VectorIndex as _;

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

        /// Run as a thin proxy to the iris daemon instead of the monolithic server.
        ///
        /// When enabled, the MCP server connects to the iris daemon at
        /// `~/.iris/irisd.sock` and delegates all indexing and querying.
        /// Uses ~20 MB vs ~2 GB for the full server.
        #[arg(long)]
        proxy: bool,

        /// Enable OAuth 2.1 authentication for the HTTP transport.
        ///
        /// When enabled, the server exposes OAuth discovery endpoints and
        /// requires Bearer token authentication on the MCP endpoint.
        /// Only effective with `--transport http`.
        #[arg(long)]
        oauth: bool,

        /// OAuth issuer URL (default: `http://<host>:<port>`).
        ///
        /// Used in OAuth metadata discovery responses. Set this to the
        /// public-facing URL when deploying behind a reverse proxy.
        #[arg(long)]
        oauth_issuer: Option<String>,
    },

    /// Run corpus ingestion synchronously and exit (no MCP server).
    ///
    /// Useful for pre-warming the index, debugging ingestion issues,
    /// or running in CI pipelines.
    Index,

    /// Show daemon status (requires iris-app to be running).
    Status,

    /// Search the corpus via the daemon (requires iris-app to be running).
    Search {
        /// Search query.
        query: String,
        /// Maximum results.
        #[arg(short = 'k', long, default_value_t = 10)]
        top_k: usize,
    },

    /// Generate .iris.toml with auto-detected project settings.
    ///
    /// Scans the current directory for project manifests (Cargo.toml,
    /// package.json, pyproject.toml), detects workspace layouts and
    /// bridge frameworks, and writes a sensible default config.
    Init {
        /// Overwrite existing .iris.toml if present.
        #[arg(long)]
        force: bool,
    },

    /// Export the corpus index to a portable `.iris-index` bundle.
    ///
    /// Creates a zstd-compressed archive containing the content database
    /// (with session-local data stripped), HNSW vector index, and metadata
    /// manifest. The bundle can be imported on another machine without
    /// re-parsing or re-embedding.
    Export {
        /// Output file path (default: `<corpus-name>.iris-index` in current dir).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Import a `.iris-index` bundle into the local corpus store.
    ///
    /// Decompresses the bundle and loads the content database and HNSW
    /// index into the corpus data directory, ready for querying without
    /// re-indexing.
    Import {
        /// Path to the `.iris-index` bundle file.
        bundle: PathBuf,
    },
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

    // Discover per-repo .iris.toml (walks up from CWD).
    let cwd = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("failed to get current directory")?;
    let corpus_config = iris_core::config::RepoConfig::discover(&cwd)
        .into_diagnostic()
        .wrap_err("failed to read .iris.toml")?;

    match &corpus_config {
        Some((config_dir, cc)) => {
            let config_file = config_dir.join(iris_core::config::CORPUS_CONFIG_FILENAME);
            tracing::info!(
                config = %config_file.display(),
                paths = cc.corpus.paths.len(),
                git_repos = cc.corpus.git.len(),
                ignore_patterns = cc.corpus.ignore.len(),
                "loaded .iris.toml"
            );

            // Validate config and emit user-friendly warnings.
            let warnings = cc.validate(config_dir);
            for w in &warnings {
                tracing::warn!("{w}");
            }
        }
        None => {
            tracing::info!("no .iris.toml found — using CLI args or config.toml defaults");
        }
    }

    // Scaffold agent config files on first run (idempotent — skips existing files).
    iris_core::scaffold::scaffold_agent_config(&cwd);

    // Resolve corpus paths: .iris.toml > --corpus CLI > config.toml corpus_paths
    let corpus_paths: Vec<String> = if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else if cli.corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli.corpus.clone()
    };

    // Collect git repos from .iris.toml for post-startup cloning.
    let git_includes: Vec<iris_core::config::GitInclude> = corpus_config
        .as_ref()
        .map(|(_, cc)| cc.corpus.git.clone())
        .unwrap_or_default();

    // Resolve embedding model name: .iris.toml > config.toml default_model
    let resolved_model = iris_core::config::resolve_model_name(
        corpus_config.as_ref().map(|(_, cc)| cc),
        None,
        &config,
    );

    // Dispatch to the appropriate subcommand (default: serve over stdio).
    match cli.command.unwrap_or(Command::Serve {
        transport: Transport::Stdio,
        host: "127.0.0.1".to_string(),
        port: 8080,
        proxy: false,
        oauth: false,
        oauth_issuer: None,
    }) {
        Command::Serve {
            transport,
            host,
            port,
            proxy,
            oauth,
            oauth_issuer,
        } => match transport {
            Transport::Stdio if proxy => cmd_serve_proxy_stdio(&corpus_paths).await,
            Transport::Stdio
                if !proxy && iris_api::client::DaemonClient::new().is_healthy().await =>
            {
                // Daemon is running (tray app) — auto-switch to lightweight proxy.
                eprintln!("iris: daemon detected at ~/.iris/irisd.sock — running as proxy");
                cmd_serve_proxy_stdio(&corpus_paths).await
            }
            Transport::Stdio => {
                cmd_serve_stdio(
                    &corpus_paths,
                    &git_includes,
                    &config_path,
                    &config,
                    &resolved_model,
                )
                .await
            }
            Transport::Http => {
                let oauth_config = if oauth {
                    Some(iris_mcp::auth::OAuthConfig {
                        issuer: oauth_issuer.unwrap_or_else(|| format!("http://{host}:{port}")),
                        ..iris_mcp::auth::OAuthConfig::default()
                    })
                } else {
                    None
                };
                cmd_serve_http(
                    &corpus_paths,
                    &git_includes,
                    &config_path,
                    &config,
                    &host,
                    port,
                    oauth_config,
                    &resolved_model,
                )
                .await
            }
        },
        Command::Index => {
            cmd_index(
                &corpus_paths,
                &git_includes,
                &config_path,
                &config,
                &resolved_model,
            )
            .await
        }
        Command::Status => cmd_daemon_status().await,
        Command::Search { query, top_k } => cmd_daemon_search(&corpus_paths, &query, top_k).await,
        Command::Init { force } => cmd_init(&cwd, force),
        Command::Export { output } => {
            cmd_export(&corpus_paths, &config, &resolved_model, output.as_deref()).await
        }
        Command::Import { bundle } => cmd_import(&corpus_paths, &config, &bundle),
    }
}

/// Initialize shared infrastructure: storage, embedder, and vector index.
///
/// Returns the corpus data directory, index directory, and Arc-wrapped components.
async fn init_infrastructure(
    corpus_paths: &[String],
    config: &iris_core::config::IrisConfig,
    resolved_model: Option<&str>,
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

    // Use resolved model name or fall back to global default.
    let model_name = resolved_model.map_or_else(|| config.default_model.clone(), String::from);
    tracing::info!(model = %model_name, "resolved embedding model");

    // Initialize embedder with content-addressable cache.
    iris_core::mem_profile::checkpoint("before embedding model init");
    let raw_embedder: Arc<dyn iris_core::embedding::Embedder> = Arc::new(
        iris_core::embedding::FastEmbedder::with_data_dir(&model_name, &config.data_dir)
            .into_diagnostic()
            .wrap_err("failed to initialize embedding model")?,
    );
    iris_core::mem_profile::checkpoint("after embedding model init");
    let embedding_cache = iris_core::embedding::cache::EmbeddingCache::new(storage.conn());
    let embedder: Arc<dyn iris_core::embedding::Embedder> = Arc::new(
        iris_core::embedding::CachedEmbedder::new(raw_embedder, embedding_cache, &model_name),
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

    iris_core::mem_profile::checkpoint("before vector index init");
    let index: Arc<dyn iris_core::index::VectorIndex> =
        load_or_create_index(&index_dir, dim, &model_name)?;

    iris_core::mem_profile::checkpoint("after vector index init");

    Ok(InfrastructureContext {
        corpus_dir,
        index_dir,
        storage: Arc::new(storage),
        embedder,
        index,
    })
}

/// Load an existing HNSW index or create a fresh one.
///
/// Detects embedding model changes (dimension or model name mismatch) and
/// discards the old index when the model has changed, forcing a re-index.
fn load_or_create_index(
    index_dir: &Path,
    dim: usize,
    model_name: &str,
) -> Result<Arc<dyn iris_core::index::VectorIndex>> {
    if index_dir.exists() {
        match iris_core::index::HnswIndex::load(index_dir) {
            Ok(loaded) => {
                let dim_mismatch = loaded.dimension() != dim;
                let model_mismatch = loaded
                    .model_name()
                    .as_ref()
                    .is_some_and(|old| old != model_name);

                if dim_mismatch || model_mismatch {
                    let old_model = loaded.model_name().unwrap_or_else(|| "unknown".to_owned());
                    tracing::warn!(
                        old_model = %old_model,
                        new_model = %model_name,
                        old_dim = loaded.dimension(),
                        new_dim = dim,
                        "embedding model changed — discarding old index for re-indexing"
                    );
                    drop(loaded);
                    let _ = std::fs::remove_dir_all(index_dir);
                    return create_fresh_index(dim, model_name);
                }
                // Legacy index without model name — adopt current model
                if loaded.model_name().is_none() {
                    tracing::info!(
                        model = %model_name,
                        "upgrading legacy index with model name tracking"
                    );
                    loaded.set_model_name(model_name);
                }
                return Ok(Arc::new(loaded));
            }
            Err(e) => {
                tracing::warn!(error = %e, "corrupted vector index — discarding and rebuilding");
                let _ = std::fs::remove_dir_all(index_dir);
            }
        }
    }
    create_fresh_index(dim, model_name)
}

/// Create a fresh HNSW index with the given dimension and model name.
fn create_fresh_index(
    dim: usize,
    model_name: &str,
) -> Result<Arc<dyn iris_core::index::VectorIndex>> {
    let fresh = iris_core::index::HnswIndex::new(dim, 100_000)
        .into_diagnostic()
        .wrap_err("failed to create vector index")?;
    fresh.set_model_name(model_name);
    Ok(Arc::new(fresh))
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
    resolved_model: Option<&str>,
) -> Result<(
    iris_mcp::server::IrisServer,
    InfrastructureContext,
    Option<tokio::task::JoinHandle<()>>,
)> {
    tracing::info!(
        corpus_count = corpus_paths.len(),
        config = %config_path.display(),
        "iris starting — {} corpus path(s)",
        corpus_paths.len()
    );
    for path in corpus_paths {
        tracing::info!(path = %path, "  corpus root");
    }

    let ctx = init_infrastructure(corpus_paths, config, resolved_model).await?;

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

/// Spawn an HTTP listener for secondary iris instances to connect to.
///
/// Runs in a background task. When the primary's main MCP session ends,
/// the tokio runtime drops this task and the listener closes.
fn spawn_http_listener(server: iris_mcp::server::IrisServer, port: u16) {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    tokio::spawn(async move {
        let server_factory = move || Ok(server.clone());
        let session_manager = Arc::new(LocalSessionManager::default());
        let http_service = StreamableHttpService::new(
            server_factory,
            session_manager,
            StreamableHttpServerConfig::default(),
        );
        let app = axum::Router::new().nest_service("/mcp", http_service);

        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await else {
            tracing::warn!(port, "failed to bind HTTP listener for secondaries");
            return;
        };
        tracing::info!(port, "HTTP listener ready for secondary instances");
        let _ = axum::serve(listener, app).await;
    });
}

/// Spawn background corpus ingestion, returning when the MCP transport finishes.
fn spawn_background_ingestion(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
    ctx: &InfrastructureContext,
    ingestion_progress: &Arc<iris_core::ingestion::IngestionProgress>,
) {
    if corpus_paths.is_empty() && git_includes.is_empty() {
        return;
    }
    let bg_corpus_paths = corpus_paths.to_vec();
    let bg_git_includes = git_includes.to_vec();
    let bg_corpus_dir = ctx.corpus_dir.clone();
    let bg_storage = Arc::clone(&ctx.storage);
    let bg_embedder = Arc::clone(&ctx.embedder);
    let bg_index = Arc::clone(&ctx.index);
    let bg_index_dir = ctx.index_dir.clone();
    let bg_progress = Arc::clone(ingestion_progress);
    tokio::spawn(async move {
        match run_corpus_ingestion(
            &bg_corpus_paths,
            &bg_git_includes,
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

/// `iris serve --proxy` — thin MCP proxy over stdin/stdout.
///
/// `iris init` — detect project structure and generate `.iris.toml`.
fn cmd_init(root: &Path, force: bool) -> Result<()> {
    let detection = iris_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .iris.toml")?;

    eprintln!("Detected project: {}", detection.project_name);
    for ws in &detection.workspaces {
        eprintln!("  {} workspace ({} members)", ws.kind, ws.members.len());
    }
    if !detection.bridges.is_empty() {
        let names: Vec<_> = detection.bridges.iter().map(|b| format!("{b:?}")).collect();
        eprintln!("  Bridges: {}", names.join(", "));
    }
    eprintln!();
    let config_path = root.join(".iris.toml");
    let total_paths = detection.source_paths.len() + detection.doc_paths.len();
    if config_path.exists() && !force {
        eprintln!(".iris.toml already exists (use --force to overwrite)");
    } else {
        eprintln!("Generated .iris.toml with {total_paths} paths");
    }
    eprintln!("Updated .mcp.json (Claude Code)");
    eprintln!("Updated .vscode/mcp.json (GitHub Copilot)");
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new Claude Code session in this directory");
    eprintln!("  2. iris will auto-index and tools will be available");
    Ok(())
}

/// Connects to the iris daemon at `~/.iris/irisd.sock` and proxies all
/// tool calls. No ONNX model, no indexes, no `SQLite` — just HTTP over UDS.
async fn cmd_serve_proxy_stdio(corpus_paths: &[String]) -> Result<()> {
    eprintln!(
        "iris: proxy starting with {} corpus paths",
        corpus_paths.len()
    );

    // Pre-register corpus with daemon before starting MCP handshake.
    let client = iris_api::client::DaemonClient::new();
    match client.register_corpus(corpus_paths).await {
        Ok(resp) => {
            eprintln!(
                "iris: corpus {} registered (indexing_started={})",
                resp.corpus_id, resp.indexing_started
            );
        }
        Err(e) => {
            eprintln!("iris: warning — corpus registration failed: {e}");
        }
    }

    eprintln!("iris: starting MCP proxy on stdio");
    let proxy = iris_mcp::proxy::ProxyServer::new(corpus_paths.to_vec());
    let service = proxy
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("proxy MCP server failed")?;

    // Keep the service alive until the client disconnects.
    let _ = service.waiting().await;
    Ok(())
}

/// `iris export` — export the corpus index to a portable bundle.
async fn cmd_export(
    corpus_paths: &[String],
    config: &iris_core::config::IrisConfig,
    resolved_model: &str,
    output: Option<&Path>,
) -> Result<()> {
    use iris_core::bundle::{
        self, BUNDLE_FORMAT_VERSION, BundleCorpusRoot, BundleManifest, compute_bundle_version,
    };
    use iris_core::storage::Storage as _;

    // Resolve the corpus data directory without loading the embedding model.
    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        corpus_data_dir_name(corpus_paths)
    };
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    let db_path = corpus_dir.join("content.db");

    if !db_path.exists() {
        miette::bail!(
            "no indexed corpus found at {}. Run `iris index` first.",
            corpus_dir.display()
        );
    }

    // Open storage (no embedder needed for export).
    let storage = iris_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    let doc_count = storage
        .document_count()
        .await
        .into_diagnostic()
        .wrap_err("failed to count documents")?;
    let roots = storage
        .list_corpus_roots()
        .await
        .into_diagnostic()
        .wrap_err("failed to list corpus roots")?;

    // Get vector count and dimension from the HNSW index.
    let index_dir = corpus_dir.join("index");
    let (vector_count, dimension) = if index_dir.exists() {
        match iris_core::index::HnswIndex::load(&index_dir) {
            Ok(loaded) => (loaded.len(), loaded.dimension()),
            Err(_) => (0, 0),
        }
    } else {
        (0, 0)
    };

    let bundle_roots: Vec<BundleCorpusRoot> = roots
        .iter()
        .map(|r| BundleCorpusRoot {
            id: r.id.clone(),
            display_name: r.display_name.clone(),
            kind: r.kind.as_str().to_string(),
            commit_sha: r.commit_sha.clone(),
            branch: r.branch.clone(),
            repo_url: r.repo_url.clone(),
        })
        .collect();

    // Capture the source commit SHA: prefer corpus root metadata, fall back
    // to `git rev-parse HEAD` in the first corpus path.
    let source_commit = bundle_roots
        .iter()
        .find_map(|r| r.commit_sha.clone())
        .or_else(|| {
            corpus_paths
                .first()
                .and_then(|p| iris_core::git::local_head_sha(std::path::Path::new(p)))
        });

    let bundle_version = Some(compute_bundle_version(&bundle_roots));

    let manifest = BundleManifest {
        format_version: BUNDLE_FORMAT_VERSION,
        model_name: resolved_model.to_string(),
        dimension,
        vector_count,
        document_count: doc_count,
        symbol_count: 0,
        corpus_roots: bundle_roots,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        bundle_version,
        source_commit,
    };

    let output_path = output.map_or_else(
        || {
            let filename = format!("{corpus_name}.iris-index");
            PathBuf::from(filename)
        },
        Path::to_path_buf,
    );

    bundle::export_bundle(&corpus_dir, &output_path, &manifest)
        .into_diagnostic()
        .wrap_err("failed to export bundle")?;

    eprintln!("Exported {doc_count} documents, {vector_count} vectors ({dimension}d)");
    eprintln!("Bundle: {}", output_path.display());
    Ok(())
}

/// `iris import` — import a `.iris-index` bundle into local storage.
fn cmd_import(
    corpus_paths: &[String],
    config: &iris_core::config::IrisConfig,
    bundle_path: &Path,
) -> Result<()> {
    use iris_core::bundle;

    if !bundle_path.exists() {
        miette::bail!("bundle not found: {}", bundle_path.display());
    }

    // Determine corpus directory name from the bundle filename or corpus paths.
    let corpus_name = if corpus_paths.is_empty() {
        bundle_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
            .to_owned()
    } else {
        corpus_data_dir_name(corpus_paths)
    };
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);

    if corpus_dir.join("content.db").exists() {
        miette::bail!(
            "corpus '{}' already exists at {}. Remove it first or use a different name.",
            corpus_name,
            corpus_dir.display()
        );
    }

    let manifest = bundle::import_bundle(bundle_path, &corpus_dir)
        .into_diagnostic()
        .wrap_err("failed to import bundle")?;

    eprintln!(
        "Imported: {} documents, {} vectors ({}d, model: {})",
        manifest.document_count, manifest.vector_count, manifest.dimension, manifest.model_name
    );
    eprintln!("Corpus: {}", corpus_dir.display());
    Ok(())
}

/// `iris status` — show corpus stats from local storage.
///
/// Opens the `SQLite` database directly (no embedding model needed) and
/// displays document counts, corpus roots, data directory, and index info.
/// Falls back to the daemon API if available for richer live status.
#[allow(clippy::too_many_lines)]
async fn cmd_daemon_status() -> Result<()> {
    use iris_core::storage::Storage as _;

    // Try daemon first for live status.
    let client = iris_api::client::DaemonClient::new();
    if client.is_available() {
        if let Ok(status) = client.status().await {
            eprintln!("iris daemon v{}", status.version);
            eprintln!("  Uptime:    {}s", status.uptime_secs);
            eprintln!("  Memory:    {:.0} MB", status.memory_mb);
            eprintln!(
                "  Model:     {} ({}d)",
                status.model, status.model_dimension
            );
            eprintln!("  Corpora:   {}", status.corpora.len());
            for c in &status.corpora {
                eprintln!(
                    "    {} — {} files, {} sections, {} embeddings [{}]",
                    c.id,
                    c.files_indexed,
                    c.sections_count,
                    c.embeddings_count,
                    match &c.status {
                        iris_api::corpus::IndexingStatus::Idle => "idle".to_string(),
                        iris_api::corpus::IndexingStatus::Indexing {
                            files_done,
                            files_total,
                        } => format!("indexing {files_done}/{files_total}"),
                        iris_api::corpus::IndexingStatus::Error { message } =>
                            format!("error: {message}"),
                    }
                );
            }
            return Ok(());
        }
    }

    // Daemon not available — show local storage stats.
    let config_path = iris_core::config::IrisConfig::default_path();
    let config = iris_core::config::IrisConfig::load(&config_path)
        .into_diagnostic()
        .wrap_err("failed to load config")?;

    let cwd = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("failed to get current directory")?;
    let corpus_config = iris_core::config::RepoConfig::discover(&cwd)
        .into_diagnostic()
        .wrap_err("failed to read .iris.toml")?;

    let corpus_paths: Vec<String> = if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else {
        config.corpus_paths.clone()
    };

    let corpus_name = if corpus_paths.is_empty() {
        "default".to_owned()
    } else {
        corpus_data_dir_name(&corpus_paths)
    };

    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    let db_path = corpus_dir.join("content.db");
    let index_dir = corpus_dir.join("index");

    eprintln!("iris status (local)");
    eprintln!();
    eprintln!("  Data dir:  {}", corpus_dir.display());
    eprintln!(
        "  Database:  {}",
        if db_path.exists() {
            "exists"
        } else {
            "not found"
        }
    );
    eprintln!(
        "  Index dir: {}",
        if index_dir.exists() {
            "exists"
        } else {
            "not found"
        }
    );

    if !db_path.exists() {
        eprintln!();
        eprintln!("  No index found. Run `iris serve` or `iris index` to build one.");
        return Ok(());
    }

    let storage = iris_core::storage::SqliteStorage::open(&db_path)
        .into_diagnostic()
        .wrap_err("failed to open content database")?;

    let doc_count = storage.document_count().await.unwrap_or(0);
    let roots = storage.list_corpus_roots().await.unwrap_or_default();

    eprintln!("  Documents: {doc_count}");
    eprintln!("  Roots:     {}", roots.len());
    for r in &roots {
        let name = r.display_name.as_deref().unwrap_or(&r.path);
        eprintln!("    {name} ({} — {} files)", r.kind.as_str(), r.file_count);
    }

    // Show index file sizes.
    if index_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&index_dir) {
            let total_bytes: u64 = entries
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                .sum();
            #[allow(clippy::cast_precision_loss)]
            let mb = total_bytes as f64 / 1_048_576.0;
            eprintln!("  Index size: {mb:.1} MB");
        }
    }

    Ok(())
}

/// `iris search` — search the corpus via the daemon.
async fn cmd_daemon_search(corpus_paths: &[String], query: &str, top_k: usize) -> Result<()> {
    let client = iris_api::client::DaemonClient::new();
    if !client.is_available() {
        miette::bail!(
            "iris daemon is not running (no socket at {:?})",
            client.socket_path()
        );
    }

    // Register corpus if needed.
    let resp = client
        .register_corpus(corpus_paths)
        .await
        .into_diagnostic()
        .wrap_err("failed to register corpus")?;

    let results = client
        .survey(&resp.corpus_id, query, Some(top_k))
        .await
        .into_diagnostic()
        .wrap_err("search failed")?;

    for r in &results.results {
        eprintln!("[{:8}] {:.3}  {}", r.resolution, r.score, r.content_id);
        eprintln!("  {}", r.text.lines().next().unwrap_or(""));
        eprintln!();
    }

    if results.results.is_empty() {
        eprintln!("No results found.");
    }

    Ok(())
}

/// `iris serve --transport stdio` — MCP server over stdin/stdout.
///
/// On first invocation for a corpus, acquires an exclusive lock and starts
/// as the primary (stdio + HTTP listener for secondaries). On subsequent
/// invocations, detects the primary and runs as a transparent proxy.
async fn cmd_serve_stdio(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    resolved_model: &str,
) -> Result<()> {
    // Compute the corpus data dir early for lock detection.
    let corpus_name = corpus_data_dir_name(corpus_paths);
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    std::fs::create_dir_all(&corpus_dir)
        .into_diagnostic()
        .wrap_err("failed to create corpus directory")?;

    let role = instance::acquire_role(&corpus_dir, corpus_paths)?;

    match role {
        instance::InstanceRole::Secondary { mcp_url } => {
            tracing::info!(url = %mcp_url, "secondary instance — proxying to primary");
            proxy::run_stdio_proxy(&mcp_url).await
        }
        instance::InstanceRole::Primary(lock) => {
            let (server, ctx, _coherence_handle) =
                build_server(corpus_paths, config_path, config, Some(resolved_model)).await?;

            let ingestion_progress = server.ingestion_progress_arc();

            // Spawn HTTP listener for secondary instances.
            spawn_http_listener(server.clone(), lock.http_port);

            // Start stdio MCP server FIRST so Claude Code doesn't time out.
            let mcp_service = server
                .serve(rmcp::transport::stdio())
                .await
                .into_diagnostic()
                .wrap_err("failed to start MCP stdio transport")?;

            // Ingest in background AFTER the MCP server is running.
            spawn_background_ingestion(corpus_paths, git_includes, &ctx, &ingestion_progress);

            mcp_service
                .waiting()
                .await
                .into_diagnostic()
                .wrap_err("MCP server exited with error")?;

            // lock dropped here → flock released, port file removed.
            drop(lock);
            tracing::info!("iris shutting down");
            Ok(())
        }
    }
}

/// `iris serve --transport http` — Streamable HTTP MCP server.
#[allow(clippy::too_many_arguments)]
async fn cmd_serve_http(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    host: &str,
    port: u16,
    oauth_config: Option<iris_mcp::auth::OAuthConfig>,
    resolved_model: &str,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let (server, ctx, _coherence_handle) =
        build_server(corpus_paths, config_path, config, Some(resolved_model)).await?;

    let ingestion_progress = server.ingestion_progress_arc();

    // Extract Arcs before moving server into the factory closure.
    let a2a_service = server.service_arc();
    let a2a_registry = server.registry_arc();

    // Each HTTP session gets its own IrisServer clone.
    // All clones share the same Arc'd infrastructure.
    let server_factory = move || Ok(server.clone());

    let session_manager = Arc::new(LocalSessionManager::default());
    let http_service = StreamableHttpService::new(
        server_factory,
        session_manager,
        StreamableHttpServerConfig::default(),
    );

    let mcp_router = axum::Router::new().nest_service("/mcp", http_service);

    // A2A protocol endpoints (agent card + task submission)
    let a2a_state = iris_mcp::a2a::A2aState {
        service: a2a_service,
        registry: a2a_registry,
        tasks: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };
    let a2a_router = iris_mcp::a2a::a2a_routes(a2a_state);

    // Bundle-serving endpoints (read-only, public).
    let bundle_state = iris_mcp::bundle_routes::BundleState {
        corpus_dir: ctx.corpus_dir.clone(),
        model_name: resolved_model.to_string(),
        storage: Arc::clone(&ctx.storage),
    };
    let bundle_router = iris_mcp::bundle_routes::bundle_routes(bundle_state);

    let app = if let Some(oauth_cfg) = oauth_config {
        tracing::info!("OAuth 2.1 authentication enabled");
        let store = iris_mcp::auth::OAuthStore::new(oauth_cfg);
        let protected = iris_mcp::auth::protected_router(mcp_router, store.clone());
        // Bundle endpoints require iris:bundle:read scope when OAuth is active.
        let protected_bundles =
            iris_mcp::auth::scope_protected_router(bundle_router, store, "iris:bundle:read");
        a2a_router.merge(protected).merge(protected_bundles)
    } else {
        a2a_router.merge(mcp_router).merge(bundle_router)
    };

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to bind HTTP server to {bind_addr}"))?;

    tracing::info!(address = %bind_addr, "iris HTTP server listening");

    // Ingest in background AFTER the HTTP server is bound.
    spawn_background_ingestion(corpus_paths, git_includes, &ctx, &ingestion_progress);

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
    git_includes: &[iris_core::config::GitInclude],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    resolved_model: &str,
) -> Result<()> {
    tracing::info!(
        corpus_count = corpus_paths.len(),
        config = %config_path.display(),
        "iris index — {} corpus path(s)",
        corpus_paths.len()
    );
    for path in corpus_paths {
        tracing::info!(path = %path, "  corpus root");
    }

    if corpus_paths.is_empty() && git_includes.is_empty() {
        tracing::warn!("no corpus paths specified, nothing to index");
        return Ok(());
    }

    let ctx = init_infrastructure(corpus_paths, config, Some(resolved_model)).await?;

    let progress = Arc::new(iris_core::ingestion::IngestionProgress::new());
    run_corpus_ingestion(
        corpus_paths,
        git_includes,
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

    let registry = server.registry_arc();

    // Create a channel for pushing coherence change notifications to MCP
    // resource subscribers (e.g. iris://status).
    let (notify_tx, notify_rx) = tokio::sync::mpsc::unbounded_channel();
    server.set_coherence_receiver(notify_rx);

    let handle = iris_core::coherence::spawn_coherence_task(
        watcher,
        engine,
        Arc::clone(storage),
        registry,
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
#[allow(clippy::too_many_arguments)]
async fn run_corpus_ingestion(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
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

    // Clone and ingest git repositories (from --corpus args and .iris.toml).
    if !git_urls.is_empty() {
        ingest_git_sources(&git_urls, &pipeline, storage, embedder, index).await;
    }
    if !git_includes.is_empty() {
        ingest_git_includes(git_includes, &pipeline, storage, embedder, index).await;
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
            .fetch_and_ingest_with_embeddings(url, pipeline, storage, embedder, index, None)
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
///
/// Registers each clone as a persistent corpus root with git provenance
/// metadata, ensuring clone directories are treated as read-only assets
/// rather than temporary artifacts.
async fn ingest_git_sources(
    urls: &[String],
    pipeline: &iris_core::ingestion::IngestionPipeline,
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
) {
    let git_fetcher = iris_core::git::GitFetcher::with_defaults();

    for url in urls {
        match git_fetcher.clone(url, None, None, None).await {
            Ok(clone_result) => {
                // Register a corpus root for the clone so it persists across sessions.
                let root_id = iris_core::ingestion::compute_root_id(&clone_result.clone_dir);
                let clone_root = iris_core::types::CorpusRoot {
                    id: root_id.clone(),
                    path: clone_result.clone_dir.to_string_lossy().to_string(),
                    kind: iris_core::types::RootKind::Git,
                    display_name: Some(git_repo_display_name(url)),
                    file_count: 0,
                    language_stats: std::collections::HashMap::new(),
                    repo_url: Some(url.clone()),
                    branch: clone_result.metadata.branch.clone(),
                    commit_sha: Some(clone_result.metadata.commit_sha.clone()),
                    clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
                    sparse_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
                    tracing::warn!(
                        url = %url,
                        error = %e,
                        "failed to register clone corpus root"
                    );
                }

                // Ingest with root-scoped ingestion to namespace documents.
                match pipeline
                    .ingest_directory_with_embeddings_rooted(
                        &clone_result.clone_dir,
                        storage,
                        embedder,
                        index,
                        Some(&root_id),
                        None,
                    )
                    .await
                {
                    Ok(stats) => {
                        // Update the root's file count after ingestion.
                        let updated_root = iris_core::types::CorpusRoot {
                            file_count: stats.files_indexed,
                            ..clone_root
                        };
                        if let Err(e) = storage.upsert_corpus_root(&updated_root).await {
                            tracing::warn!(
                                url = %url,
                                error = %e,
                                "failed to update clone root stats"
                            );
                        }

                        // Record in git cache for staleness tracking.
                        let git_cache_record = iris_core::storage::GitCacheRecord {
                            repo_url: url.clone(),
                            branch: clone_result.metadata.branch.clone(),
                            commit_sha: clone_result.metadata.commit_sha.clone(),
                            clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                            clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                            checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                        };
                        if let Err(e) = storage.upsert_git_cache(&git_cache_record).await {
                            tracing::warn!(
                                url = %url,
                                error = %e,
                                "failed to record git cache"
                            );
                        }

                        tracing::info!(
                            url = %url,
                            clone_dir = %clone_result.clone_dir.display(),
                            files_indexed = stats.files_indexed,
                            sections = stats.total_sections,
                            root_id = %root_id,
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

/// Clone and ingest git repositories specified in `.iris.toml`.
///
/// Unlike [`ingest_git_sources`], this accepts [`GitInclude`] structs
/// which support sparse checkout paths and branch selection.
async fn ingest_git_includes(
    includes: &[iris_core::config::GitInclude],
    pipeline: &iris_core::ingestion::IngestionPipeline,
    storage: &iris_core::storage::SqliteStorage,
    embedder: &dyn iris_core::embedding::Embedder,
    index: &dyn iris_core::index::VectorIndex,
) {
    let git_fetcher = iris_core::git::GitFetcher::with_defaults();

    for inc in includes {
        let paths_ref: Option<Vec<String>> = inc.paths.clone();
        match git_fetcher
            .clone(&inc.repo, paths_ref.as_deref(), inc.branch.as_deref(), None)
            .await
        {
            Ok(clone_result) => {
                let root_id = iris_core::ingestion::compute_root_id(&clone_result.clone_dir);
                let clone_root = iris_core::types::CorpusRoot {
                    id: root_id.clone(),
                    path: clone_result.clone_dir.to_string_lossy().to_string(),
                    kind: iris_core::types::RootKind::Git,
                    display_name: Some(git_repo_display_name(&inc.repo)),
                    file_count: 0,
                    language_stats: std::collections::HashMap::new(),
                    repo_url: Some(inc.repo.clone()),
                    branch: clone_result.metadata.branch.clone(),
                    commit_sha: Some(clone_result.metadata.commit_sha.clone()),
                    clone_timestamp: Some(clone_result.metadata.clone_timestamp.clone()),
                    sparse_paths: clone_result.metadata.checked_out_paths.clone(),
                };
                if let Err(e) = storage.upsert_corpus_root(&clone_root).await {
                    tracing::warn!(repo = %inc.repo, error = %e, "failed to register clone root");
                }

                match pipeline
                    .ingest_directory_with_embeddings_rooted(
                        &clone_result.clone_dir,
                        storage,
                        embedder,
                        index,
                        Some(&root_id),
                        None,
                    )
                    .await
                {
                    Ok(stats) => {
                        let updated_root = iris_core::types::CorpusRoot {
                            file_count: stats.files_indexed,
                            ..clone_root
                        };
                        let _ = storage.upsert_corpus_root(&updated_root).await;

                        let git_cache_record = iris_core::storage::GitCacheRecord {
                            repo_url: inc.repo.clone(),
                            branch: clone_result.metadata.branch.clone(),
                            commit_sha: clone_result.metadata.commit_sha.clone(),
                            clone_timestamp: clone_result.metadata.clone_timestamp.clone(),
                            clone_dir: clone_result.clone_dir.to_string_lossy().to_string(),
                            checked_out_paths: clone_result.metadata.checked_out_paths.clone(),
                        };
                        let _ = storage.upsert_git_cache(&git_cache_record).await;

                        tracing::info!(
                            repo = %inc.repo,
                            files_indexed = stats.files_indexed,
                            sections = stats.total_sections,
                            "git include from .iris.toml ingested"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(repo = %inc.repo, error = %e, "git include ingestion failed");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(repo = %inc.repo, error = %e, "git include clone failed");
            }
        }
    }
}

/// Derive a human-readable display name from a git repository URL.
///
/// Extracts the `owner/repo` portion from common URL formats, falling
/// back to the full URL if parsing fails.
fn git_repo_display_name(url: &str) -> String {
    // Strip trailing .git
    let cleaned = url.strip_suffix(".git").unwrap_or(url);
    // Try to extract owner/repo from the last two path segments.
    let segments: Vec<&str> = cleaned.rsplit('/').take(2).collect();
    if segments.len() == 2 {
        format!("{}/{}", segments[1], segments[0])
    } else {
        cleaned.to_string()
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
