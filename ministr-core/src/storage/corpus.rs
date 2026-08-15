//! Corpus on-disk layout management.
//!
//! Each corpus lives at `~/.ministr/corpora/<name>/` with the following structure:
//!
//! ```text
//! ~/.ministr/corpora/<name>/
//! ├── meta.toml       # Corpus configuration
//! ├── content.db      # SQLite database
//! └── sessions/       # Persisted session shadows
//! ```

use std::path::{Path, PathBuf};

use tracing::instrument;

use crate::config::CorpusConfig;
use crate::error::StorageError;

/// Creates the on-disk directory layout for a corpus and writes its `meta.toml`.
///
/// If the directory already exists, this is a no-op for the directories but
/// will overwrite `meta.toml` with the provided configuration.
///
/// # Errors
///
/// Returns [`StorageError::Io`] if directory creation or file writing fails.
#[instrument(skip(config))]
pub fn ensure_corpus_layout(
    data_dir: &Path,
    config: &CorpusConfig,
) -> Result<PathBuf, StorageError> {
    let corpus_dir = data_dir.join("corpora").join(&config.name);
    std::fs::create_dir_all(&corpus_dir)?;
    std::fs::create_dir_all(corpus_dir.join("sessions"))?;

    let meta_path = corpus_dir.join("meta.toml");
    let toml_str = toml::to_string_pretty(config).map_err(|e| StorageError::Serialization {
        reason: e.to_string(),
    })?;
    std::fs::write(&meta_path, toml_str)?;

    tracing::info!(corpus = %config.name, path = %corpus_dir.display(), "corpus layout ready");
    Ok(corpus_dir)
}

/// Write a corpus's `meta.toml` identity sidecar only when none exists.
///
/// The daemon calls this for every registered corpus dir so the directory
/// itself records which source paths it indexes — if the registration is
/// ever lost (an old bug class that orphaned 26 GB of index data), the
/// orphan surface can offer adoption instead of only deletion. Never
/// overwrites: a richer CLI-written `meta.toml` wins.
///
/// # Errors
///
/// Returns [`StorageError::Io`] on write failure or
/// [`StorageError::Serialization`] if the config can't be serialized.
pub fn ensure_corpus_sidecar(corpus_dir: &Path, config: &CorpusConfig) -> Result<(), StorageError> {
    let meta_path = corpus_dir.join("meta.toml");
    if meta_path.exists() {
        return Ok(());
    }
    let toml_str = toml::to_string_pretty(config).map_err(|e| StorageError::Serialization {
        reason: e.to_string(),
    })?;
    std::fs::write(&meta_path, toml_str)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_corpus_layout_creates_dirs_and_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let config = CorpusConfig {
            name: "test-corpus".into(),
            ..CorpusConfig::default()
        };

        let corpus_dir = ensure_corpus_layout(tmp.path(), &config).unwrap();

        assert!(corpus_dir.join("meta.toml").exists());
        assert!(corpus_dir.join("sessions").is_dir());

        let meta_contents = std::fs::read_to_string(corpus_dir.join("meta.toml")).unwrap();
        let loaded: CorpusConfig = toml::from_str(&meta_contents).unwrap();
        assert_eq!(loaded.name, "test-corpus");
    }

    #[test]
    fn ensure_corpus_layout_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let config = CorpusConfig {
            name: "idempotent".into(),
            ..CorpusConfig::default()
        };

        let dir1 = ensure_corpus_layout(tmp.path(), &config).unwrap();
        let dir2 = ensure_corpus_layout(tmp.path(), &config).unwrap();
        assert_eq!(dir1, dir2);
        assert!(dir2.join("meta.toml").exists());
    }
}
