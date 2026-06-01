//! Embedding: dense, sparse, batch insertion, and vector deletion.

use tracing::{debug, info, warn};

use crate::embedding::{DualEmbedder, Embedder};
use crate::error::IngestionError;
use crate::index::VectorIndex;
use crate::mem_profile;
use crate::storage::{SqliteStorage, Storage};
use crate::types::{ContentId, DocumentTree, Section, VectorId};

/// Maximum texts per embedding inference call.
pub(super) const EMBED_BATCH_CHUNK: usize = 128;

/// Accumulated pairs that trigger an intermediate flush.
pub(super) const EMBED_FLUSH_THRESHOLD: usize = 4096;

/// Embed a document tree at all three resolution levels (immediate).
pub(super) async fn embed_document<
    E: Embedder + ?Sized,
    I: VectorIndex + ?Sized,
    S: Storage + ?Sized,
>(
    doc: &DocumentTree,
    embedder: &E,
    index: &I,
    storage: &S,
) -> Result<usize, IngestionError> {
    let mut texts: Vec<String> = Vec::new();
    let mut ids: Vec<VectorId> = Vec::new();

    if let Some(ref summary) = doc.summary
        && !summary.trim().is_empty()
    {
        ids.push(VectorId::doc_summary(doc.id.as_ref()));
        texts.push(summary.clone());
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

    // D4: persist the exact indexed vectors so the index can be rebuilt from
    // SQLite on load (the immediate-embed entry point, used by content ingest).
    let indexed: Vec<(String, Vec<f32>)> = ids
        .iter()
        .zip(vectors.iter())
        .map(|(vid, v)| (vid.to_string(), v.clone()))
        .collect();
    storage
        .store_indexed_vectors(&indexed)
        .await
        .map_err(IngestionError::from)?;

    let count = ids.len();
    debug!(embeddings = count, doc_id = %doc.id, "embedded document");
    Ok(count)
}

/// Collect embeddable `(id, text)` pairs from a document tree without embedding.
pub(super) fn collect_document_embeddings(doc: &DocumentTree, pairs: &mut Vec<(VectorId, String)>) {
    if let Some(ref summary) = doc.summary
        && !summary.trim().is_empty()
    {
        pairs.push((VectorId::doc_summary(doc.id.as_ref()), summary.clone()));
    }

    let mut ids = Vec::new();
    let mut texts = Vec::new();
    collect_embeddable_items(&doc.sections, &mut ids, &mut texts);
    pairs.extend(ids.into_iter().zip(texts));
}

/// Embed and insert a batch of `(id, text)` pairs into the vector index.
///
/// When `service` is `Some`, each chunk is embedded through the dedicated
/// [`EmbeddingService`](crate::embedding::EmbeddingService) (ADR 0001 D1): the
/// model runs on the service's own thread and this task `await`s without
/// blocking a Tokio worker. When `None`, the embedder is called inline (the
/// path tests / `ministr index` / web fetch use).
pub(super) async fn batch_embed_and_insert<
    E: Embedder + ?Sized,
    I: VectorIndex + ?Sized,
    S: Storage + ?Sized,
>(
    pairs: &[(VectorId, String)],
    embedder: &E,
    service: Option<&crate::embedding::EmbeddingService>,
    index: &I,
    storage: &S,
) -> Result<usize, IngestionError> {
    if pairs.is_empty() {
        return Ok(0);
    }
    let mut total = 0;
    let num_chunks = pairs.len().div_ceil(EMBED_BATCH_CHUNK);

    for (i, chunk) in pairs.chunks(EMBED_BATCH_CHUNK).enumerate() {
        mem_profile::checkpoint_every(5, i, "before embedder.embed()");
        let vectors = if let Some(svc) = service {
            // Off-runtime: the service thread owns the model; we await its
            // reply without pinning a Tokio worker for the GPU call.
            let texts: Vec<String> = chunk.iter().map(|(_, t)| t.clone()).collect();
            svc.embed(texts)
                .await
                .map_err(|e| IngestionError::Embedding {
                    reason: e.to_string(),
                })?
        } else {
            let text_refs: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
            embedder
                .embed(&text_refs)
                .map_err(|e| IngestionError::Embedding {
                    reason: e.to_string(),
                })?
        };
        mem_profile::checkpoint_every(5, i, "after embedder.embed()");

        for ((vid, _), vector) in chunk.iter().zip(vectors.iter()) {
            index
                .insert(vid.as_str(), vector)
                .map_err(|e| IngestionError::Embedding {
                    reason: format!("failed to insert vector {vid}: {e}"),
                })?;
        }

        // D4: persist the exact indexed vectors in the ACID store so the
        // in-memory HNSW can be rebuilt from SQLite on load. The rebuild
        // re-applies the degenerate guard, so storing every vector here
        // (zeros included) stays consistent with what the index accepts.
        let indexed: Vec<(String, Vec<f32>)> = chunk
            .iter()
            .zip(vectors.iter())
            .map(|((vid, _), v)| (vid.to_string(), v.clone()))
            .collect();
        storage
            .store_indexed_vectors(&indexed)
            .await
            .map_err(IngestionError::from)?;

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

        // D4: persist the exact INDEXED (truncated) vectors as the rebuild
        // source of truth — distinct from the full-dim rerank vectors stored
        // below. This is what `rebuild_hnsw_from_store` reconstructs from.
        let indexed_entries: Vec<(String, Vec<f32>)> = chunk
            .iter()
            .zip(dual.truncated.iter())
            .map(|((vid, _), vec)| (vid.to_string(), vec.clone()))
            .collect();
        storage
            .store_indexed_vectors(&indexed_entries)
            .await
            .map_err(IngestionError::from)?;

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
        if let Some(ref summary) = section.summary
            && !summary.trim().is_empty()
        {
            ids.push(VectorId::sec_summary(section.id.as_ref()));
            texts.push(summary.clone());
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
pub(crate) async fn delete_document_vectors<S: Storage + ?Sized, I: VectorIndex + ?Sized>(
    doc_id: &crate::types::ContentId,
    storage: &S,
    index: &I,
) -> Result<usize, IngestionError> {
    let mut deleted = 0;
    // D4: collect every vector id we remove from the index so we can also
    // delete it from the `indexed_vectors` source of truth — SQLite and a
    // future rebuild must never disagree about what was indexed. Collected
    // unconditionally (a vector stored but skipped by the index guard still
    // has a row to clean up).
    let mut removed_vids: Vec<String> = Vec::new();

    let vid = VectorId::doc_summary(doc_id.as_ref());
    removed_vids.push(vid.as_str().to_owned());
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
        removed_vids.push(vid.as_str().to_owned());
        if index
            .delete(vid.as_str())
            .map_err(|e| IngestionError::Embedding {
                reason: e.to_string(),
            })?
        {
            deleted += 1;
        }

        let vid = VectorId::section(section.id.as_ref());
        removed_vids.push(vid.as_str().to_owned());
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
            removed_vids.push(vid.as_str().to_owned());
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
            removed_vids.push(stub_vid.as_str().to_owned());
            if index.delete(stub_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
            let full_vid = VectorId::symbol_full(sym.id.as_ref());
            removed_vids.push(full_vid.as_str().to_owned());
            if index.delete(full_vid.as_str()).unwrap_or(false) {
                deleted += 1;
            }
        }
        let _ = storage.delete_symbols_for_file(&doc.source_path).await;
    }

    // D4: remove the same ids from the indexed_vectors source of truth.
    let refs: Vec<&str> = removed_vids.iter().map(String::as_str).collect();
    storage
        .delete_indexed_vectors(&refs)
        .await
        .map_err(IngestionError::from)?;

    debug!(deleted, doc_id = %doc_id, "deleted document vectors");
    Ok(deleted)
}

/// Roll back a set of partially-indexed documents after an embedding failure:
/// delete each document's vectors, its storage record, and its file-hash so
/// `SQLite` and the vector index never disagree about whether a file was
/// indexed — the Persist-stage "no partial document" invariant. Best-effort:
/// per-document failures are logged and skipped, since the caller is already on
/// the error path.
pub(crate) async fn rollback_partial_documents<S, I>(docs: &[ContentId], storage: &S, index: &I)
where
    S: Storage + ?Sized,
    I: VectorIndex + ?Sized,
{
    for doc_id in docs {
        if let Err(e) = delete_document_vectors(doc_id, storage, index).await {
            warn!(doc_id = %doc_id, error = %e, "rollback: delete vectors failed");
        }
        if let Err(e) = storage.delete_document(doc_id).await {
            warn!(doc_id = %doc_id, error = %e, "rollback: delete document failed");
        }
        if let Err(e) = storage.delete_file_hash(&doc_id.0).await {
            warn!(doc_id = %doc_id, error = %e, "rollback: delete file hash failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::HnswIndex;

    #[tokio::test]
    async fn rollback_deletes_a_documents_vectors() {
        let storage = SqliteStorage::open_in_memory().expect("storage");
        let index = HnswIndex::new(4, 16).expect("hnsw index");
        index
            .insert(
                VectorId::doc_summary("doc1").as_str(),
                &[1.0, 0.0, 0.0, 0.0],
            )
            .expect("insert vector");
        assert_eq!(index.len(), 1);

        rollback_partial_documents(
            std::slice::from_ref(&ContentId("doc1".to_owned())),
            &storage,
            &index,
        )
        .await;

        assert_eq!(index.len(), 0, "rollback removes the document's vectors");
    }
}
