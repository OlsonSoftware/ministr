//! iris-cli — binary entry point for the iris MCP server.

use miette::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("iris starting");

    Ok(())
}
