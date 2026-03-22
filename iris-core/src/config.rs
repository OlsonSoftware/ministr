//! Configuration schema and loader for iris.
//!
//! Global configuration lives at `~/.iris/config.toml`. The loader reads this
//! file and falls back to sensible defaults when the file or individual fields
//! are missing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::StorageError;
use crate::parser::ParserKind;

/// Top-level iris configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IrisConfig {
    /// Root data directory (default: `~/.iris`).
    pub data_dir: PathBuf,

    /// Default embedding model name for new corpora.
    pub default_model: String,

    /// Log output format: `"pretty"` or `"json"`.
    pub log_format: String,

    /// Default context budget in tokens for new sessions.
    pub default_context_budget: usize,

    /// Corpus paths to index (directories, individual files, or glob patterns).
    ///
    /// When empty, falls back to the CLI `--corpus` flag. Accepts a mix of
    /// directory paths, individual file paths, and glob patterns (e.g. `"*.md"`).
    pub corpus_paths: Vec<PathBuf>,

    /// Prefetch configuration.
    pub prefetch: PrefetchConfig,
}

impl Default for IrisConfig {
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

impl IrisConfig {
    /// Returns the default config file path: `~/.iris/config.toml`.
    #[must_use]
    pub fn default_path() -> PathBuf {
        default_data_dir().join("config.toml")
    }

    /// Load configuration from a TOML file.
    ///
    /// Returns `Ok(IrisConfig::default())` if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] if the file exists but cannot be read,
    /// or [`StorageError::Serialization`] if the TOML is malformed.
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
    /// use iris_core::config::IrisConfig;
    ///
    /// let config = IrisConfig::from_toml(r#"
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
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_size: 50,
            topic_window: 5,
        }
    }
}

/// Per-corpus configuration (stored in `~/.iris/corpora/<name>/meta.toml`).
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

/// Returns the default iris data directory (`~/.iris`).
fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".iris")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = IrisConfig::default();
        assert_eq!(config.default_model, "all-MiniLM-L6-v2");
        assert_eq!(config.log_format, "pretty");
        assert_eq!(config.default_context_budget, 100_000);
        assert!(config.prefetch.enabled);
        assert_eq!(config.prefetch.cache_size, 50);
    }

    #[test]
    fn parse_partial_toml_uses_defaults() {
        let toml = r#"
            default_model = "bge-small-en-v1.5"
            default_context_budget = 50000
        "#;
        let config = IrisConfig::from_toml(toml).unwrap();
        assert_eq!(config.default_model, "bge-small-en-v1.5");
        assert_eq!(config.default_context_budget, 50_000);
        // Unset fields use defaults
        assert_eq!(config.log_format, "pretty");
        assert!(config.prefetch.enabled);
    }

    #[test]
    fn parse_full_toml() {
        let toml = r#"
            data_dir = "/tmp/iris-test"
            default_model = "nomic-embed-text-v1.5"
            log_format = "json"
            default_context_budget = 200000

            [prefetch]
            enabled = false
            cache_size = 100
            topic_window = 10
        "#;
        let config = IrisConfig::from_toml(toml).unwrap();
        assert_eq!(config.data_dir, PathBuf::from("/tmp/iris-test"));
        assert_eq!(config.default_model, "nomic-embed-text-v1.5");
        assert_eq!(config.log_format, "json");
        assert!(!config.prefetch.enabled);
        assert_eq!(config.prefetch.cache_size, 100);
    }

    #[test]
    fn parse_empty_toml_returns_defaults() {
        let config = IrisConfig::from_toml("").unwrap();
        assert_eq!(config, IrisConfig::default());
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let result = IrisConfig::from_toml("this is [[[not valid");
        assert!(result.is_err());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let config = IrisConfig::load(Path::new("/nonexistent/config.toml")).unwrap();
        assert_eq!(config, IrisConfig::default());
    }

    #[test]
    fn default_path_ends_with_config_toml() {
        let path = IrisConfig::default_path();
        assert!(path.ends_with("config.toml"));
        assert!(path.to_string_lossy().contains(".iris"));
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
}
