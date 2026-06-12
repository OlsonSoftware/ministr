//! Shared sparse-component construction for both surfaces (CLI + daemon).
//!
//! One function builds the (encoder, inverted index) pair so the two
//! surfaces cannot drift, and an `sparse_encoder.tag` marker next to the
//! sidecar guards against mixing encoders: document vectors written by one
//! encoder scored against query vectors from another are meaningless, so an
//! encoder switch discards the sidecar (it repopulates on the next ingest).

use std::path::Path;
use std::sync::Arc;

use tracing::{info, warn};

use super::SparseEmbedder;
use super::ast_sparse::AstSparseEncoder;
use super::sparse::{DEFAULT_SPARSE_MODEL, FastSparseEmbedder};
use crate::error::IndexError;
use crate::index::{InvertedIndex, SparseIndex};

/// Marker file recording which encoder wrote the sparse sidecar.
const ENCODER_TAG_FILE: &str = "sparse_encoder.tag";

/// Which sparse encoder backs hybrid retrieval for a corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SparseEncoderKind {
    /// Zero-model BM25F over AST-derived roles ([`AstSparseEncoder`]).
    /// The default: beats the neural sparse bars on the code evals at
    /// ~3,000× lower encode cost with no model download, fully
    /// deterministic. Caveat: on human-authored queries the neural encoder
    /// retains a small (−.019 nDCG) semantic-expansion edge.
    #[default]
    Ast,
    /// Neural sparse (SPLADE via fastembed; downloads `splade-pp-v1`).
    Splade,
}

impl SparseEncoderKind {
    /// Parse the `[corpus] sparse_encoder` config value. Unknown values fall
    /// back to the default (visible in logs, not an error — config files
    /// outlive binaries).
    #[must_use]
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("splade") => Self::Splade,
            Some("ast") | None => Self::Ast,
            Some(other) => {
                warn!(value = other, "unknown sparse_encoder — using \"ast\"");
                Self::Ast
            }
        }
    }

    /// Stable tag written to the marker file.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Splade => "splade",
        }
    }
}

/// Build the sparse encoder + inverted index for a corpus.
///
/// Loads the persisted sidecar from `index_dir` — unless the
/// `sparse_encoder.tag` marker shows it was written by a *different*
/// encoder, in which case the sidecar is discarded (with a warning) and an
/// empty index is returned for the next ingest to repopulate. The marker is
/// (re)written to reflect `kind`.
///
/// `models_dir` is only used by [`SparseEncoderKind::Splade`] (model cache
/// location); the AST encoder needs no model.
///
/// # Errors
///
/// Returns [`IndexError`] if the sidecar fails to load or the SPLADE model
/// fails to initialize.
pub fn build_sparse_components(
    kind: SparseEncoderKind,
    models_dir: Option<&Path>,
    index_dir: &Path,
) -> Result<(Arc<dyn SparseEmbedder>, Arc<InvertedIndex>), IndexError> {
    let tag_path = index_dir.join(ENCODER_TAG_FILE);
    let stored_tag = std::fs::read_to_string(&tag_path).ok();
    let stored_tag = stored_tag.as_deref().map(str::trim);

    if let Some(stored) = stored_tag
        && stored != kind.tag()
    {
        warn!(
            stored,
            configured = kind.tag(),
            "sparse encoder changed — discarding the sparse sidecar; re-index to repopulate hybrid retrieval"
        );
        let sidecar = index_dir.join("sparse_index.json");
        if sidecar.exists() {
            std::fs::remove_file(&sidecar).map_err(|e| IndexError::LoadFailed {
                path: sidecar,
                reason: format!("failed to discard stale sparse sidecar: {e}"),
            })?;
        }
    }

    let inverted = Arc::new(InvertedIndex::load_sparse(index_dir)?);

    // Record which encoder owns the sidecar from now on. Best-effort until
    // the index dir exists (fresh corpus: persist_sparse creates it later;
    // the tag is rewritten on every construction so it converges).
    if std::fs::create_dir_all(index_dir).is_ok()
        && let Err(e) = std::fs::write(&tag_path, kind.tag())
    {
        warn!(error = %e, "failed to write sparse encoder tag");
    }

    let embedder: Arc<dyn SparseEmbedder> = match kind {
        SparseEncoderKind::Ast => {
            info!(encoder = "ast", "sparse encoder: zero-model AST/BM25F");
            Arc::new(AstSparseEncoder::new(Arc::clone(&inverted)))
        }
        SparseEncoderKind::Splade => {
            info!(
                encoder = "splade",
                model = DEFAULT_SPARSE_MODEL,
                "sparse encoder: SPLADE"
            );
            let models_dir_str = models_dir.map(|p| p.to_string_lossy().into_owned());
            Arc::new(FastSparseEmbedder::new(
                DEFAULT_SPARSE_MODEL,
                models_dir_str.as_deref(),
            )?)
        }
    };

    Ok((embedder, inverted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_ast() {
        assert_eq!(SparseEncoderKind::parse(None), SparseEncoderKind::Ast);
        assert_eq!(
            SparseEncoderKind::parse(Some("ast")),
            SparseEncoderKind::Ast
        );
        assert_eq!(
            SparseEncoderKind::parse(Some("splade")),
            SparseEncoderKind::Splade
        );
        assert_eq!(
            SparseEncoderKind::parse(Some("nonsense")),
            SparseEncoderKind::Ast
        );
    }

    #[test]
    fn encoder_switch_discards_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        // Populate + persist a sidecar under the AST tag.
        let (_, inverted) =
            build_sparse_components(SparseEncoderKind::Ast, None, dir.path()).unwrap();
        inverted.insert_sparse("doc1", &[1], &[1.0]).unwrap();
        inverted.persist_sparse(dir.path()).unwrap();

        // Same encoder: sidecar survives a reload.
        let (_, reloaded) =
            build_sparse_components(SparseEncoderKind::Ast, None, dir.path()).unwrap();
        assert_eq!(reloaded.len_sparse(), 1, "same-encoder reload keeps docs");

        // Simulate a configured switch by rewriting the tag by hand (building
        // the Splade variant would download a model; the discard logic only
        // reads the tag, so this exercises the same path).
        std::fs::write(dir.path().join("sparse_encoder.tag"), "splade").unwrap();
        let (_, discarded) =
            build_sparse_components(SparseEncoderKind::Ast, None, dir.path()).unwrap();
        assert_eq!(
            discarded.len_sparse(),
            0,
            "encoder mismatch discards the sidecar"
        );
        // And the tag now reflects the configured encoder.
        let tag = std::fs::read_to_string(dir.path().join("sparse_encoder.tag")).unwrap();
        assert_eq!(tag.trim(), "ast");
    }
}
