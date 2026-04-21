//! Configuration schema and loader for ministr.
//!
//! Global configuration lives at `~/.ministr/config.toml`. The loader reads this
//! file and falls back to sensible defaults when the file or individual fields
//! are missing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::StorageError;
use crate::parser::ParserKind;

/// Top-level ministr configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MinistrConfig {
    /// Root data directory (default: `~/.ministr`).
    pub data_dir: PathBuf,

    /// Default embedding model name for new corpora.
    pub default_model: String,

    /// Log output format: `"pretty"` or `"json"`.
    pub log_format: String,

    /// Default context budget in tokens for new sessions.
    pub default_context_budget: usize,

    /// Corpus paths to index — local paths, `https://` URLs, or `github://` URLs.
    ///
    /// When empty, falls back to the CLI `--corpus` flag. Accepts a mix of
    /// directory paths, individual file paths, glob patterns (e.g. `"*.md"`),
    /// `https://` URLs (routed to `WebFetcher`), and `github://owner/repo`
    /// or bare git URLs (routed to `GitFetcher`).
    pub corpus_paths: Vec<String>,

    /// Prefetch configuration.
    pub prefetch: PrefetchConfig,
}

impl Default for MinistrConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            default_model: String::from("all-MiniLM-L6-v2"),
            log_format: String::from("pretty"),
            default_context_budget: 100_000,
            corpus_paths: Vec::new(),
            prefetch: PrefetchConfig::default(),
        }
    }
}

impl MinistrConfig {
    /// Returns the default config file path: `~/.ministr/config.toml`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        default_data_dir().join("config.toml")
    }

    /// Load configuration from a TOML file.
    ///
    /// Returns `Ok(MinistrConfig::default())` if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] if the file exists but cannot be read,
    /// or [`StorageError::Serialization`] if the TOML is malformed.
    #[must_use = "returns the loaded config"]
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path).map_err(StorageError::from)?;
        Self::from_toml(&contents)
    }

    /// Parse configuration from a TOML string.
    ///
    /// Missing fields fall back to their default values, so partial TOML
    /// is accepted.
    ///
    /// # Examples
    ///
    /// ```
    /// use ministr_core::config::MinistrConfig;
    ///
    /// let config = MinistrConfig::from_toml(r#"
    ///     default_model = "bge-small-en-v1.5"
    ///     default_context_budget = 50000
    /// "#).unwrap();
    ///
    /// assert_eq!(config.default_model, "bge-small-en-v1.5");
    /// assert_eq!(config.default_context_budget, 50_000);
    /// // Unset fields use defaults
    /// assert_eq!(config.log_format, "pretty");
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Serialization`] if the TOML cannot be parsed.
    #[must_use = "returns the parsed config"]
    pub fn from_toml(s: &str) -> Result<Self, StorageError> {
        toml::from_str(s).map_err(|e| StorageError::Serialization {
            reason: e.to_string(),
        })
    }
}

/// Prefetch engine configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrefetchConfig {
    /// Whether speculative prefetching is enabled.
    pub enabled: bool,

    /// Maximum number of items in the prefetch cache.
    pub cache_size: usize,

    /// Number of recent sections to use for topical prefetch vector.
    pub topic_window: usize,

    /// Initial EMA alpha for the topic tracker (0.0-1.0).
    /// Higher means more weight on recent sections.
    pub alpha: Option<f32>,

    /// Whether to auto-tune alpha based on topical hit rate.
    /// When enabled, alpha decreases when the topic is stable (high hits)
    /// and increases when the agent jumps between topics (low hits).
    pub adaptive_alpha: Option<bool>,

    /// Whether to enable speculative prefetch-ahead scheduling.
    /// When enabled, prefetch runs proactively during agent processing time.
    pub speculative: Option<bool>,

    /// Maximum candidates per strategy in a single prefetch cycle.
    pub max_candidates_per_strategy: Option<usize>,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_size: 128,
            topic_window: 5,
            alpha: None,
            adaptive_alpha: None,
            speculative: None,
            max_candidates_per_strategy: None,
        }
    }
}

/// Per-corpus configuration (stored in `~/.ministr/corpora/<name>/meta.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorpusConfig {
    /// Human-readable corpus name.
    pub name: String,

    /// Source directories to index.
    pub source_dirs: Vec<PathBuf>,

    /// Embedding model override (falls back to global default).
    pub model: Option<String>,

    /// Whether to watch source directories for changes.
    pub watch: bool,

    /// Claim extraction mode.
    pub claim_extraction: ClaimExtractionMode,

    /// Override the parser for all files in this corpus.
    /// When `None`, the parser is auto-detected from the file extension.
    pub parser: Option<ParserKind>,

    /// Minimum token count for a section to remain standalone.
    ///
    /// Sections below this threshold are candidates for merging with adjacent
    /// siblings of the same depth. Set to `0` to disable merging.
    pub min_section_tokens: usize,
}

impl CorpusConfig {
    /// Load corpus configuration from a `meta.toml` file.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] if the file cannot be read, or
    /// [`StorageError::Serialization`] if the TOML is malformed.
    #[must_use = "returns the loaded config"]
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(|e| StorageError::Serialization {
            reason: e.to_string(),
        })
    }

    /// Save corpus configuration to a `meta.toml` file.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] if the file cannot be written, or
    /// [`StorageError::Serialization`] if serialization fails.
    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        let toml_str = toml::to_string_pretty(self).map_err(|e| StorageError::Serialization {
            reason: e.to_string(),
        })?;
        std::fs::write(path, toml_str)?;
        Ok(())
    }
}

impl Default for CorpusConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            source_dirs: Vec::new(),
            model: None,
            watch: true,
            claim_extraction: ClaimExtractionMode::Heuristic,
            parser: None,
            min_section_tokens: 50,
        }
    }
}

/// How claims are extracted from sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimExtractionMode {
    /// Fast sentence-boundary splitting with heuristic filtering.
    Heuristic,
    /// Use a small local model for higher-quality extraction.
    ModelAssisted,
}

/// A classified corpus source after parsing a raw corpus path string.
///
/// Used to dispatch ingestion to the appropriate fetcher at startup.
///
/// # Examples
///
/// ```
/// use ministr_core::config::classify_corpus_path;
/// use ministr_core::config::CorpusSource;
///
/// assert!(matches!(classify_corpus_path("./docs"), CorpusSource::Local(_)));
/// assert!(matches!(classify_corpus_path("https://example.com/docs"), CorpusSource::Web(_)));
/// assert!(matches!(classify_corpus_path("github://owner/repo"), CorpusSource::Git(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusSource {
    /// A local filesystem path (directory, file, or glob pattern).
    Local(PathBuf),
    /// An `https://` URL to fetch via `WebFetcher`.
    Web(String),
    /// A git repository URL to clone via `GitFetcher`.
    Git(String),
}

/// Classify a raw corpus path string into a [`CorpusSource`].
///
/// Recognition rules:
/// - `https://` or `http://` → [`CorpusSource::Web`]
/// - `github://owner/repo` → [`CorpusSource::Git`] (normalized to `https://github.com/owner/repo.git`)
/// - URLs ending in `.git` → [`CorpusSource::Git`]
/// - `git@` SSH URLs → [`CorpusSource::Git`]
/// - Everything else → [`CorpusSource::Local`]
///
/// # Examples
///
/// ```
/// use ministr_core::config::{classify_corpus_path, CorpusSource};
///
/// // HTTPS web URL
/// let src = classify_corpus_path("https://docs.rs/tokio/latest/tokio/");
/// assert!(matches!(src, CorpusSource::Web(_)));
///
/// // GitHub shorthand
/// let src = classify_corpus_path("github://tokio-rs/tokio");
/// assert_eq!(src, CorpusSource::Git("https://github.com/tokio-rs/tokio.git".into()));
///
/// // Bare git URL
/// let src = classify_corpus_path("https://github.com/user/repo.git");
/// assert!(matches!(src, CorpusSource::Git(_)));
///
/// // SSH git URL
/// let src = classify_corpus_path("git@github.com:user/repo.git");
/// assert!(matches!(src, CorpusSource::Git(_)));
///
/// // Local path
/// let src = classify_corpus_path("/home/user/docs");
/// assert!(matches!(src, CorpusSource::Local(_)));
/// ```
#[must_use]
pub fn classify_corpus_path(raw: &str) -> CorpusSource {
    // github:// shorthand → normalize to HTTPS .git URL
    if let Some(rest) = raw.strip_prefix("github://") {
        let rest = rest.trim_end_matches('/');
        return CorpusSource::Git(format!("https://github.com/{rest}.git"));
    }

    // SSH git URLs (git@host:owner/repo.git)
    if raw.starts_with("git@") {
        return CorpusSource::Git(raw.to_owned());
    }

    // HTTPS/HTTP URLs
    if raw.starts_with("https://") || raw.starts_with("http://") {
        // URLs ending in .git are git repos
        if raw.to_ascii_lowercase().ends_with(".git") {
            return CorpusSource::Git(raw.to_owned());
        }
        return CorpusSource::Web(raw.to_owned());
    }

    // Everything else is a local path
    CorpusSource::Local(PathBuf::from(raw))
}

/// Returns the default ministr data directory (`~/.ministr`).
fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ministr")
}

// ---------------------------------------------------------------------------
// Per-repo `.ministr.toml` configuration
// ---------------------------------------------------------------------------

/// Per-repo corpus configuration, loaded from `.ministr.toml`.
///
/// Declares what to index from the repo: local paths, ignore patterns,
/// external directories, and git repositories.
///
/// # Examples
///
/// ```
/// use ministr_core::config::RepoConfig;
///
/// let toml = r#"
/// [corpus]
/// paths = ["src", "docs"]
/// ignore = ["*.test.ts"]
///
/// [[corpus.include]]
/// path = "~/Code/shared-types/src"
///
/// [[corpus.git]]
/// repo = "https://github.com/owner/repo.git"
/// paths = ["src"]
/// "#;
///
/// let config: RepoConfig = toml::from_str(toml).unwrap();
/// assert_eq!(config.corpus.paths, vec!["src", "docs"]);
/// assert_eq!(config.corpus.git.len(), 1);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoConfig {
    /// What to index.
    pub corpus: CorpusSpec,
    /// Agent configuration: custom rules and preferences.
    #[serde(default)]
    pub agent: AgentConfig,
}

/// Custom agent configuration in `.ministr.toml`.
///
/// Rules listed here are appended to all generated advisory files
/// (`.claude/rules/`, `.cursor/rules/`, `.github/copilot-instructions.md`, etc.).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Custom rules appended to generated agent instruction files.
    pub rules: Vec<String>,
}

/// Specification of what to index from a repo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorpusSpec {
    /// Paths relative to the repo root to index (defaults to `["."]`).
    pub paths: Vec<String>,
    /// Additional ignore patterns (on top of built-in always-ignore lists).
    pub ignore: Vec<String>,
    /// External local directories to include.
    pub include: Vec<ExternalInclude>,
    /// Git repositories to clone and index.
    pub git: Vec<GitInclude>,
    /// Pre-built cloud index bundles to fetch instead of re-indexing.
    pub cloud: Vec<CloudInclude>,
    /// Embedding model override for this repo.
    ///
    /// When set, overrides the global `default_model`. Use
    /// [`supported_models()`](crate::embedding::supported_models) for valid names.
    pub model: Option<String>,
    /// Target embedding dimension for Matryoshka truncation.
    ///
    /// When set, embeddings are truncated to this dimensionality and
    /// re-normalized. Only useful with Matryoshka-capable models
    /// (e.g. `nomic-embed-text-v1.5`).
    pub dimension: Option<usize>,
    /// Number of coarse HNSW candidates to retrieve for full-dimension
    /// rescoring during two-stage Matryoshka retrieval.
    ///
    /// Only effective when `dimension` is set (Matryoshka truncation active).
    /// Defaults to 100. Set to 0 to disable two-stage reranking while still
    /// using truncated embeddings.
    pub rerank_depth: Option<usize>,
}

/// An external local directory to include in the corpus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalInclude {
    /// Absolute or `~`-relative path to the directory.
    pub path: String,
}

/// A git repository to clone and index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitInclude {
    /// Remote git repository URL (HTTPS or SSH).
    pub repo: String,
    /// Optional paths for sparse checkout.
    pub paths: Option<Vec<String>>,
    /// Optional branch (defaults to the repo's default branch).
    pub branch: Option<String>,
}

/// A pre-built index bundle to fetch from a remote URL.
///
/// When specified, ministr downloads the `.ministr-index` bundle and imports it
/// instead of cloning and re-indexing locally. This is useful for large
/// codebases where indexing is expensive but a maintainer publishes
/// pre-built bundles.
///
/// # Example
///
/// ```toml
/// [[corpus.cloud]]
/// url = "https://releases.example.com/my-project.ministr-index"
/// name = "my-project"
/// pin_version = "abc123def456"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudInclude {
    /// URL to the `.ministr-index` bundle file.
    pub url: String,
    /// Optional name for the imported corpus (defaults to URL filename stem).
    pub name: Option<String>,
    /// Pin to a specific bundle version hash.
    ///
    /// When set, the client skips staleness checks and only re-fetches if the
    /// local cached version differs from this value.
    #[serde(default)]
    pub pin_version: Option<String>,
}

/// Resolve the effective embedding model name.
///
/// Priority (highest to lowest):
/// 1. Per-repo `.ministr.toml` `corpus.model`
/// 2. Per-corpus `meta.toml` `model`
/// 3. Global `~/.ministr/config.toml` `default_model`
///
/// # Examples
///
/// ```
/// use ministr_core::config::{resolve_model_name, MinistrConfig, RepoConfig, CorpusConfig, CorpusSpec};
///
/// let global = MinistrConfig::default();
///
/// // No overrides — uses global default
/// assert_eq!(resolve_model_name(None, None, &global), "all-MiniLM-L6-v2");
///
/// // Repo config overrides global
/// let mut repo = RepoConfig::default();
/// repo.corpus.model = Some("jina-embeddings-v2-base-code".into());
/// assert_eq!(
///     resolve_model_name(Some(&repo), None, &global),
///     "jina-embeddings-v2-base-code"
/// );
/// ```
#[must_use]
pub fn resolve_model_name(
    repo_config: Option<&RepoConfig>,
    corpus_config: Option<&CorpusConfig>,
    global_config: &MinistrConfig,
) -> String {
    // 1. Per-repo .ministr.toml model
    if let Some(repo) = repo_config
        && let Some(ref model) = repo.corpus.model
    {
        return model.clone();
    }
    // 2. Per-corpus meta.toml model
    if let Some(corpus) = corpus_config
        && let Some(ref model) = corpus.model
    {
        return model.clone();
    }
    // 3. Global default
    global_config.default_model.clone()
}

/// The name of the per-repo config file.
pub const CORPUS_CONFIG_FILENAME: &str = ".ministr.toml";

impl RepoConfig {
    /// Discover `.ministr.toml` by walking up from `start_dir`.
    ///
    /// Returns `Some((config_dir, config))` if found, `None` otherwise.
    /// The `config_dir` is the directory containing `.ministr.toml`, used
    /// to resolve relative paths.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the file exists but cannot be parsed.
    #[must_use = "returns the discovered config"]
    pub fn discover(start_dir: &Path) -> Result<Option<(PathBuf, Self)>, StorageError> {
        let mut dir = start_dir.to_path_buf();
        loop {
            let config_path = dir.join(CORPUS_CONFIG_FILENAME);
            if config_path.exists() {
                let contents = std::fs::read_to_string(&config_path).map_err(StorageError::from)?;
                let config: Self =
                    toml::from_str(&contents).map_err(|e| StorageError::Serialization {
                        reason: format!(
                            "invalid TOML in {path}: {e}\n\n  \
                             hint: check for missing quotes, mismatched brackets, \
                             or invalid field names",
                            path = config_path.display()
                        ),
                    })?;
                return Ok(Some((dir, config)));
            }
            if !dir.pop() {
                break;
            }
        }
        Ok(None)
    }

    /// Resolve all corpus paths from this config relative to `base_dir`.
    ///
    /// Returns a flat list of corpus path strings suitable for passing
    /// to the ingestion pipeline. Local paths are resolved relative to
    /// `base_dir`, `~` in external includes is expanded, and git repos
    /// are returned as separate lists.
    #[must_use]
    pub fn resolve_local_paths(&self, base_dir: &Path) -> Vec<String> {
        let mut paths = Vec::new();

        // Repo-local paths (default to "." if empty)
        let local_paths = if self.corpus.paths.is_empty() {
            vec![".".to_string()]
        } else {
            self.corpus.paths.clone()
        };

        for p in &local_paths {
            let resolved = base_dir.join(p);
            paths.push(resolved.to_string_lossy().to_string());
        }

        // External includes (expand ~)
        for inc in &self.corpus.include {
            let expanded = expand_tilde(&inc.path);
            paths.push(expanded);
        }

        paths
    }
}

/// A warning about a potential issue in a `.ministr.toml` config file.
///
/// Validation produces warnings rather than hard errors because some
/// issues (like not-yet-created directories) may be intentional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWarning {
    /// A path listed in `[corpus] paths` does not exist on disk.
    MissingPath {
        /// The path that could not be found.
        path: String,
        /// The absolute resolved path that was checked.
        resolved: PathBuf,
    },
    /// The config has no paths and no git repos — nothing to index.
    EmptyCorpus,
}

impl std::fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPath { path, resolved } => {
                write!(
                    f,
                    "corpus path \"{path}\" does not exist (resolved to {})",
                    resolved.display()
                )
            }
            Self::EmptyCorpus => write!(
                f,
                ".ministr.toml has no paths and no git repos — nothing to index"
            ),
        }
    }
}

impl RepoConfig {
    /// Validate the config against the filesystem and return any warnings.
    ///
    /// Checks that corpus paths exist, that at least one source is
    /// configured, and that external includes resolve to valid directories.
    #[must_use]
    pub fn validate(&self, base_dir: &Path) -> Vec<ConfigWarning> {
        let mut warnings = Vec::new();

        // Check if completely empty
        if self.corpus.paths.is_empty()
            && self.corpus.git.is_empty()
            && self.corpus.include.is_empty()
        {
            warnings.push(ConfigWarning::EmptyCorpus);
            return warnings;
        }

        // Check local paths exist
        for p in &self.corpus.paths {
            let resolved = base_dir.join(p);
            if !resolved.exists() {
                warnings.push(ConfigWarning::MissingPath {
                    path: p.clone(),
                    resolved,
                });
            }
        }

        // Check external includes exist
        for inc in &self.corpus.include {
            let expanded = expand_tilde(&inc.path);
            let resolved = PathBuf::from(&expanded);
            if !resolved.exists() {
                warnings.push(ConfigWarning::MissingPath {
                    path: inc.path.clone(),
                    resolved,
                });
            }
        }

        warnings
    }
}

/// Expand `~` at the start of a path to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().to_string();
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = MinistrConfig::default();
        assert_eq!(config.default_model, "all-MiniLM-L6-v2");
        assert_eq!(config.log_format, "pretty");
        assert_eq!(config.default_context_budget, 100_000);
        assert!(config.prefetch.enabled);
        assert_eq!(config.prefetch.cache_size, 128);
    }

    #[test]
    fn parse_partial_toml_uses_defaults() {
        let toml = r#"
            default_model = "bge-small-en-v1.5"
            default_context_budget = 50000
        "#;
        let config = MinistrConfig::from_toml(toml).unwrap();
        assert_eq!(config.default_model, "bge-small-en-v1.5");
        assert_eq!(config.default_context_budget, 50_000);
        // Unset fields use defaults
        assert_eq!(config.log_format, "pretty");
        assert!(config.prefetch.enabled);
    }

    #[test]
    fn parse_full_toml() {
        let toml = r#"
            data_dir = "/tmp/ministr-test"
            default_model = "nomic-embed-text-v1.5"
            log_format = "json"
            default_context_budget = 200000

            [prefetch]
            enabled = false
            cache_size = 100
            topic_window = 10
        "#;
        let config = MinistrConfig::from_toml(toml).unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/tmp/ministr-test"));
        assert_eq!(config.default_model, "nomic-embed-text-v1.5");
        assert_eq!(config.log_format, "json");
        assert!(!config.prefetch.enabled);
        assert_eq!(config.prefetch.cache_size, 100);
    }

    #[test]
    fn parse_empty_toml_returns_defaults() {
        let config = MinistrConfig::from_toml("").unwrap();
        assert_eq!(config, MinistrConfig::default());
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let result = MinistrConfig::from_toml("this is [[[not valid");
        assert!(result.is_err());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let config = MinistrConfig::load(Path::new("/nonexistent/config.toml")).unwrap();
        assert_eq!(config, MinistrConfig::default());
    }

    #[test]
    fn default_path_ends_with_config_toml() {
        let path = MinistrConfig::default_path();
        assert!(path.ends_with("config.toml"));
        assert!(path.to_string_lossy().contains(".ministr"));
    }

    #[test]
    fn corpus_config_defaults() {
        let config = CorpusConfig::default();
        assert!(config.name.is_empty());
        assert!(config.source_dirs.is_empty());
        assert!(config.watch);
        assert_eq!(config.claim_extraction, ClaimExtractionMode::Heuristic);
    }

    #[test]
    fn corpus_config_from_toml() {
        let toml = r#"
            name = "my-docs"
            source_dirs = ["/home/user/docs"]
            watch = false
            claim_extraction = "model_assisted"
        "#;
        let config: CorpusConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.name, "my-docs");
        assert!(!config.watch);
        assert_eq!(config.claim_extraction, ClaimExtractionMode::ModelAssisted);
    }

    #[test]
    fn corpus_config_save_and_load_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = CorpusConfig {
            name: "roundtrip".into(),
            source_dirs: vec![PathBuf::from("/docs")],
            model: Some("bge-small".into()),
            watch: false,
            claim_extraction: ClaimExtractionMode::ModelAssisted,
            parser: None,
            min_section_tokens: 100,
        };
        config.save(tmp.path()).unwrap();
        let loaded = CorpusConfig::load(tmp.path()).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn corpus_config_load_missing_file() {
        let result = CorpusConfig::load(Path::new("/nonexistent/meta.toml"));
        assert!(result.is_err());
    }

    // -- classify_corpus_path tests --

    #[test]
    fn classify_local_absolute_path() {
        let src = classify_corpus_path("/home/user/docs");
        assert_eq!(src, CorpusSource::Local(PathBuf::from("/home/user/docs")));
    }

    #[test]
    fn classify_local_relative_path() {
        let src = classify_corpus_path("./docs");
        assert_eq!(src, CorpusSource::Local(PathBuf::from("./docs")));
    }

    #[test]
    fn classify_local_glob_pattern() {
        let src = classify_corpus_path("*.md");
        assert_eq!(src, CorpusSource::Local(PathBuf::from("*.md")));
    }

    #[test]
    fn classify_https_web_url() {
        let src = classify_corpus_path("https://docs.rs/tokio/latest/tokio/");
        assert_eq!(
            src,
            CorpusSource::Web("https://docs.rs/tokio/latest/tokio/".into())
        );
    }

    #[test]
    fn classify_http_web_url() {
        let src = classify_corpus_path("http://example.com/api");
        assert_eq!(src, CorpusSource::Web("http://example.com/api".into()));
    }

    #[test]
    fn classify_https_git_url() {
        let src = classify_corpus_path("https://github.com/user/repo.git");
        assert_eq!(
            src,
            CorpusSource::Git("https://github.com/user/repo.git".into())
        );
    }

    #[test]
    fn classify_ssh_git_url() {
        let src = classify_corpus_path("git@github.com:user/repo.git");
        assert_eq!(
            src,
            CorpusSource::Git("git@github.com:user/repo.git".into())
        );
    }

    #[test]
    fn classify_github_shorthand() {
        let src = classify_corpus_path("github://tokio-rs/tokio");
        assert_eq!(
            src,
            CorpusSource::Git("https://github.com/tokio-rs/tokio.git".into())
        );
    }

    #[test]
    fn classify_github_shorthand_trailing_slash() {
        let src = classify_corpus_path("github://owner/repo/");
        assert_eq!(
            src,
            CorpusSource::Git("https://github.com/owner/repo.git".into())
        );
    }

    #[test]
    fn classify_mixed_corpus_paths_from_toml() {
        let toml = r#"
            corpus_paths = [
                "/home/user/docs",
                "https://docs.rs/serde",
                "github://serde-rs/serde",
                "git@github.com:user/repo.git"
            ]
        "#;
        let config = MinistrConfig::from_toml(toml).unwrap();
        assert_eq!(config.corpus_paths.len(), 4);

        let classified: Vec<CorpusSource> = config
            .corpus_paths
            .iter()
            .map(|p| classify_corpus_path(p))
            .collect();
        assert!(matches!(classified[0], CorpusSource::Local(_)));
        assert!(matches!(classified[1], CorpusSource::Web(_)));
        assert!(matches!(classified[2], CorpusSource::Git(_)));
        assert!(matches!(classified[3], CorpusSource::Git(_)));
    }

    #[test]
    fn validate_missing_path_warns() {
        let config = RepoConfig {
            corpus: CorpusSpec {
                paths: vec!["src".to_string(), "nonexistent-dir".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();

        let warnings = config.validate(root);
        assert_eq!(warnings.len(), 1);
        assert!(
            matches!(&warnings[0], ConfigWarning::MissingPath { path, .. } if path == "nonexistent-dir")
        );
    }

    #[test]
    fn validate_empty_corpus_warns() {
        let config = RepoConfig::default();
        let tmp = tempfile::TempDir::new().unwrap();

        let warnings = config.validate(tmp.path());
        assert_eq!(warnings.len(), 1);
        assert!(matches!(&warnings[0], ConfigWarning::EmptyCorpus));
    }

    #[test]
    fn validate_valid_config_no_warnings() {
        let config = RepoConfig {
            corpus: CorpusSpec {
                paths: vec!["src".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();

        let warnings = config.validate(tmp.path());
        assert!(warnings.is_empty());
    }

    #[test]
    fn validate_git_only_no_warning() {
        let config = RepoConfig {
            corpus: CorpusSpec {
                git: vec![GitInclude {
                    repo: "https://github.com/example/repo.git".to_string(),
                    paths: None,
                    branch: None,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let tmp = tempfile::TempDir::new().unwrap();

        let warnings = config.validate(tmp.path());
        assert!(warnings.is_empty());
    }

    // -- resolve_model_name tests --

    #[test]
    fn resolve_model_falls_back_to_global() {
        let global = MinistrConfig::default();
        assert_eq!(resolve_model_name(None, None, &global), "all-MiniLM-L6-v2");
    }

    #[test]
    fn resolve_model_corpus_overrides_global() {
        let global = MinistrConfig::default();
        let corpus = CorpusConfig {
            model: Some("bge-base-en-v1.5".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_model_name(None, Some(&corpus), &global),
            "bge-base-en-v1.5"
        );
    }

    #[test]
    fn resolve_model_repo_overrides_all() {
        let global = MinistrConfig::default();
        let corpus = CorpusConfig {
            model: Some("bge-base-en-v1.5".into()),
            ..Default::default()
        };
        let repo = RepoConfig {
            corpus: CorpusSpec {
                model: Some("jina-embeddings-v2-base-code".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_model_name(Some(&repo), Some(&corpus), &global),
            "jina-embeddings-v2-base-code"
        );
    }

    #[test]
    fn resolve_model_repo_none_falls_through() {
        let global = MinistrConfig::default();
        let corpus = CorpusConfig {
            model: Some("bge-base-en-v1.5".into()),
            ..Default::default()
        };
        let repo = RepoConfig {
            corpus: CorpusSpec {
                model: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            resolve_model_name(Some(&repo), Some(&corpus), &global),
            "bge-base-en-v1.5"
        );
    }

    // -- CorpusSpec model field --

    #[test]
    fn corpus_spec_model_from_toml() {
        let toml = r#"
            [corpus]
            paths = ["src"]
            model = "jina-embeddings-v2-base-code"
        "#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.corpus.model.as_deref(),
            Some("jina-embeddings-v2-base-code")
        );
    }

    #[test]
    fn corpus_spec_model_default_none() {
        let config = CorpusSpec::default();
        assert!(config.model.is_none());
        assert!(config.dimension.is_none());
    }

    #[test]
    fn corpus_spec_dimension_from_toml() {
        let toml = r#"
            [corpus]
            paths = ["src"]
            model = "nomic-embed-text-v1.5"
            dimension = 256
        "#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.corpus.dimension, Some(256));
    }

    #[test]
    fn corpus_spec_cloud_includes_from_toml() {
        let toml = r#"
            [corpus]
            paths = ["src"]

            [[corpus.cloud]]
            url = "https://releases.example.com/my-project.ministr-index"
            name = "my-project"

            [[corpus.cloud]]
            url = "https://cdn.example.com/shared-types.ministr-index"
        "#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.corpus.cloud.len(), 2);
        assert_eq!(
            config.corpus.cloud[0].url,
            "https://releases.example.com/my-project.ministr-index"
        );
        assert_eq!(config.corpus.cloud[0].name, Some("my-project".into()));
        assert_eq!(
            config.corpus.cloud[1].url,
            "https://cdn.example.com/shared-types.ministr-index"
        );
        assert!(config.corpus.cloud[1].name.is_none());
        // pin_version defaults to None when omitted
        assert!(config.corpus.cloud[0].pin_version.is_none());
        assert!(config.corpus.cloud[1].pin_version.is_none());
    }

    #[test]
    fn corpus_spec_cloud_pin_version() {
        let toml = r#"
            [corpus]
            paths = ["src"]

            [[corpus.cloud]]
            url = "https://example.com/pinned.ministr-index"
            pin_version = "abc123"
        "#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.corpus.cloud.len(), 1);
        assert_eq!(config.corpus.cloud[0].pin_version, Some("abc123".into()));
    }

    #[test]
    fn corpus_spec_cloud_defaults_to_empty() {
        let toml = r#"
            [corpus]
            paths = ["src"]
        "#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert!(config.corpus.cloud.is_empty());
    }
}
