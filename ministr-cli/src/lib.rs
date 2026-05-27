//! ministr-cli library surface.
//!
//! Most of the CLI's logic lives in modules under `src/`. The binary
//! at `src/main.rs` is a thin clap-dispatch wrapper over these modules.
//!
//! F31.2b also exposes [`commands::cmd_serve_http`] as a library entry
//! point so the proprietary `ministr-cloud-tools` binary can run the
//! same serve flow with a [`ministr_api::CloudRouterMounter`] wired in
//! (where the public `ministr` binary calls it with `mounter = None`).

// The cmd_* helpers are dispatcher entry points that just surface
// `miette::Result` from the underlying calls. Adding `# Errors` /
// `# Panics` doc sections to each one would just paraphrase the
// propagated error without adding signal — the bin is the only caller
// and treats the whole result as "exit code".
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod commands;
pub mod infra;
pub mod ingestion;
pub mod worker;
