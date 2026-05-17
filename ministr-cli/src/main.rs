//! ministr-cli — binary entry point for the ministr MCP server.
//!
//! Provides subcommands: `serve` (default), `index`, `status`, `search`,
//! `init`, `export`, `import`, and `hooks test`.
//!
//! This module handles CLI argument parsing and dispatch. Implementation
//! lives in:
//! - [`commands`] — subcommand handlers
//! - [`infra`] — shared infrastructure setup (storage, embedder, index)
//! - [`ingestion`] — corpus ingestion orchestration and file watching

mod commands;
mod infra;
mod ingestion;
mod instance;
mod proxy;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::Result;

/// ministr — a code intelligence MCP server for AI coding agents.
///
/// Runs an MCP server that provides intelligent context retrieval
/// tools (survey, read, extract) for a local document corpus.
/// Supports stdio and Streamable HTTP transports.
#[derive(Parser, Debug)]
#[command(name = "ministr", version, about)]
struct Cli {
    /// Corpus sources: local paths, `https://` URLs, or `github://` URLs.
    ///
    /// Accepts multiple values via repeated flags:
    /// `ministr --corpus ./docs --corpus https://docs.rs/serde`
    #[arg(short, long, global = true)]
    corpus: Vec<String>,

    /// Path to config file (default: ~/.ministr/config.toml).
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

        /// Run as a thin proxy to the ministr daemon instead of the monolithic server.
        ///
        /// When enabled, the MCP server connects to the ministr daemon at
        /// `~/.ministr/ministrd.sock` and delegates all indexing and querying.
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

    /// Show daemon status (requires ministr-app to be running).
    Status,

    /// Search the corpus via the daemon (requires ministr-app to be running).
    Search {
        /// Search query.
        query: String,
        /// Maximum results.
        #[arg(short = 'k', long, default_value_t = 10)]
        top_k: usize,
    },

    /// Generate .ministr.toml with auto-detected project settings.
    ///
    /// Scans the current directory for project manifests (Cargo.toml,
    /// package.json, pyproject.toml), detects workspace layouts and
    /// bridge frameworks, and writes a sensible default config.
    Init {
        /// Overwrite existing .ministr.toml if present.
        #[arg(long)]
        force: bool,

        /// Run interactive setup: show the detected project type and
        /// exactly what will be written, then confirm before scaffolding.
        #[arg(long, short)]
        interactive: bool,
    },

    /// Export the corpus index to a portable `.ministr-index` bundle.
    ///
    /// Creates a zstd-compressed archive containing the content database
    /// (with session-local data stripped), HNSW vector index, and metadata
    /// manifest. The bundle can be imported on another machine without
    /// re-parsing or re-embedding.
    Export {
        /// Output file path (default: `<corpus-name>.ministr-index` in current dir).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Import a `.ministr-index` bundle into the local corpus store.
    ///
    /// Decompresses the bundle and loads the content database and HNSW
    /// index into the corpus data directory, ready for querying without
    /// re-indexing.
    Import {
        /// Path to the `.ministr-index` bundle file.
        bundle: PathBuf,
    },

    /// Manage ministr agent hooks.
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Add the `ministr` binary's directory to the user's PATH.
    ///
    /// Detects installed shells (bash, zsh, fish, nushell, `PowerShell`, tcsh,
    /// xonsh) and writes the appropriate rc file edits via the `onpath` crate.
    /// On Windows, writes the per-user `HKCU\Environment\PATH` registry entry.
    ///
    /// Idempotent — re-running won't duplicate entries. Used by `install.sh`
    /// and the Tauri desktop app's first-run setup.
    Setup {
        /// Directory to add to (or remove from) PATH.
        ///
        /// Default: parent of the running `ministr` binary.
        #[arg(long)]
        bin_dir: Option<PathBuf>,

        /// Print what would be edited, don't write.
        #[arg(long)]
        dry_run: bool,

        /// Remove the directory from PATH instead of adding it.
        ///
        /// Used by the NSIS uninstaller hook before tearing down the
        /// install dir. Idempotent — no-op if the dir isn't on PATH.
        #[arg(long)]
        uninstall: bool,
    },
}

/// Subcommands for `ministr hooks`.
#[derive(Debug, Subcommand)]
enum HooksAction {
    /// Test installed hooks by simulating tool calls.
    ///
    /// Checks all agent platform hook files, validates their structure,
    /// and simulates common tool calls to report which would be blocked.
    Test,
}

/// MCP transport mode.
#[derive(Debug, Clone, clap::ValueEnum)]
enum Transport {
    /// JSON-RPC over stdin/stdout (default for local MCP clients).
    Stdio,
    /// Streamable HTTP (MCP spec 2025-03-26) for remote/multi-client deployments.
    Http,
}

impl Default for Command {
    fn default() -> Self {
        Self::Serve {
            transport: Transport::Stdio,
            host: "127.0.0.1".to_string(),
            port: 8080,
            proxy: false,
            oauth: false,
            oauth_issuer: None,
        }
    }
}

/// Resolved configuration from CLI args, config.toml, and .ministr.toml.
struct ResolvedConfig {
    config_path: PathBuf,
    config: ministr_core::config::MinistrConfig,
    cwd: PathBuf,
    corpus_paths: Vec<String>,
    git_includes: Vec<ministr_core::config::GitInclude>,
    resolved_model: String,
    repo_config_dir: Option<PathBuf>,
    /// Matryoshka truncation dimension from `.ministr.toml` `[corpus] dimension`.
    resolved_dimension: Option<usize>,
    /// Two-stage rerank depth from `.ministr.toml` `[corpus] rerank_depth`.
    rerank_depth: Option<usize>,
}

/// Load global config, discover per-repo .ministr.toml, and resolve corpus paths.
fn resolve_config(cli: &Cli) -> Result<ResolvedConfig> {
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(ministr_core::config::MinistrConfig::default_path);
    let config = ministr_core::config::MinistrConfig::load(&config_path).map_err(|e| {
        miette::miette!("failed to load config from {}: {e}", config_path.display())
    })?;

    let cwd = std::env::current_dir()
        .map_err(|e| miette::miette!("failed to get current directory: {e}"))?;
    let corpus_config = ministr_core::config::RepoConfig::discover(&cwd)
        .map_err(|e| miette::miette!("failed to read .ministr.toml: {e}"))?;

    if let Some((ref config_dir, ref cc)) = corpus_config {
        let config_file = config_dir.join(ministr_core::config::CORPUS_CONFIG_FILENAME);
        tracing::info!(
            config = %config_file.display(),
            paths = cc.corpus.paths.len(),
            git_repos = cc.corpus.git.len(),
            ignore_patterns = cc.corpus.ignore.len(),
            "loaded .ministr.toml"
        );
        for w in &cc.validate(config_dir) {
            tracing::warn!("{w}");
        }
    } else {
        tracing::info!("no .ministr.toml found — using CLI args or config.toml defaults");
    }

    let corpus_paths: Vec<String> = if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else if cli.corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli.corpus.clone()
    };

    let repo_config_dir = corpus_config.as_ref().map(|(dir, _)| dir.clone());

    let git_includes = corpus_config
        .as_ref()
        .map(|(_, cc)| cc.corpus.git.clone())
        .unwrap_or_default();

    let resolved_model = ministr_core::config::resolve_model_name(
        corpus_config.as_ref().map(|(_, cc)| cc),
        None,
        &config,
    );

    let resolved_dimension = corpus_config
        .as_ref()
        .and_then(|(_, cc)| cc.corpus.dimension);
    let rerank_depth = corpus_config
        .as_ref()
        .and_then(|(_, cc)| cc.corpus.rerank_depth);

    Ok(ResolvedConfig {
        config_path,
        config,
        cwd,
        corpus_paths,
        git_includes,
        resolved_model,
        repo_config_dir,
        resolved_dimension,
        rerank_depth,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().build())
    }))
    .expect("miette hook should be set once");

    ministr_core::tracing::init_tracing();

    let command = cli.command.take().unwrap_or_default();

    // `ministr setup` runs *before* resolve_config() so a malformed
    // .ministr.toml in cwd can't lock the user out of the subcommand that
    // gets `ministr` on PATH. Setup needs no corpus paths, no model
    // resolution, no repo config — it just edits shell rc files /
    // HKCU\Environment\PATH.
    if let Command::Setup {
        bin_dir,
        dry_run,
        uninstall,
    } = command
    {
        return commands::cmd_setup(bin_dir.as_deref(), dry_run, uninstall);
    }

    let rc = resolve_config(&cli)?;

    dispatch(command, rc).await
}

#[allow(clippy::too_many_lines)]
async fn dispatch(command: Command, rc: ResolvedConfig) -> Result<()> {
    match command {
        Command::Serve {
            transport,
            host,
            port,
            proxy,
            oauth,
            oauth_issuer,
        } => {
            // Create-only: a routine MCP start must never silently
            // rewrite an existing .claude/settings.json hooks block.
            // Healing is reserved for explicit `ministr init` / the
            // desktop Repair action (run with a known-current binary).
            ministr_core::scaffold::scaffold_agent_config_with(
                &rc.cwd,
                ministr_core::scaffold::ScaffoldMode::CreateOnly,
            );

            match transport {
                Transport::Stdio if proxy => {
                    commands::cmd_serve_proxy_stdio(&rc.corpus_paths).await
                }
                Transport::Stdio
                    if !proxy && ministr_api::client::DaemonClient::new().is_healthy().await =>
                {
                    eprintln!(
                        "ministr: daemon detected at ~/.ministr/ministrd.sock — running as proxy"
                    );
                    commands::cmd_serve_proxy_stdio(&rc.corpus_paths).await
                }
                Transport::Stdio => {
                    commands::cmd_serve_stdio(
                        &rc.corpus_paths,
                        &rc.git_includes,
                        &rc.config_path,
                        &rc.config,
                        &rc.resolved_model,
                        rc.repo_config_dir.as_deref(),
                        rc.resolved_dimension,
                        rc.rerank_depth,
                    )
                    .await
                }
                Transport::Http => {
                    let oauth_config = if oauth {
                        Some(ministr_mcp::auth::OAuthConfig {
                            issuer: oauth_issuer.unwrap_or_else(|| format!("http://{host}:{port}")),
                            ..ministr_mcp::auth::OAuthConfig::default()
                        })
                    } else {
                        None
                    };
                    commands::cmd_serve_http(
                        &rc.corpus_paths,
                        &rc.git_includes,
                        &rc.config_path,
                        &rc.config,
                        &host,
                        port,
                        oauth_config,
                        &rc.resolved_model,
                        rc.repo_config_dir.as_deref(),
                        rc.resolved_dimension,
                        rc.rerank_depth,
                    )
                    .await
                }
            }
        }
        Command::Index => {
            commands::cmd_index(
                &rc.corpus_paths,
                &rc.git_includes,
                &rc.config_path,
                &rc.config,
                &rc.resolved_model,
                rc.resolved_dimension,
                rc.rerank_depth,
            )
            .await
        }
        Command::Status => commands::cmd_daemon_status().await,
        Command::Search { query, top_k } => {
            commands::cmd_daemon_search(&rc.corpus_paths, &query, top_k).await
        }
        Command::Init { force, interactive } => {
            if interactive {
                commands::cmd_init_interactive(&rc.cwd, force)
            } else {
                commands::cmd_init(&rc.cwd, force)
            }
        }
        Command::Export { output } => {
            commands::cmd_export(
                &rc.corpus_paths,
                &rc.config,
                &rc.resolved_model,
                output.as_deref(),
            )
            .await
        }
        Command::Import { bundle } => commands::cmd_import(&rc.corpus_paths, &rc.config, &bundle),
        Command::Hooks { action } => match action {
            HooksAction::Test => {
                commands::cmd_hooks_test(&rc.cwd);
                Ok(())
            }
        },
        Command::Setup { .. } => {
            unreachable!("ministr setup is dispatched before resolve_config in main()")
        }
    }
}
