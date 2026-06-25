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
//!
//! Cloud-operator subcommands (atlas, audit, api-keys, cloud) moved to
//! the `ministr-cloud-tools` crate in — run `cargo run -p
//! ministr-cloud-tools -- <subcommand>` for the proprietary operator CLI.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::Result;

use ministr_cli::{ResolvedConfig, commands, resolve_config};

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

        /// Deprecated, no-op (kept for backward compatibility).
        ///
        /// stdio now ALWAYS runs as a thin proxy to the ministr daemon
        /// (auto-spawned if not running); the old monolithic server was
        /// removed. This flag is accepted but has no effect.
        #[arg(long, hide = true)]
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

    /// Internal: run the headless ministr daemon. Auto-spawned by the
    /// MCP proxy (and desktop app); not intended for direct use.
    #[command(hide = true, name = "__daemon")]
    Daemon,

    /// Show daemon status (requires the ministr daemon to be running).
    Status,

    /// Search the corpus via the daemon (requires the ministr daemon to
    /// be running).
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

        /// Opt in to exec-only steering: the scaffolded `PreToolUse` hook
        /// denies the raw Bash tool and redirects the agent to the
        /// recorded `ministr_run` MCP tool family. Reversible — delete
        /// `.claude/hooks/ministr-exec-only` to turn it off.
        #[arg(long)]
        exec_only: bool,
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

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    // rustls 0.23 requires the process to install a default
    // CryptoProvider before any TLS work runs. The workspace pulls in
    // both `ring` and `aws-lc-rs` transitively (via reqwest +
    // rustls-platform-verifier + tokio-postgres-rustls), so rustls
    // refuses to auto-pick. Use `aws_lc_rs` — same crypto stack the
    // platform-verifier crate already brings in, so we don't double
    // the binary size with `ring`. Idempotent: the panic is downgraded
    // to a log because a second call (in tests / a re-entrant runtime)
    // would otherwise crash on `unwrap`.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        // A provider was already installed by an earlier call site
        // (e.g. a library's `static`-init path). Safe to ignore.
    }

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

    // `ministr __daemon` is the headless daemon host (auto-spawned by the
    // MCP proxy / desktop app). It runs *before* resolve_config() too:
    // it serves every corpus over the IPC endpoint and must not depend on
    // the spawning process's cwd or a `.ministr.toml` there.
    if let Command::Daemon = command {
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(ministr_core::config::MinistrConfig::default_path);
        // `load` already returns the default when the file is simply
        // absent; an `Err` therefore means the file IS present but
        // unreadable/invalid. Silently defaulting there would start the
        // daemon with the wrong data_dir/model and no clue why — so make
        // that case loud rather than swallowing it.
        let config = match ministr_core::config::MinistrConfig::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %e,
                    "config file present but invalid — starting daemon with DEFAULT \
                     settings; fix the config and restart the daemon"
                );
                ministr_core::config::MinistrConfig::default()
            }
        };
        return ministr_daemon::bootstrap::run(config)
            .await
            .map_err(|e| miette::miette!("daemon exited: {e}"));
    }

    let rc = resolve_config(&cli.corpus, cli.config.as_deref())?;

    dispatch(command, rc).await
}

/// `ministr init` dispatch: scaffold, then apply the exec-only opt-in.
fn dispatch_init(
    cwd: &std::path::Path,
    force: bool,
    interactive: bool,
    exec_only: bool,
) -> Result<()> {
    if interactive {
        commands::cmd_init_interactive(cwd, force)?;
    } else {
        commands::cmd_init(cwd, force)?;
    }
    if exec_only {
        commands::cmd_enable_exec_only(cwd)?;
    }
    Ok(())
}

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
                Transport::Stdio => {
                    // Always a thin proxy. The proxy auto-starts the
                    // headless daemon (self-exec `ministr __daemon`) if
                    // none is running, so shared-daemon is the single
                    // architecture — there is no separate monolithic
                    // per-corpus server. `--proxy` is now implicit; the
                    // flag is retained only for backward compatibility.
                    let _ = proxy;
                    commands::cmd_serve_proxy_stdio(&rc.corpus_paths, &rc.linked).await
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
                        // MIT `ministr` binary passes None;
                        // the inline cloud branch in cmd_serve_http
                        // still runs unchanged. `ministr-cloud-tools
                        // serve` (chunk B) passes Some(&mounter).
                        None,
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
        Command::Init {
            force,
            interactive,
            exec_only,
        } => dispatch_init(&rc.cwd, force, interactive, exec_only),
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
        Command::Daemon => {
            unreachable!("ministr __daemon is dispatched before resolve_config in main()")
        }
    }
}
