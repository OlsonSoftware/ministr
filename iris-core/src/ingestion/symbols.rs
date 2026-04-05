//! Code symbol extraction, cross-reference resolution, and bridge linking.

use std::path::Path;

use tracing::{debug, info, warn};

use crate::code::bridge::linker::{BridgeLinker, SourceFile as BridgeSourceFile};
use crate::code::bridge::BridgeEndpoint;
use crate::code::package_graph::PackageGraph;
use crate::code::refs::extract_refs;
use crate::code::{AstParser, GrammarRegistry, extract_symbols, generic_extract_symbols};
use crate::error::IngestionError;
use crate::storage::traits::{
    BridgeEndpointRecord, BridgeLinkRecord, PendingRefRecord, Storage, SymbolFilter, SymbolRecord,
    SymbolRefRecord,
};
use crate::types::{RefKind, SymbolId, VectorId};

use super::roots::module_path_from_file;

/// Result of code symbol extraction.
pub(super) struct CodeSymbolsResult {
    pub pending_refs: Vec<PendingRef>,
    pub bridge_endpoints: Vec<BridgeEndpoint>,
    pub embedding_pairs: Vec<(VectorId, String)>,
}

/// A reference that could not be resolved during first-pass ingestion.
#[derive(Debug, Clone)]
pub(super) struct PendingRef {
    pub from_id: SymbolId,
    pub target_name: String,
    pub kind: RefKind,
    pub file_path: String,
    pub target_crate: Option<String>,
}

pub(super) struct ResolveResult {
    pub(super) pending: Vec<PendingRef>,
}

#[allow(clippy::too_many_lines)]
pub(super) async fn extract_code_symbols<S: Storage + ?Sized>(
    relative_path: &str,
    content: &str,
    storage: &S,
    package_graph: Option<&PackageGraph>,
    bridge_linker: Option<&BridgeLinker>,
) -> Result<CodeSymbolsResult, IngestionError> {
    let source = content.as_bytes();
    let empty_result = || {
        Ok(CodeSymbolsResult {
            pending_refs: Vec::new(),
            bridge_endpoints: Vec::new(),
            embedding_pairs: Vec::new(),
        })
    };

    let registry = GrammarRegistry::global();
    let ext = Path::new(relative_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let language = registry.language_name_for_extension(ext);
    let is_rust = language.is_none_or(|l| l == "rust");

    let tree = if is_rust {
        let mut ast_parser = AstParser::new();
        match ast_parser.parse(source) {
            Ok(t) => t,
            Err(_) => return empty_result(),
        }
    } else {
        let Some(ts_lang) = registry.language_for_extension(ext) else {
            return empty_result();
        };
        let Ok(mut ast_parser) = AstParser::with_language(ts_lang) else {
            return empty_result();
        };
        match ast_parser.parse(source) {
            Ok(t) => t,
            Err(_) => return empty_result(),
        }
    };

    let module_parts = module_path_from_file(relative_path);
    let module_path: Vec<&str> = module_parts.iter().map(String::as_str).collect();

    let symbols = if is_rust {
        extract_symbols(&tree, source, relative_path, &module_path)
    } else {
        generic_extract_symbols(&tree, source, relative_path, &module_path)
    };

    if symbols.is_empty() {
        return empty_result();
    }

    let _ = storage.delete_symbols_for_file(relative_path).await;

    let symbol_records: Vec<SymbolRecord> = symbols
        .iter()
        .map(|sym| {
            let module_str = sym.module_path.join("::");
            let qualified_name = if sym.kind == crate::code::ItemKind::Impl {
                format!("impl-{}", sym.name)
            } else {
                sym.name.clone()
            };
            let symbol_id = if module_str.is_empty() {
                format!("sym-{relative_path}::{qualified_name}")
            } else {
                format!("sym-{relative_path}::{module_str}::{qualified_name}")
            };

            #[allow(clippy::cast_possible_truncation)]
            let line_start = content[..sym.byte_range.start].matches('\n').count() as u32 + 1;
            #[allow(clippy::cast_possible_truncation)]
            let line_end = content[..sym.byte_range.end].matches('\n').count() as u32 + 1;

            let cyclomatic_complexity = if sym.kind == crate::code::ItemKind::Function {
                tree.root_node()
                    .descendant_for_byte_range(sym.byte_range.start, sym.byte_range.end)
                    .map(|node| crate::code::cyclomatic_complexity(&node, source))
            } else {
                None
            };

            SymbolRecord {
                id: SymbolId(symbol_id),
                file_path: relative_path.to_string(),
                name: sym.name.clone(),
                kind: sym.kind.as_str().to_string(),
                visibility: sym.visibility.as_str().to_string(),
                signature: sym.signature.clone(),
                doc_comment: sym.doc_comment.clone(),
                module_path: module_str,
                line_start,
                line_end,
                cyclomatic_complexity,
            }
        })
        .collect();

    storage
        .insert_symbols(&symbol_records)
        .await
        .map_err(IngestionError::from)?;

    let language = Path::new(relative_path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| crate::code::GrammarRegistry::global().language_name_for_extension(ext))
        .unwrap_or("rust");
    let pending_refs = match resolve_and_store_refs(
        &tree,
        source,
        relative_path,
        language,
        &symbol_records,
        storage,
        package_graph,
    )
    .await
    {
        Ok(result) => result.pending,
        Err(e) => {
            warn!(path = %relative_path, error = %e, "failed to extract symbol refs");
            Vec::new()
        }
    };

    let bridge_endpoints = if let Some(linker) = bridge_linker {
        let language_name = language;
        let sf = BridgeSourceFile {
            file_path: relative_path,
            language: language_name,
            tree: &tree,
            source,
        };
        linker.extract_all(&[sf])
    } else {
        Vec::new()
    };

    let mut embedding_pairs: Vec<(VectorId, String)> = Vec::new();
    for (sym, record) in symbols.iter().zip(symbol_records.iter()) {
        let stub_text = match &sym.doc_comment {
            Some(doc) => format!("{}\n{doc}", sym.signature),
            None => sym.signature.clone(),
        };
        if !stub_text.trim().is_empty() {
            embedding_pairs.push((VectorId::symbol_stub(record.id.as_ref()), stub_text));
        }

        let full_text = &content[sym.byte_range.clone()];
        if !full_text.trim().is_empty() {
            embedding_pairs.push((
                VectorId::symbol_full(record.id.as_ref()),
                full_text.to_string(),
            ));
        }
    }

    debug!(
        symbols = embedding_pairs.len(),
        path = %relative_path,
        "extracted code symbol embeddings"
    );
    Ok(CodeSymbolsResult {
        pending_refs,
        bridge_endpoints,
        embedding_pairs,
    })
}

// ── Reference resolution ─────────────────────────────────────────────────────

/// Resolve target symbols using a shared disambiguation strategy.
///
/// Both first-pass and second-pass resolution need the same logic:
/// filter to primary definitions, disambiguate via package graph, prefer cross-file.
fn disambiguate_target<'a>(
    primary: &[&'a SymbolRecord],
    file_path: &str,
    target_crate: Option<&str>,
    package_graph: Option<&PackageGraph>,
) -> Option<&'a SymbolRecord> {
    match primary.len() {
        0 => None,
        1 => Some(primary[0]),
        _ => {
            let crate_filtered: Vec<_> =
                if let (Some(tc), Some(graph)) = (target_crate, package_graph) {
                    if let Some(dir_prefix) = graph.dir_prefix_for_crate(tc) {
                        primary
                            .iter()
                            .filter(|s| s.file_path.starts_with(dir_prefix))
                            .copied()
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

            if crate_filtered.len() == 1 {
                Some(crate_filtered[0])
            } else if !crate_filtered.is_empty() {
                Some(
                    crate_filtered
                        .iter()
                        .find(|s| s.file_path != file_path)
                        .copied()
                        .unwrap_or(crate_filtered[0]),
                )
            } else {
                Some(
                    primary
                        .iter()
                        .find(|s| s.file_path != file_path)
                        .copied()
                        .unwrap_or(primary[0]),
                )
            }
        }
    }
}

/// Filter symbols to primary definitions (not impl blocks or nested items).
fn filter_primary(matches: &[SymbolRecord]) -> Vec<&SymbolRecord> {
    matches
        .iter()
        .filter(|s| {
            matches!(
                s.kind.as_str(),
                "struct" | "enum" | "trait" | "function" | "type" | "const" | "static" | "mod"
            )
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
pub(super) async fn resolve_and_store_refs<S: Storage + ?Sized>(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &str,
    local_symbols: &[SymbolRecord],
    storage: &S,
    package_graph: Option<&PackageGraph>,
) -> Result<ResolveResult, IngestionError> {
    let raw_refs = extract_refs(tree, source, language);
    if raw_refs.is_empty() {
        return Ok(ResolveResult {
            pending: Vec::new(),
        });
    }

    let local_id_set: std::collections::HashSet<&SymbolId> =
        local_symbols.iter().map(|s| &s.id).collect();

    let _ = storage.delete_refs_for_file(file_path).await;

    let file_anchor = local_symbols
        .iter()
        .find(|s| s.kind == "mod")
        .or_else(|| {
            local_symbols.iter().find(|s| {
                matches!(
                    s.kind.as_str(),
                    "struct" | "enum" | "trait" | "function" | "type"
                )
            })
        })
        .or(local_symbols.first())
        .map(|s| s.id.clone());

    let mut resolved = Vec::new();
    let mut pending = Vec::new();

    for raw in &raw_refs {
        let from_id = match &raw.from_context {
            Some(type_name) => local_symbols
                .iter()
                .find(|s| {
                    s.name == *type_name
                        && (s.kind == "struct" || s.kind == "enum" || s.kind == "type")
                })
                .map(|s| s.id.clone()),
            None => file_anchor.clone(),
        };

        let Some(from_id) = from_id else {
            continue;
        };

        let target_filter = SymbolFilter {
            name_exact: Some(raw.target_name.clone()),
            ..SymbolFilter::default()
        };

        let Ok(matches) = storage.list_symbols(&target_filter).await else {
            continue;
        };

        let primary = filter_primary(&matches);

        let Some(target) = disambiguate_target(
            &primary,
            file_path,
            raw.target_crate.as_deref(),
            package_graph,
        ) else {
            pending.push(PendingRef {
                from_id,
                target_name: raw.target_name.clone(),
                kind: raw.kind,
                file_path: file_path.to_owned(),
                target_crate: raw.target_crate.clone(),
            });
            continue;
        };

        if from_id == target.id {
            continue;
        }

        if !local_id_set.contains(&from_id) {
            continue;
        }

        resolved.push(SymbolRefRecord {
            from_symbol_id: from_id,
            to_symbol_id: target.id.clone(),
            ref_kind: raw.kind,
        });
    }

    if resolved.is_empty() {
        return Ok(ResolveResult {
            pending,
        });
    }

    let mut inserted = 0usize;
    for r in &resolved {
        if storage
            .insert_symbol_refs(std::slice::from_ref(r))
            .await
            .is_ok()
        {
            inserted += 1;
        }
    }

    if inserted > 0 {
        debug!(
            refs = inserted,
            pending = pending.len(),
            path = %file_path,
            "resolved symbol cross-references"
        );
    }

    Ok(ResolveResult { pending })
}

/// Second-pass: resolve refs whose targets weren't indexed during first pass.
pub(super) async fn resolve_pending_refs<S: Storage + ?Sized>(
    pending: &[PendingRef],
    storage: &S,
    package_graph: Option<&PackageGraph>,
) -> (usize, Vec<PendingRef>) {
    if pending.is_empty() {
        return (0, Vec::new());
    }

    let mut resolved = 0;
    let mut still_unresolved = Vec::new();

    for pr in pending {
        let target_filter = SymbolFilter {
            name_exact: Some(pr.target_name.clone()),
            ..SymbolFilter::default()
        };

        let Ok(matches) = storage.list_symbols(&target_filter).await else {
            continue;
        };

        let primary = filter_primary(&matches);

        let Some(target) = disambiguate_target(
            &primary,
            &pr.file_path,
            pr.target_crate.as_deref(),
            package_graph,
        ) else {
            still_unresolved.push(pr.clone());
            continue;
        };

        if pr.from_id == target.id {
            continue;
        }

        let record = SymbolRefRecord {
            from_symbol_id: pr.from_id.clone(),
            to_symbol_id: target.id.clone(),
            ref_kind: pr.kind,
        };

        if storage
            .insert_symbol_refs(std::slice::from_ref(&record))
            .await
            .is_ok()
        {
            resolved += 1;
        }
    }

    if resolved > 0 {
        debug!(
            refs = resolved,
            still_pending = still_unresolved.len(),
            total_pending = pending.len(),
            "second-pass reference resolution"
        );
    }

    (resolved, still_unresolved)
}

/// Persist unresolved pending refs to SQLite for warm-restart resolution.
pub(super) async fn persist_pending_refs<S: Storage + ?Sized>(pending: &[PendingRef], storage: &S) {
    if pending.is_empty() {
        return;
    }
    let records: Vec<PendingRefRecord> = pending
        .iter()
        .map(|pr| PendingRefRecord {
            from_symbol_id: pr.from_id.0.clone(),
            target_name: pr.target_name.clone(),
            kind: format!("{:?}", pr.kind),
            file_path: pr.file_path.clone(),
            target_crate: pr.target_crate.clone(),
        })
        .collect();
    if let Err(e) = storage.upsert_pending_refs(&records).await {
        warn!(error = %e, "failed to persist pending refs");
    } else {
        debug!(
            count = records.len(),
            "persisted pending refs for deferred resolution"
        );
    }
}

/// Scan for impl symbols missing `implements` refs and repair them.
pub(super) async fn repair_missing_refs<S: Storage + ?Sized>(
    storage: &S,
    _package_graph: Option<&PackageGraph>,
) {
    let impl_filter = SymbolFilter {
        kind: Some("impl".to_string()),
        ..SymbolFilter::default()
    };
    let Ok(impls) = storage.list_symbols(&impl_filter).await else {
        return;
    };

    let mut repaired = 0;

    for imp in &impls {
        let Some(trait_name) = extract_trait_from_impl_signature(&imp.signature) else {
            continue;
        };
        let Some(type_name) = extract_type_from_impl_signature(&imp.signature) else {
            continue;
        };

        let type_filter = SymbolFilter {
            name_exact: Some(type_name.clone()),
            file_path: Some(imp.file_path.clone()),
            ..SymbolFilter::default()
        };
        let Ok(type_matches) = storage.list_symbols(&type_filter).await else {
            continue;
        };
        let Some(from_sym) = type_matches.first() else {
            continue;
        };

        let trait_filter = SymbolFilter {
            name_exact: Some(trait_name.clone()),
            kind: Some("trait".to_string()),
            ..SymbolFilter::default()
        };
        let Ok(trait_matches) = storage.list_symbols(&trait_filter).await else {
            continue;
        };
        let Some(to_sym) = trait_matches.first() else {
            continue;
        };

        let Ok(existing_refs) = storage
            .query_refs(&from_sym.id, Some(RefKind::Implements))
            .await
        else {
            continue;
        };
        if existing_refs.iter().any(|r| r.to_symbol_id == to_sym.id) {
            continue;
        }

        let record = SymbolRefRecord {
            from_symbol_id: from_sym.id.clone(),
            to_symbol_id: to_sym.id.clone(),
            ref_kind: RefKind::Implements,
        };
        if storage
            .insert_symbol_refs(std::slice::from_ref(&record))
            .await
            .is_ok()
        {
            repaired += 1;
        }
    }

    if repaired > 0 {
        info!(
            refs = repaired,
            "repaired missing implements refs on warm restart"
        );
    }
}

pub(super) fn extract_trait_from_impl_signature(sig: &str) -> Option<String> {
    let after_impl = sig.strip_prefix("impl ")?;
    let for_pos = after_impl.find(" for ")?;
    let trait_part = &after_impl[..for_pos];
    let trait_name = trait_part.split('<').next()?.trim();
    if trait_name.is_empty() {
        return None;
    }
    Some(trait_name.to_string())
}

pub(super) fn extract_type_from_impl_signature(sig: &str) -> Option<String> {
    let for_pos = sig.find(" for ")?;
    let type_part = &sig[for_pos + 5..];
    let type_name = type_part.split(['<', '\n', '{']).next()?.trim();
    if type_name.is_empty() {
        return None;
    }
    Some(type_name.to_string())
}

/// Run the bridge linker on accumulated endpoints and store results.
pub(super) async fn store_bridge_links<S: Storage + ?Sized>(
    endpoints: &[BridgeEndpoint],
    linker: Option<&BridgeLinker>,
    storage: &S,
) {
    let Some(linker) = linker else { return };
    if endpoints.is_empty() {
        return;
    }

    let links = linker.link(endpoints);

    let ep_records: Vec<BridgeEndpointRecord> = endpoints
        .iter()
        .map(|ep| BridgeEndpointRecord {
            id: None,
            file_path: ep.file_path.clone(),
            binding_key: ep.binding_key.clone(),
            kind: ep.kind.as_str().to_string(),
            role: ep.role.as_str().to_string(),
            language: ep.language.clone(),
            line: ep.line,
            symbol_name: ep.symbol_name.clone(),
            confidence: ep.confidence,
        })
        .collect();

    let Ok(ep_ids) = storage.insert_bridge_endpoints(&ep_records).await else {
        warn!("failed to insert bridge endpoints");
        return;
    };

    let link_records: Vec<BridgeLinkRecord> = links
        .iter()
        .filter_map(|link| {
            let export_id = find_endpoint_id(endpoints, &ep_ids, &link.export)?;
            let import_id = find_endpoint_id(endpoints, &ep_ids, &link.import)?;
            Some(BridgeLinkRecord {
                export_ep_id: export_id,
                import_ep_id: import_id,
                kind: link.kind.as_str().to_string(),
                confidence: link.confidence,
            })
        })
        .collect();

    if let Err(e) = storage.insert_bridge_links(&link_records).await {
        warn!(error = %e, "failed to insert bridge links");
        return;
    }

    info!(
        endpoints = ep_records.len(),
        links = link_records.len(),
        "bridge extraction complete"
    );
}

fn find_endpoint_id(
    endpoints: &[BridgeEndpoint],
    ids: &[i64],
    target: &BridgeEndpoint,
) -> Option<i64> {
    endpoints.iter().zip(ids.iter()).find_map(|(ep, &id)| {
        (ep.file_path == target.file_path && ep.line == target.line && ep.role == target.role)
            .then_some(id)
    })
}
