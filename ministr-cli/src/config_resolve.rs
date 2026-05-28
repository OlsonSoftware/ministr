//! Shared `resolve_config` + `ResolvedConfig` exposed for reuse by
//! `ministr-cloud-tools` (F31.2b-ii-B).
//!
//! Previously inlined as private items in `src/main.rs`; relocated here
//! so both the MIT `ministr` binary and the proprietary
//! `ministr-cloud-tools serve` subcommand can build the same input bag
//! for [`crate::commands::cmd_serve_http`] (and the other `cmd_*`
//! entrypoints that need a resolved corpus list).

use std::path::{Path, PathBuf};

use miette::Result;

/// Resolved configuration from CLI args, `config.toml`, and `.ministr.toml`.
pub struct ResolvedConfig {
    pub config_path: PathBuf,
    pub config: ministr_core::config::MinistrConfig,
    pub cwd: PathBuf,
    pub corpus_paths: Vec<String>,
    /// Projects linked into this workspace via `.ministr.toml` `[[linked]]`.
    pub linked: Vec<ministr_core::config::ResolvedLinkedProject>,
    pub git_includes: Vec<ministr_core::config::GitInclude>,
    pub resolved_model: String,
    pub repo_config_dir: Option<PathBuf>,
    /// Matryoshka truncation dimension from `.ministr.toml` `[corpus] dimension`.
    pub resolved_dimension: Option<usize>,
    /// Two-stage rerank depth from `.ministr.toml` `[corpus] rerank_depth`.
    pub rerank_depth: Option<usize>,
}

/// Load global config, discover per-repo `.ministr.toml`, and resolve corpus paths.
///
/// `cli_corpus` mirrors the `--corpus` repeatable flag (empty when none
/// supplied). `cli_config` mirrors `--config` (None ⇒ default path).
pub fn resolve_config(cli_corpus: &[String], cli_config: Option<&Path>) -> Result<ResolvedConfig> {
    let config_path = cli_config.map_or_else(
        ministr_core::config::MinistrConfig::default_path,
        PathBuf::from,
    );
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

    // `MINISTR_CORPUS_PATHS` overrides every other source. Used by the
    // cloud deployment so the ACA container can be steered to index a
    // specific path (typically `/data/corpus`) without having to plant a
    // `.ministr.toml` on the Azure Files mount.
    let env_paths: Vec<String> = std::env::var("MINISTR_CORPUS_PATHS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            s.split(':')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let corpus_paths: Vec<String> = if !env_paths.is_empty() {
        tracing::info!(
            paths = env_paths.len(),
            "loaded corpus paths from MINISTR_CORPUS_PATHS env var"
        );
        env_paths
    } else if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else if cli_corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli_corpus.to_vec()
    };

    let linked = corpus_config
        .as_ref()
        .map(|(_, cc)| cc.resolve_linked_projects())
        .unwrap_or_default();

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
        linked,
        git_includes,
        resolved_model,
        repo_config_dir,
        resolved_dimension,
        rerank_depth,
    })
}
