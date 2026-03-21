//! iris-cli — binary entry point for the iris MCP server.
//!
//! Parses command-line arguments, loads configuration, initializes tracing,
//! and starts the MCP server over stdio transport.

use std::path::PathBuf;

use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr};
use rmcp::ServiceExt;

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
    let _config = iris_core::config::IrisConfig::load(&config_path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to load config from {}", config_path.display()))?;

    tracing::info!(
        corpus = ?cli.corpus,
        config = %config_path.display(),
        "iris starting"
    );

    // Create the MCP server and serve over stdio.
    let server = iris_mcp::server::IrisServer::new();
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .into_diagnostic()
        .wrap_err("failed to start MCP stdio transport")?;

    // Wait for the service to shut down.
    service
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("MCP server exited with error")?;

    tracing::info!("iris shutting down");
    Ok(())
}
