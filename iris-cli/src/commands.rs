//! CLI subcommand implementations for the iris CLI.
//!
//! Each `pub(crate)` function corresponds to a CLI subcommand dispatched from
//! [`main`](crate::main). This module keeps `main.rs` focused on argument
//! parsing and dispatch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt as _;

use iris_core::index::VectorIndex as _;
use iris_core::index::VectorIndexLoad as _;

use crate::infra;
use crate::ingestion;

// ---------------------------------------------------------------------------
// iris serve --transport stdio
// ---------------------------------------------------------------------------

/// `iris serve --transport stdio` — MCP server over stdin/stdout.
///
/// On first invocation for a corpus, acquires an exclusive lock and starts
/// as the primary (stdio + HTTP listener for secondaries). On subsequent
/// invocations, detects the primary and runs as a transparent proxy.
pub(crate) async fn cmd_serve_stdio(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    resolved_model: &str,
    repo_config_dir: Option<&Path>,
) -> Result<()> {
    // Compute the corpus data dir early for lock detection.
    let corpus_name = infra::corpus_data_dir_name(corpus_paths);
    let corpus_dir = config.data_dir.join("corpora").join(&corpus_name);
    std::fs::create_dir_all(&corpus_dir)
        .into_diagnostic()
        .wrap_err("failed to create corpus directory")?;

    let role = crate::instance::acquire_role(&corpus_dir, corpus_paths)?;

    match role {
        crate::instance::InstanceRole::Secondary { mcp_url } => {
            tracing::info!(url = %mcp_url, "secondary instance — proxying to primary");
            crate::proxy::run_stdio_proxy(&mcp_url).await
        }
        crate::instance::InstanceRole::Primary(lock) => {
            let (server, ctx, _coherence_handle) =
                infra::build_server(corpus_paths, config_path, config, Some(resolved_model))
                    .await?;

            let ingestion_progress = server.ingestion_progress_arc();

            // Spawn HTTP listener for secondary instances.
            infra::spawn_http_listener(server.clone(), lock.http_port);

            // Start stdio MCP server FIRST so Claude Code doesn't time out.
            let mcp_service = server
                .serve(rmcp::transport::stdio())
                .await
                .into_diagnostic()
                .wrap_err("failed to start MCP stdio transport")?;

            // Ingest in background AFTER the MCP server is running.
            infra::spawn_background_ingestion(
                corpus_paths,
                git_includes,
                &ctx,
                &ingestion_progress,
            );

            // Watch .iris.toml for path changes and re-index automatically.
            let _config_watcher_handle = repo_config_dir.and_then(|dir| {
                ingestion::spawn_config_watcher(
                    dir.to_path_buf(),
                    corpus_paths.to_vec(),
                    &ctx,
                    &ingestion_progress,
                )
            });

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

// ---------------------------------------------------------------------------
// iris serve --transport http
// ---------------------------------------------------------------------------

/// `iris serve --transport http` — Streamable HTTP MCP server.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_serve_http(
    corpus_paths: &[String],
    git_includes: &[iris_core::config::GitInclude],
    config_path: &Path,
    config: &iris_core::config::IrisConfig,
    host: &str,
    port: u16,
    oauth_config: Option<iris_mcp::auth::OAuthConfig>,
    resolved_model: &str,
    repo_config_dir: Option<&Path>,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let (server, ctx, _coherence_handle) =
        infra::build_server(corpus_paths, config_path, config, Some(resolved_model)).await?;

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
    infra::spawn_background_ingestion(corpus_paths, git_includes, &ctx, &ingestion_progress);

    // Watch .iris.toml for path changes and re-index automatically.
    let _config_watcher_handle = repo_config_dir.and_then(|dir| {
        ingestion::spawn_config_watcher(
            dir.to_path_buf(),
            corpus_paths.to_vec(),
            &ctx,
            &ingestion_progress,
        )
    });

    axum::serve(listener, app)
        .await
        .into_diagnostic()
        .wrap_err("HTTP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// iris serve --proxy
// ---------------------------------------------------------------------------

/// `iris serve --proxy` — thin MCP proxy over stdin/stdout.
///
/// Connects to the iris daemon at `~/.iris/irisd.sock` and proxies all
/// tool calls. No ONNX model, no indexes, no `SQLite` — just HTTP over UDS.
pub(crate) async fn cmd_serve_proxy_stdio(corpus_paths: &[String]) -> Result<()> {
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

    // Eagerly create a daemon session so the GUI shows it immediately.
    if let Err(e) = proxy.initialize().await {
        eprintln!("iris: warning — eager session init failed: {e}");
    }

    let proxy_handle = proxy.clone();
    let service = proxy
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("proxy MCP server failed")?;

    // Keep the service alive until the client disconnects.
    let _ = service.waiting().await;

    // Clean up the daemon session so the GUI doesn't show stale entries.
    proxy_handle.shutdown().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// iris index
// ---------------------------------------------------------------------------

/// `iris index` — run ingestion synchronously and exit.
pub(crate) async fn cmd_index(
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

    let ctx = infra::init_infrastructure(corpus_paths, config, Some(resolved_model)).await?;

    let progress = Arc::new(iris_core::ingestion::IngestionProgress::new());
    ingestion::run_corpus_ingestion(
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

// ---------------------------------------------------------------------------
// iris init
// ---------------------------------------------------------------------------

/// `iris init` — detect project structure and generate `.iris.toml`.
pub(crate) fn cmd_init(root: &Path, force: bool) -> Result<()> {
    let detection = iris_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .iris.toml")?;

    // Scaffold agent config files (Claude Code hooks, Cursor rules, etc.).
    let scaffolded = iris_core::scaffold::scaffold_agent_config(root);

    eprintln!(
        "Detected project: {} ({})",
        detection.project_name, detection.project_type
    );
    for ws in &detection.workspaces {
        eprintln!("  {} workspace ({} members)", ws.kind, ws.members.len());
    }
    if !detection.bridges.is_empty() {
        let names: Vec<_> = detection.bridges.iter().map(|b| format!("{b:?}")).collect();
        eprintln!("  Bridges: {}", names.join(", "));
    }
    let langs = detection.detected_languages();
    if !langs.is_empty() {
        let names: Vec<_> = langs.iter().map(|l| format!("{l:?}")).collect();
        eprintln!("  Languages: {}", names.join(", "));
    }
    eprintln!();
    let config_path = root.join(".iris.toml");
    let total_paths = detection.source_paths.len() + detection.doc_paths.len();
    if config_path.exists() && !force {
        eprintln!(".iris.toml already exists (use --force to overwrite)");
    } else {
        eprintln!("Generated .iris.toml with {total_paths} paths");
    }

    eprintln!();
    eprintln!("MCP server configs:");
    eprintln!("  ✓ .mcp.json (Claude Code)");
    eprintln!("  ✓ .vscode/mcp.json (VS Code / GitHub Copilot)");
    eprintln!("  ✓ .cursor/mcp.json (Cursor)");
    eprintln!();
    eprintln!(
        "Agent instructions ({} created, {} healed):",
        scaffolded.created, scaffolded.healed
    );
    eprintln!("  ✓ .claude/rules/          (tool scope, playbook)");
    eprintln!("  ✓ .claude/settings.json   (PreToolUse hooks — blocks Grep/Glob/Bash search)");
    eprintln!("  ✓ .github/hooks/          (Copilot CLI + cloud agent hooks — same enforcement)");
    eprintln!("  ✓ .cursor/rules/iris.mdc  (Cursor rules — aggressive advisory)");
    eprintln!("  ✓ .cursor/hooks.json      (Cursor hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ .windsurf/hooks.json    (Windsurf hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ windsurf/rules/iris.md  (Windsurf rules)");
    eprintln!("  ✓ .continue/rules/iris.md (Continue.dev rules)");
    eprintln!("  ✓ .github/copilot-instructions.md");
    eprintln!("  ✓ AGENTS.md               (universal)");
    if scaffolded.custom_rules > 0 {
        eprintln!(
            "  ✓ iris-custom.md           ({} custom rules injected)",
            scaffolded.custom_rules
        );
    }
    if !langs.is_empty() {
        let names: Vec<_> = langs.iter().map(|l| format!("{l:?}")).collect();
        eprintln!(
            "  ✓ iris-lang-rules.md       ({} language playbook)",
            names.join(", ")
        );
    }
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new session in your preferred agent (Claude Code, Cursor, Copilot)");
    eprintln!("  2. iris will auto-index and semantic search tools become available");
    eprintln!("  3. Grep/Glob/Bash search are blocked — agents must use iris tools");
    Ok(())
}

// ---------------------------------------------------------------------------
// iris export / import
// ---------------------------------------------------------------------------

/// `iris export` — export the corpus index to a portable bundle.
pub(crate) async fn cmd_export(
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
        infra::corpus_data_dir_name(corpus_paths)
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
pub(crate) fn cmd_import(
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
        infra::corpus_data_dir_name(corpus_paths)
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

// ---------------------------------------------------------------------------
// iris status / search
// ---------------------------------------------------------------------------

/// `iris status` — show corpus stats from local storage.
///
/// Opens the `SQLite` database directly (no embedding model needed) and
/// displays document counts, corpus roots, data directory, and index info.
/// Falls back to the daemon API if available for richer live status.
#[allow(clippy::too_many_lines)]
pub(crate) async fn cmd_daemon_status() -> Result<()> {
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
        infra::corpus_data_dir_name(&corpus_paths)
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
pub(crate) async fn cmd_daemon_search(
    corpus_paths: &[String],
    query: &str,
    top_k: usize,
) -> Result<()> {
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

// ---------------------------------------------------------------------------
// iris hooks test
// ---------------------------------------------------------------------------

/// `iris hooks test` — validate installed hook files and simulate tool calls.
pub(crate) fn cmd_hooks_test(root: &Path) {
    use std::collections::BTreeMap;

    /// A simulated tool call for testing.
    struct TestCase {
        tool: &'static str,
        args: &'static str,
        should_block: bool,
    }

    let test_cases = &[
        TestCase {
            tool: "Grep",
            args: r#"{"pattern": "fn main"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Glob",
            args: r#"{"pattern": "**/*.rs"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "grep -r TODO ."}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "find . -name '*.rs'"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cat file.rs | grep fn"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "rg pattern src/"}"#,
            should_block: true,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cargo test"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "cargo build"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Bash",
            args: r#"{"command": "git status"}"#,
            should_block: false,
        },
        TestCase {
            tool: "Read",
            args: r#"{"path": "src/main.rs"}"#,
            should_block: false,
        },
    ];

    // ── Check hook files ────────────────────────────────────────────────
    let hook_files: BTreeMap<&str, std::path::PathBuf> = [
        ("Claude Code", root.join(".claude/settings.json")),
        ("Copilot CLI", root.join(".github/hooks/iris-enforce.json")),
        ("Cursor", root.join(".cursor/hooks.json")),
        ("Windsurf", root.join(".windsurf/hooks.json")),
    ]
    .into_iter()
    .collect();

    eprintln!("Hook files:");
    let mut any_missing = false;
    for (platform, path) in &hook_files {
        if path.exists() {
            // Validate JSON structure.
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(_) => eprintln!(
                        "  ✓ {platform:<14} {}",
                        path.strip_prefix(root).unwrap_or(path).display()
                    ),
                    Err(e) => eprintln!("  ✗ {platform:<14} invalid JSON: {e}"),
                },
                Err(e) => eprintln!("  ✗ {platform:<14} read error: {e}"),
            }
        } else {
            eprintln!("  ✗ {platform:<14} not found (run `iris init`)");
            any_missing = true;
        }
    }

    // ── Check advisory files ────────────────────────────────────────────
    eprintln!();
    eprintln!("Advisory files:");
    let advisory_files: &[(&str, &str)] = &[
        ("Claude rules", ".claude/rules/iris-scope.md"),
        ("Cursor rules", ".cursor/rules/iris.mdc"),
        ("Windsurf rules", "windsurf/rules/iris.md"),
        ("Continue rules", ".continue/rules/iris.md"),
        ("Copilot instructions", ".github/copilot-instructions.md"),
        ("AGENTS.md", "AGENTS.md"),
        ("Language rules", ".claude/rules/iris-lang-rules.md"),
        ("Custom rules", ".claude/rules/iris-custom.md"),
    ];
    for (name, rel_path) in advisory_files {
        let path = root.join(rel_path);
        if path.exists() {
            eprintln!("  ✓ {name:<22} {rel_path}");
        } else {
            eprintln!("  · {name:<22} not present");
        }
    }

    if any_missing {
        eprintln!();
        eprintln!("⚠ Some hook files are missing. Run `iris init` to generate them.");
    }

    // ── Simulate tool calls ─────────────────────────────────────────────
    eprintln!();
    eprintln!("Simulated tool calls:");
    let mut pass = 0;
    let mut fail = 0;
    for tc in test_cases {
        let expected = if tc.should_block { "BLOCK" } else { "ALLOW" };
        let actual_blocked = would_hook_block(tc.tool, tc.args);
        let actual = if actual_blocked { "BLOCK" } else { "ALLOW" };
        let ok = actual_blocked == tc.should_block;
        if ok {
            pass += 1;
        } else {
            fail += 1;
        }

        // Truncate args for display.
        let cmd_display = if tc.tool == "Bash" {
            serde_json::from_str::<serde_json::Value>(tc.args)
                .ok()
                .and_then(|v| v["command"].as_str().map(String::from))
                .unwrap_or_else(|| tc.args.to_string())
        } else {
            tc.tool.to_string()
        };

        let icon = if ok { "✓" } else { "✗" };
        let expect_str = if ok {
            String::new()
        } else {
            format!(" (expected {expected})")
        };
        eprintln!("  {icon} {cmd_display:<40} → {actual}{expect_str}");
    }

    eprintln!();
    eprintln!("{pass} passed, {fail} failed");

    if fail > 0 {
        eprintln!("⚠ Some simulations did not match expected behavior.");
    }
}

/// Check if the Claude Code hooks would block a given tool/args combination.
///
/// This simulates the `PreToolUse` hook logic from `.claude/settings.json`.
fn would_hook_block(tool_name: &str, tool_args: &str) -> bool {
    let search_tools = ["grep", "Grep", "egrep", "fgrep", "rg", "ag", "ack"];
    let find_tools = ["find", "fd"];

    match tool_name {
        "Grep" | "Glob" => true,
        "Bash" => {
            let cmd = serde_json::from_str::<serde_json::Value>(tool_args)
                .ok()
                .and_then(|v| v["command"].as_str().map(String::from))
                .unwrap_or_default();

            // Direct search command.
            let first_word = cmd.split_whitespace().next().unwrap_or("");
            if search_tools.contains(&first_word) || find_tools.contains(&first_word) {
                return true;
            }

            // Piped to search tool.
            if cmd.contains('|') {
                for tool in &search_tools {
                    if cmd.contains(&format!("| {tool}")) || cmd.contains(&format!("|{tool}")) {
                        return true;
                    }
                }
            }

            false
        }
        _ => false,
    }
}
