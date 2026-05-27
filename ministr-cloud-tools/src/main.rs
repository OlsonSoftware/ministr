//! `ministr-cloud-tools` binary entry point.
//!
//! Thin wrapper over [`ministr_cloud_tools::run`] — the actual CLI
//! surface lives in the crate's `[lib]` target so the private-repo
//! `ministr-cli-cloud` binary (F31.3) can call the same code path
//! without duplicating ~750 lines of clap + dispatch + serve logic.

use miette::Result;

#[tokio::main]
async fn main() -> Result<()> {
    ministr_cloud_tools::run().await
}
