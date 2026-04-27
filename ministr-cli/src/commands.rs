//! CLI subcommand implementations for the ministr CLI.
//!
//! Each `pub(crate)` function corresponds to a CLI subcommand dispatched from
//! [`main`](crate::main). This module keeps `main.rs` focused on argument
//! parsing and dispatch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt as _;

use ministr_core::index::VectorIndex as _;
use ministr_core::index::VectorIndexLoad as _;

use crate::infra;
use crate::ingestion;

// ---------------------------------------------------------------------------
// ministr serve --transport stdio
// ---------------------------------------------------------------------------

/// `ministr serve --transport stdio` — MCP server over stdin/stdout.
///
/// On first invocation for a corpus, acquires an exclusive lock and starts
/// as the primary (stdio + HTTP listener for secondaries). On subsequent
/// invocations, detects the primary and runs as a transparent proxy.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) async fn cmd_serve_stdio(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    resolved_model: &str,
    repo_config_dir: Option<&Path>,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
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
            let (server, ctx, _coherence_handle) = infra::build_server(
                corpus_paths,
                config_path,
                config,
                Some(resolved_model),
                resolved_dimension,
                rerank_depth,
            )
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

            // Watch .ministr.toml for path changes and re-index automatically.
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
            tracing::info!("ministr shutting down");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// ministr serve --transport http
// ---------------------------------------------------------------------------

/// `ministr serve --transport http` — Streamable HTTP MCP server.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_serve_http(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    host: &str,
    port: u16,
    oauth_config: Option<ministr_mcp::auth::OAuthConfig>,
    resolved_model: &str,
    repo_config_dir: Option<&Path>,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let (server, ctx, _coherence_handle) = infra::build_server(
        corpus_paths,
        config_path,
        config,
        Some(resolved_model),
        resolved_dimension,
        rerank_depth,
    )
    .await?;

    let ingestion_progress = server.ingestion_progress_arc();

    // Extract Arcs before moving server into the factory closure.
    let a2a_service = server.service_arc();
    let a2a_registry = server.registry_arc();

    // Each HTTP session gets its own MinistrServer clone.
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
    let a2a_state = ministr_mcp::a2a::A2aState {
        service: a2a_service,
        registry: a2a_registry,
        tasks: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };
    let a2a_router = ministr_mcp::a2a::a2a_routes(a2a_state);

    // Bundle-serving endpoints (read-only, public).
    let bundle_state = ministr_mcp::bundle_routes::BundleState {
        corpus_dir: ctx.corpus_dir.clone(),
        model_name: resolved_model.to_string(),
        storage: Arc::clone(&ctx.storage),
    };
    let bundle_router = ministr_mcp::bundle_routes::bundle_routes(bundle_state);

    let app = if let Some(oauth_cfg) = oauth_config {
        tracing::info!("OAuth 2.1 authentication enabled");
        let store = ministr_mcp::auth::OAuthStore::new(oauth_cfg);
        let protected = ministr_mcp::auth::protected_router(mcp_router, store.clone());
        // Bundle endpoints require ministr:bundle:read scope when OAuth is active.
        let protected_bundles =
            ministr_mcp::auth::scope_protected_router(bundle_router, store, "ministr:bundle:read");
        a2a_router.merge(protected).merge(protected_bundles)
    } else {
        a2a_router.merge(mcp_router).merge(bundle_router)
    };

    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to bind HTTP server to {bind_addr}"))?;

    tracing::info!(address = %bind_addr, "ministr HTTP server listening");

    // Ingest in background AFTER the HTTP server is bound.
    infra::spawn_background_ingestion(corpus_paths, git_includes, &ctx, &ingestion_progress);

    // Watch .ministr.toml for path changes and re-index automatically.
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

    tracing::info!("ministr shutting down");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr serve --proxy
// ---------------------------------------------------------------------------

/// `ministr serve --proxy` — thin MCP proxy over stdin/stdout.
///
/// Connects to the ministr daemon at `~/.ministr/ministrd.sock` and proxies all
/// tool calls. No ONNX model, no indexes, no `SQLite` — just HTTP over UDS.
pub(crate) async fn cmd_serve_proxy_stdio(corpus_paths: &[String]) -> Result<()> {
    eprintln!(
        "ministr: proxy starting with {} corpus paths",
        corpus_paths.len()
    );

    // Pre-register corpus with daemon before starting MCP handshake.
    let client = ministr_api::client::DaemonClient::new();
    match client.register_corpus(corpus_paths).await {
        Ok(resp) => {
            eprintln!(
                "ministr: corpus {} registered (indexing_started={})",
                resp.corpus_id, resp.indexing_started
            );
        }
        Err(e) => {
            eprintln!("ministr: warning — corpus registration failed: {e}");
        }
    }

    eprintln!("ministr: starting MCP proxy on stdio");
    let proxy = ministr_mcp::proxy::ProxyServer::new(corpus_paths.to_vec());

    // Eagerly create a daemon session so the GUI shows it immediately.
    if let Err(e) = proxy.initialize().await {
        eprintln!("ministr: warning — eager session init failed: {e}");
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
// ministr index
// ---------------------------------------------------------------------------

/// `ministr index` — run ingestion synchronously and exit.
pub(crate) async fn cmd_index(
    corpus_paths: &[String],
    git_includes: &[ministr_core::config::GitInclude],
    config_path: &Path,
    config: &ministr_core::config::MinistrConfig,
    resolved_model: &str,
    resolved_dimension: Option<usize>,
    rerank_depth: Option<usize>,
) -> Result<()> {
    tracing::info!(
        corpus_count = corpus_paths.len(),
        config = %config_path.display(),
        "ministr index — {} corpus path(s)",
        corpus_paths.len()
    );
    for path in corpus_paths {
        tracing::info!(path = %path, "  corpus root");
    }

    if corpus_paths.is_empty() && git_includes.is_empty() {
        tracing::warn!("no corpus paths specified, nothing to index");
        return Ok(());
    }

    let ctx = infra::init_infrastructure(
        corpus_paths,
        config,
        Some(resolved_model),
        resolved_dimension,
        rerank_depth,
    )
    .await?;

    let progress = Arc::new(ministr_core::ingestion::IngestionProgress::new());
    ingestion::run_corpus_ingestion(corpus_paths, git_includes, &ctx, &progress).await?;

    tracing::info!("indexing complete");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr init
// ---------------------------------------------------------------------------

/// `ministr init` — detect project structure and generate `.ministr.toml`.
pub(crate) fn cmd_init(root: &Path, force: bool) -> Result<()> {
    let detection = ministr_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .ministr.toml")?;

    // Scaffold agent config files (Claude Code hooks, Cursor rules, etc.).
    let scaffolded = ministr_core::scaffold::scaffold_agent_config(root);

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
    let config_path = root.join(".ministr.toml");
    let total_paths = detection.source_paths.len() + detection.doc_paths.len();
    if config_path.exists() && !force {
        eprintln!(".ministr.toml already exists (use --force to overwrite)");
    } else {
        eprintln!("Generated .ministr.toml with {total_paths} paths");
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
    eprintln!("  ✓ .cursor/rules/ministr.mdc  (Cursor rules — aggressive advisory)");
    eprintln!("  ✓ .cursor/hooks.json      (Cursor hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ .windsurf/hooks.json    (Windsurf hooks — blocks shell search/find/pipes)");
    eprintln!("  ✓ windsurf/rules/ministr.md  (Windsurf rules)");
    eprintln!("  ✓ .continue/rules/ministr.md (Continue.dev rules)");
    eprintln!("  ✓ .github/copilot-instructions.md");
    eprintln!("  ✓ AGENTS.md               (universal)");
    if scaffolded.custom_rules > 0 {
        eprintln!(
            "  ✓ ministr-custom.md           ({} custom rules injected)",
            scaffolded.custom_rules
        );
    }
    if !langs.is_empty() {
        let names: Vec<_> = langs.iter().map(|l| format!("{l:?}")).collect();
        eprintln!(
            "  ✓ ministr-lang-rules.md       ({} language playbook)",
            names.join(", ")
        );
    }
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new session in your preferred agent (Claude Code, Cursor, Copilot)");
    eprintln!("  2. ministr will auto-index and semantic search tools become available");
    eprintln!("  3. Grep/Glob/Bash search are blocked — agents must use ministr tools");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr init --interactive
// ---------------------------------------------------------------------------

/// `ministr init --interactive` — guided setup wizard.
#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_init_interactive(root: &Path, force: bool) -> Result<()> {
    use dialoguer::{Confirm, MultiSelect, Select};

    eprintln!("ministr interactive setup wizard\n");

    // Step 1: detect project and confirm type
    let detection = ministr_core::init::detect_project(root);

    let confirmed_type = {
        use ministr_core::init::ProjectType;
        let types = &[
            ProjectType::Monorepo,
            ProjectType::Library,
            ProjectType::Cli,
            ProjectType::WebApp,
            ProjectType::Api,
            ProjectType::Unknown,
        ];
        let labels: Vec<String> = types.iter().map(std::string::ToString::to_string).collect();
        let detected_idx = types
            .iter()
            .position(|t| *t == detection.project_type)
            .unwrap_or(types.len() - 1);
        let idx = Select::new()
            .with_prompt(format!(
                "Detected project type: {}. Confirm or change",
                detection.project_type
            ))
            .items(&labels)
            .default(detected_idx)
            .interact()
            .into_diagnostic()?;
        types[idx]
    };

    eprintln!("  Project type: {confirmed_type}");

    // Step 2: choose agent platforms
    let platforms = &[
        "Claude Code (.claude/rules/, settings.json hooks)",
        "Cursor (.cursor/rules/, hooks.json)",
        "GitHub Copilot (.github/hooks/, copilot-instructions.md)",
        "Windsurf (.windsurf/hooks.json, rules/)",
        "Continue.dev (.continue/rules/)",
    ];
    let platform_defaults = vec![true, true, true, true, true];
    let selected_platforms = MultiSelect::new()
        .with_prompt("Agent platforms to configure")
        .items(platforms)
        .defaults(&platform_defaults)
        .interact()
        .into_diagnostic()?;

    let platform_names: Vec<&str> = selected_platforms
        .iter()
        .map(|&i| match i {
            0 => "claude",
            1 => "cursor",
            2 => "copilot",
            3 => "windsurf",
            4 => "continue",
            _ => "unknown",
        })
        .collect();
    eprintln!("  Platforms: {}", platform_names.join(", "));

    // Step 3: hook strictness
    let strictness_levels = &[
        "strict — block Grep/Glob/Bash search, enforce ministr tools",
        "moderate — warn on Grep/Glob/Bash, allow with confirmation",
        "advisory — suggest ministr tools, never block",
    ];
    let strictness_idx = Select::new()
        .with_prompt("Hook strictness level")
        .items(strictness_levels)
        .default(0)
        .interact()
        .into_diagnostic()?;

    let strictness = match strictness_idx {
        0 => "strict",
        1 => "moderate",
        _ => "advisory",
    };
    eprintln!("  Strictness: {strictness}");

    // Step 4: confirm and write
    eprintln!();
    let proceed = Confirm::new()
        .with_prompt("Write .ministr.toml and scaffold agent configs?")
        .default(true)
        .interact()
        .into_diagnostic()?;

    if !proceed {
        eprintln!("Aborted.");
        return Ok(());
    }

    // Write config (reuses the non-interactive path)
    let _detection = ministr_core::init::write_config(root, force)
        .into_diagnostic()
        .wrap_err("failed to generate .ministr.toml")?;

    // Scaffold agent configs
    let scaffolded = ministr_core::scaffold::scaffold_agent_config(root);

    eprintln!();
    eprintln!(
        "Done! Created .ministr.toml and scaffolded {} files ({} created, {} healed).",
        scaffolded.touched(),
        scaffolded.created,
        scaffolded.healed,
    );
    eprintln!();
    eprintln!("Selected platforms: {}", platform_names.join(", "));
    eprintln!("Hook strictness: {strictness}");
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Start a new session in your preferred agent");
    eprintln!("  2. ministr will auto-index and semantic search tools become available");
    eprintln!(
        "  3. Grep/Glob/Bash search are {} by hooks",
        match strictness {
            "strict" => "blocked",
            "moderate" => "warned",
            _ => "unaffected",
        }
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// ministr export / import
// ---------------------------------------------------------------------------

/// `ministr export` — export the corpus index to a portable bundle.
pub(crate) async fn cmd_export(
    corpus_paths: &[String],
    config: &ministr_core::config::MinistrConfig,
    resolved_model: &str,
    output: Option<&Path>,
) -> Result<()> {
    use ministr_core::bundle::{
        self, BUNDLE_FORMAT_VERSION, BundleCorpusRoot, BundleManifest, compute_bundle_version,
    };
    use ministr_core::storage::Storage as _;

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
            "no indexed corpus found at {}. Run `ministr index` first.",
            corpus_dir.display()
        );
    }

    // Open storage (no embedder needed for export).
    let storage = ministr_core::storage::SqliteStorage::open(&db_path)
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
        match ministr_core::index::HnswIndex::load(&index_dir) {
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
                .and_then(|p| ministr_core::git::local_head_sha(std::path::Path::new(p)))
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
            let filename = format!("{corpus_name}.ministr-index");
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

/// `ministr import` — import a `.ministr-index` bundle into local storage.
pub(crate) fn cmd_import(
    corpus_paths: &[String],
    config: &ministr_core::config::MinistrConfig,
    bundle_path: &Path,
) -> Result<()> {
    use ministr_core::bundle;

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
// ministr status / search
// ---------------------------------------------------------------------------

/// `ministr status` — show corpus stats from local storage.
///
/// Opens the `SQLite` database directly (no embedding model needed) and
/// displays document counts, corpus roots, data directory, and index info.
/// Falls back to the daemon API if available for richer live status.
#[allow(clippy::too_many_lines)]
pub(crate) async fn cmd_daemon_status() -> Result<()> {
    use ministr_core::storage::Storage as _;

    // Try daemon first for live status.
    let client = ministr_api::client::DaemonClient::new();
    if client.is_available()
        && let Ok(status) = client.status().await
    {
        eprintln!("ministr daemon v{}", status.version);
        eprintln!("  Uptime:    {}s", status.uptime_secs);
        eprintln!("  Memory:    {:.0} MB", status.memory_mb);
        eprintln!(
            "  Model:     {} ({}d)",
            status.model, status.model_dimension
        );
        eprintln!("  Corpora:   {}", status.corpora.len());
        for c in &status.corpora {
            // Show the human-readable display name; fall back to the id
            // only if the daemon predates the `display_name` field.
            let label = if c.display_name.is_empty() {
                c.id.as_str()
            } else {
                c.display_name.as_str()
            };
            eprintln!(
                "    {label} — {} files, {} sections, {} embeddings [{}]",
                c.files_indexed,
                c.sections_count,
                c.embeddings_count,
                match &c.status {
                    ministr_api::corpus::IndexingStatus::Idle => "idle".to_string(),
                    ministr_api::corpus::IndexingStatus::Indexing {
                        files_done,
                        files_total,
                    } => format!("indexing {files_done}/{files_total}"),
                    ministr_api::corpus::IndexingStatus::Error { message } =>
                        format!("error: {message}"),
                }
            );
        }
        return Ok(());
    }

    // Daemon not available — show local storage stats.
    let config_path = ministr_core::config::MinistrConfig::default_path();
    let config = ministr_core::config::MinistrConfig::load(&config_path)
        .into_diagnostic()
        .wrap_err("failed to load config")?;

    let cwd = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("failed to get current directory")?;
    let corpus_config = ministr_core::config::RepoConfig::discover(&cwd)
        .into_diagnostic()
        .wrap_err("failed to read .ministr.toml")?;

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

    eprintln!("ministr status (local)");
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
        eprintln!("  No index found. Run `ministr serve` or `ministr index` to build one.");
        return Ok(());
    }

    let storage = ministr_core::storage::SqliteStorage::open(&db_path)
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
    if index_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&index_dir)
    {
        let total_bytes: u64 = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum();
        #[allow(clippy::cast_precision_loss)]
        let mb = total_bytes as f64 / 1_048_576.0;
        eprintln!("  Index size: {mb:.1} MB");
    }

    Ok(())
}

/// `ministr search` — search the corpus via the daemon.
pub(crate) async fn cmd_daemon_search(
    corpus_paths: &[String],
    query: &str,
    top_k: usize,
) -> Result<()> {
    let client = ministr_api::client::DaemonClient::new();
    if !client.is_available() {
        miette::bail!(
            "ministr daemon is not running (no endpoint at {})",
            client.endpoint()
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
// ministr hooks test
// ---------------------------------------------------------------------------

/// `ministr hooks test` — validate installed hook files and simulate tool calls.
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
        (
            "Copilot CLI",
            root.join(".github/hooks/ministr-enforce.json"),
        ),
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
            eprintln!("  ✗ {platform:<14} not found (run `ministr init`)");
            any_missing = true;
        }
    }

    // ── Check advisory files ────────────────────────────────────────────
    eprintln!();
    eprintln!("Advisory files:");
    let advisory_files: &[(&str, &str)] = &[
        ("Claude rules", ".claude/rules/ministr-scope.md"),
        ("Cursor rules", ".cursor/rules/ministr.mdc"),
        ("Windsurf rules", "windsurf/rules/ministr.md"),
        ("Continue rules", ".continue/rules/ministr.md"),
        ("Copilot instructions", ".github/copilot-instructions.md"),
        ("AGENTS.md", "AGENTS.md"),
        ("Language rules", ".claude/rules/ministr-lang-rules.md"),
        ("Custom rules", ".claude/rules/ministr-custom.md"),
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
        eprintln!("⚠ Some hook files are missing. Run `ministr init` to generate them.");
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

// ---------------------------------------------------------------------------
// ministr setup
// ---------------------------------------------------------------------------

/// `ministr setup` — add the `ministr` binary's directory to the user's PATH.
///
/// Wraps the `onpath` crate so installer scripts (`install.sh`, the Tauri
/// first-run flow) don't have to hand-roll cross-shell rc-file edits. On
/// Unix, writes to bash / zsh / fish / nushell / `PowerShell` / tcsh / xonsh
/// rc files for shells the user actually has installed. On Windows, writes
/// the per-user `HKCU\Environment\PATH` registry entry — same surface
/// `install.ps1` and the Tauri NSIS installer hook target, so re-running is
/// idempotent regardless of how the user got here.
///
/// `bin_dir` defaults to the parent of the running `ministr` binary so a
/// fresh `~/.ministr/bin/ministr setup` after `install.sh` Just Works
/// without the user having to know the path.
///
/// `uninstall=true` calls `onpath::remove` instead of `add` — used by the
/// NSIS uninstaller hook before tearing down the install dir.
pub(crate) fn cmd_setup(bin_dir: Option<&Path>, dry_run: bool, uninstall: bool) -> Result<()> {
    let bin_dir = if let Some(p) = bin_dir {
        p.to_path_buf()
    } else {
        let exe = std::env::current_exe()
            .into_diagnostic()
            .wrap_err("failed to resolve current executable for default --bin-dir")?;
        exe.parent()
            .ok_or_else(|| miette::miette!("running binary has no parent dir; pass --bin-dir"))?
            .to_path_buf()
    };

    let manager = onpath::PathManager::new(&bin_dir, "ministr").dry_run(dry_run);
    let (verb, report) = if uninstall {
        let r = manager
            .remove()
            .into_diagnostic()
            .wrap_err_with(|| format!("onpath failed to remove {} from PATH", bin_dir.display()))?;
        ("remove", r)
    } else {
        let r = manager
            .add()
            .into_diagnostic()
            .wrap_err_with(|| format!("onpath failed to add {} to PATH", bin_dir.display()))?;
        ("add", r)
    };

    // Report (which shells / files were edited) goes to stdout so callers
    // like install.sh can capture it; user-facing reminders go to stderr.
    println!("{report}");

    if dry_run {
        eprintln!("(dry-run — nothing was written)");
    } else if verb == "add" {
        eprintln!();
        eprintln!(
            "Open a new shell (or `source` the modified rc file) for the change to take effect."
        );
    }

    Ok(())
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
