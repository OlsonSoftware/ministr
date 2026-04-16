//! Embedding: dense, sparse, batch insertion, and vector deletion.

use tracing::{debug, info};

use crate::embedding::{DualEmbedder, Embedder};
use crate::error::IngestionError;
use crate::index::VectorIndex;
use crate::mem_profile;
use crate::storage::{SqliteStorage, Storage};
use crate::types::{DocumentTree, Section, VectorId};

/// Maximum texts per embedding inference call.
pub(super) const EMBED_BATCH_CHUNK: usize = 128;

/// Accumulated pairs that trigger an intermediate flush.
pub(super) const EMBED_FLUSH_THRESHOLD: usize = 4096;

/// Embed a document tree at all three resolution levels (immediate).
pub(super) fn embed_document<E: Embedder + ?Sized, I: VectorIndex + ?Sized>(
    doc: &DocumentTree,
    embedder: &E,
    index: &I,
) -> Result<usize, IngestionError> {
    let mut texts: Vec<String> = Vec::new();
    let mut ids: Vec<VectorId> = Vec::new();

    if let Some(ref summary) = doc.summary {
        if !summary.trim().is_empty() {
            ids.push(VectorId::doc_summary(doc.id.as_ref()));
            texts.push(summary.clone());
        }
    }

    collect_embeddable_items(&doc.sections, &mut ids, &mut texts);

    if texts.is_empty() {
        return Ok(0);
    }

    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let vectors = embedder
        .embed(&text_refs)
        .map_err(|e| IngestionError::Embedding {
            reason: e.to_string(),
        })?;

    for (vid, vector) in ids.iter().zip(vectors.iter()) {
        index
            .insert(vid.as_str(), vector)
            .map_err(|e| IngestionError::Embedding {
                reason: format!("failed to insert vector {vid}: {e}"),
            })?;
    }

    let count = ids.len();
    debug!(embeddings = count, doc_id = %doc.id, "embedded document");
    Ok(count)
}

/// Collect embeddable `(id, text)` pairs from a document tree without embedding.
pub(super) fn collect_document_embeddings(doc: &DocumentTree, pairs: &mut Vec<(VectorId, String)>) {
    if let Some(ref summary) = doc.summary {
        if !summary.trim().is_empty() {
            pairs.push((VectorId::doc_summary(doc.id.as_ref()), summary.clone()));
        }
    }

    let mut ids = Vec::new();
    let mut texts = Vec::new();
    collect_embeddable_items(&doc.sections, &mut ids, &mut texts);
    pairs.extend(ids.into_iter().zip(texts));
}

/// Embed and insert a batch of `(id, text)` pairs into the vector index.
pub(super) async fn batch_embed_and_insert<E: Embedder + ?Sized, I: VectorIndex + ?Sized>(
    pairs: &[(VectorId, String)],
    embedder: &E,
    index: &I,
) -> Result<usize, IngestionError> {
    if pairs.is_empty() {
        return Ok(0);
    }
    let mut total = 0;
    let num_chunks = pairs.len().div_ceil(EMBED_BATCH_CHUNK);

    for (i, chunk) in pairs.chunks(EMBED_BATCH_CHUNK).enumerate() {
        let text_refs: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        mem_profile::checkpoint_every(5, i, "before embedder.embed()");
        let vectors = embedder
            .embed(&text_refs)
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?;
        mem_profile::checkpoint_every(5, i, "after embedder.embed()");

        for ((vid, _), vector) in chunk.iter().zip(vectors.iter()) {
            index
                .insert(vid.as_str(), vector)
                .map_err(|e| IngestionError::Embedding {
                    reason: format!("failed to insert vector {vid}: {e}"),
                })?;
        }
        total += chunk.len();
        mem_profile::checkpoint_every(5, i, "after index.insert() batch");

        if num_chunks > 1 {
            info!(
                chunk = i + 1,
                of = num_chunks,
                embedded = total,
                "embedding progress"
            );
            tokio::task::yield_now().await;
        }
    }

    info!(embeddings = total, "batch embedding complete");
    Ok(total)
}

/// Embed and insert a batch using a [`DualEmbedder`], storing both truncated
/// vectors in the HNSW index and full-dimension vectors in SQLite.
///
/// Falls back to the single-embed path when `dual_embedder` is `None`.
pub(super) async fn batch_embed_and_insert_dual<I: VectorIndex + ?Sized>(
    pairs: &[(VectorId, String)],
    dual_embedder: &dyn DualEmbedder,
    index: &I,
    storage: &SqliteStorage,
) -> Result<usize, IngestionError> {
    if pairs.is_empty() {
        return Ok(0);
    }
    let mut total = 0;
    let num_chunks = pairs.len().div_ceil(EMBED_BATCH_CHUNK);

    for (i, chunk) in pairs.chunks(EMBED_BATCH_CHUNK).enumerate() {
        let text_refs: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        mem_profile::checkpoint_every(5, i, "before dual_embedder.embed_dual()");
        let dual = dual_embedder
            .embed_dual(&text_refs)
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?;
        mem_profile::checkpoint_every(5, i, "after dual_embedder.embed_dual()");

        // Insert truncated vectors into HNSW index.
        for ((vid, _), vector) in chunk.iter().zip(dual.truncated.iter()) {
            index
                .insert(vid.as_str(), vector)
                .map_err(|e| IngestionError::Embedding {
                    reason: format!("failed to insert vector {vid}: {e}"),
                })?;
        }

        // Store full-dim vectors in SQLite.
        let full_entries: Vec<(String, Vec<f32>)> = chunk
            .iter()
            .zip(dual.full.iter())
            .map(|((vid, _), vec)| (vid.to_string(), vec.clone()))
            .collect();
        storage
            .store_full_dim_vectors(&full_entries)
            .await
            .map_err(|e| IngestionError::Embedding {
                reason: format!("failed to store full-dim vectors: {e}"),
            })?;

        total += chunk.len();
        mem_profile::checkpoint_every(5, i, "after dual index+storage batch");

        if num_chunks > 1 {
            info!(
                chunk = i + 1,
                of = num_chunks,
                embedded = total,
                "dual embedding progress"
            );
            tokio::task::yield_now().await;
        }
    }

    info!(embeddings = total, "dual batch embedding complete");
    Ok(total)
}

/// Recursively collect embeddable items (section summaries, section texts, claims).
fn collect_embeddable_items(
    sections: &[Section],
    ids: &mut Vec<VectorId>,
    texts: &mut Vec<String>,
) {
    for section in sections {
        if let Some(ref summary) = section.summary {
            if !summary.trim().is_empty() {
                ids.push(VectorId::sec_summary(section.id.as_ref()));
                texts.push(summary.clone());
            }
        }

        if !section.text.trim().is_empty() {
            ids.push(VectorId::section(section.id.as_ref()));
            texts.push(section.text.clone());
        }

        for claim in &section.claims {
            if !claim.text.trim().is_empty() {
                ids.push(VectorId::claim(claim.id.as_ref()));
                texts.push(claim.text.clone());
            }
        }

        collect_embeddable_items(&section.children, ids, texts);
    }
}

/// Delete all vectors associated with a document from the index.
pub(super) async fn delete_document_vectors<S: Storage + ?Sized, I: VectorIndex + ?Sized>(
    doc_id: &crate::types::ContentId,
    storage: &S,
    index: &I,
) -> Result<usize, IngestionError> {
    let mut deleted = 0;

    let vid = VectorId::doc_summary(doc_id.as_ref());
    if index
        .delete(vid.as_str())
        .map_err(|e| IngestionError::Embedding {
            reason: e.to_string(),
        })?
    {
        deleted += 1;
    }

    let sections = storage
        .list_sections(doc_id)
        .await
        .map_err(IngestionError::from)?;

    for section in &sections {
        let vid = VectorId::sec_summary(section.id.as_ref());
        if index
            .delete(vid.as_str())
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?
        {
            deleted += 1;
        }

        let vid = VectorId::section(section.id.as_ref());
        if index
            .delete(vid.as_str())
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?
        {
            deleted += 1;
        }

        let claims = storage
            .list_claims(&section.id)
            .await
            .map_err(IngestionError::from)?;
        for claim in &claims {
            let vid = VectorId::claim(claim.id.as_ref());
            if index
                .delete(vid.as_str())
                .map_err(|e| IngestionError::Embedding {
                    reason: e.to_string(),
                })?
            {
                deleted += 1;
            }
        }
    }

    let doc_record = storage
        .get_document(doc_id)
        .await
        .map_err(IngestionError::from)?;
    if let Some(doc) = doc_record {
        let symbols = storage
            .list_symbols(&crate::storage::SymbolFilter {
                file_path: Some(doc.source_path.clone()),
                ..Default::default()
            })
            .await
            .map_err(IngestionError::from)?;
        for sym in &symbols {
            let stub_vid = VectorId::symbol_stub(sym.id.as_ref());
            if index.delete(stub_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
            let full_vid = VectorId::symbol_full(sym.id.as_ref());
            if index.delete(full_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
        }
        let _ = storage.delete_symbols_for_file(&doc.source_path).await;
    }

    debug!(deleted, doc_id = %doc_id, "deleted document vectors");
    Ok(deleted)
}
