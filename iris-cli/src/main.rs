//! iris-cli — binary entry point for the iris MCP server.

use miette::Result;

#[tokio::main]
async fn main() -> Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().build())
    }))
    .expect("miette hook should be set once");

    iris_core::tracing::init_tracing();

    tracing::info!("iris starting");

    Ok(())
}
