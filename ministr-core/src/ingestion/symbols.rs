//! Code symbol extraction, cross-reference resolution, and bridge linking.

use std::path::Path;

use tracing::{debug, info, warn};

use crate::code::bridge::BridgeEndpoint;
use crate::code::bridge::linker::{BridgeLinker, SourceFile as BridgeSourceFile};
use crate::code::package_graph::PackageGraph;
use crate::code::refs::extract_refs;
use crate::code::{AstParser, GrammarRegistry, extract_symbols, generic_extract_symbols_for};
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
) -> Result<CodeSymbolsResult, IngestionError> {
    let source = content.as_bytes();
    let empty_result = || {
        Ok(CodeSymbolsResult {
            pending_refs: Vec::new(),
            embedding_pairs: Vec::new(),
        })
    };

    let registry = GrammarRegistry::global();
    let ext = Path::new(relative_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    // Shader languages take a separate path: there's no tree-sitter
    // grammar, so we drive a Logos lexer to pull out top-level
    // declarations. Skips ref / bridge resolution for now — adding
    // shader includes into the cross-symbol graph is a follow-up.
    if crate::code::hlsl::is_shader_extension(ext) {
        return extract_shader_symbols(relative_path, content, storage).await;
    }

    let language = registry.language_name_for_extension(ext);
    let is_rust = language.is_none_or(|l| l == "rust");
    let is_cpp_family = matches!(language, Some("cpp" | "c"));

    let tree = if is_rust {
        let Ok(mut ast_parser) = AstParser::try_new() else {
            return empty_result();
        };
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
        let Ok(t) = ast_parser.parse(source) else {
            // C/C++ has the deepest pathological-input risk
            // (recursive templates, Slate widgets). Run the
            // Logos fallback so we still recover top-level
            // class/struct/enum/function/UE-macro symbols.
            if is_cpp_family {
                return extract_cpp_fallback_into_storage(relative_path, content, storage).await;
            }
            return empty_result();
        };
        t
    };

    let module_parts = module_path_from_file(relative_path);
    let module_path: Vec<&str> = module_parts.iter().map(String::as_str).collect();

    let symbols = if is_rust {
        extract_symbols(&tree, source, relative_path, &module_path)
    } else {
        generic_extract_symbols_for(&tree, source, relative_path, &module_path, language)
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

    // PHASE4 chunk 4: per-file bridge extraction was here. It was
    // expensive (extract_all parses every applicable language extractor
    // per file) and the result was accumulated into all_bridge_endpoints
    // and discarded — finalize_ingestion rebuilds bridges from
    // all_files anyway. The full-corpus rebuild is the authoritative
    // source; doing it here too is duplicate work.

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
        embedding_pairs,
    })
}

// ── C/C++ fallback (Logos-driven, when tree-sitter timed out) ─────────────────

/// Run the Logos C++ fallback when the tree-sitter parse couldn't
/// complete (most commonly: per-file timeout on a pathological
/// template or Slate widget). Recovers top-level class / struct /
/// enum / function / Unreal-reflection-macro symbols so the file
/// still appears in `ministr_symbols`. Skips ref / bridge resolution.
async fn extract_cpp_fallback_into_storage<S: Storage + ?Sized>(
    relative_path: &str,
    content: &str,
    storage: &S,
) -> Result<CodeSymbolsResult, IngestionError> {
    let extraction =
        crate::code::cpp_fallback::extract_cpp_fallback_symbols(content, relative_path);

    if extraction.symbols.is_empty() {
        let _ = storage.delete_symbols_for_file(relative_path).await;
        return Ok(CodeSymbolsResult {
            pending_refs: Vec::new(),
            embedding_pairs: Vec::new(),
        });
    }

    let _ = storage.delete_symbols_for_file(relative_path).await;
    storage
        .insert_symbols(&extraction.symbols)
        .await
        .map_err(IngestionError::from)?;

    let bytes = content.as_bytes();
    let mut embedding_pairs: Vec<(VectorId, String)> = Vec::new();
    for record in &extraction.symbols {
        let stub_text = if record.signature.trim().is_empty() {
            record.name.clone()
        } else {
            record.signature.clone()
        };
        if !stub_text.trim().is_empty() {
            embedding_pairs.push((VectorId::symbol_stub(record.id.as_ref()), stub_text.clone()));
        }
        if let Some(full_text) = slice_lines(bytes, record.line_start, record.line_end)
            && !full_text.trim().is_empty()
        {
            embedding_pairs.push((
                VectorId::symbol_full(record.id.as_ref()),
                full_text.to_string(),
            ));
        }
    }

    info!(
        path = %relative_path,
        symbols = extraction.symbols.len(),
        "tree-sitter parse failed for C/C++ — recovered symbols via Logos fallback"
    );

    Ok(CodeSymbolsResult {
        pending_refs: Vec::new(),
        embedding_pairs,
    })
}

// ── Shader extraction (Logos-driven, no tree-sitter) ──────────────────────────

/// Run the HLSL/GLSL/MSL/WGSL Logos extractor and persist its symbols.
///
/// Mirrors the tree-sitter path's storage shape — wipes the file's
/// previous symbol rows, inserts the freshly-extracted ones, and
/// returns the embedding pairs for the deferred batch embed. Skips
/// ref / bridge resolution: shader includes will eventually flow into
/// the `RawRef` graph but that's out of scope here.
async fn extract_shader_symbols<S: Storage + ?Sized>(
    relative_path: &str,
    content: &str,
    storage: &S,
) -> Result<CodeSymbolsResult, IngestionError> {
    let extraction = crate::code::hlsl::extract_hlsl_symbols(content, relative_path);

    if extraction.symbols.is_empty() {
        // Still wipe stale rows in case a previous indexer pass had
        // produced symbols for this file — keeps the index honest
        // when a shader gets edited down to nothing.
        let _ = storage.delete_symbols_for_file(relative_path).await;
        return Ok(CodeSymbolsResult {
            pending_refs: Vec::new(),
            embedding_pairs: Vec::new(),
        });
    }

    let _ = storage.delete_symbols_for_file(relative_path).await;
    storage
        .insert_symbols(&extraction.symbols)
        .await
        .map_err(IngestionError::from)?;

    let bytes = content.as_bytes();
    let mut embedding_pairs: Vec<(VectorId, String)> = Vec::new();
    for record in &extraction.symbols {
        let stub_text = if record.signature.trim().is_empty() {
            record.name.clone()
        } else {
            record.signature.clone()
        };
        if !stub_text.trim().is_empty() {
            embedding_pairs.push((VectorId::symbol_stub(record.id.as_ref()), stub_text.clone()));
        }

        if let Some(full_text) = slice_lines(bytes, record.line_start, record.line_end)
            && !full_text.trim().is_empty()
        {
            embedding_pairs.push((
                VectorId::symbol_full(record.id.as_ref()),
                full_text.to_string(),
            ));
        }
    }

    debug!(
        path = %relative_path,
        symbols = extraction.symbols.len(),
        includes = extraction.includes.len(),
        "extracted shader symbols (Logos)"
    );

    Ok(CodeSymbolsResult {
        pending_refs: Vec::new(),
        embedding_pairs,
    })
}

/// Slice the inclusive 1-based line range `[start, end]` out of `bytes`
/// as UTF-8 text. Returns `None` if the range is out of bounds or the
/// slice isn't valid UTF-8 (rare, since we got the offsets by hashing
/// the same file's bytes).
fn slice_lines(bytes: &[u8], start_line: u32, end_line: u32) -> Option<&str> {
    if start_line == 0 || end_line < start_line {
        return None;
    }
    let mut line: u32 = 1;
    let mut start_off: Option<usize> = None;
    let mut end_off: Option<usize> = None;
    if start_line == 1 {
        start_off = Some(0);
    }
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line += 1;
            if start_off.is_none() && line == start_line {
                start_off = Some(i + 1);
            }
            if line > end_line {
                end_off = Some(i);
                break;
            }
        }
    }
    let end = end_off.unwrap_or(bytes.len());
    let start = start_off?;
    if start > end {
        return None;
    }
    std::str::from_utf8(&bytes[start..end]).ok()
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
            // When the ref carries an explicit `target_crate` (e.g., from
            // `use ministr_core::Foo`), filter to that crate first — that's
            // ground truth.
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
                return Some(crate_filtered[0]);
            }
            if !crate_filtered.is_empty() {
                return Some(
                    crate_filtered
                        .iter()
                        .find(|s| s.file_path != file_path)
                        .copied()
                        .unwrap_or(crate_filtered[0]),
                );
            }

            // No explicit target_crate (or unknown crate): prefer matches
            // in the source's *own* package. When two crates each define a
            // symbol with the same name (deliberate wire-type duplication,
            // a re-export, or just a coincidence), Rust's compiler would
            // resolve to the in-scope local one — so should we. Falling
            // back to "prefer cross-file" caused phantom cross-crate edges
            // for every same-name twin.
            if let Some(graph) = package_graph
                && let Some(source_pkg) = graph.package_for_file(file_path)
                && let Some(source_dir) = graph.dir_prefix_for_crate(source_pkg)
            {
                let same_pkg: Vec<_> = primary
                    .iter()
                    .filter(|s| s.file_path.starts_with(source_dir))
                    .copied()
                    .collect();
                if !same_pkg.is_empty() {
                    return Some(
                        same_pkg
                            .iter()
                            .find(|s| s.file_path != file_path)
                            .copied()
                            .unwrap_or(same_pkg[0]),
                    );
                }
            }

            // Final fallback: prefer cross-file among all primary matches.
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

/// Find the symbol whose `[line_start, line_end]` range tightly encloses
/// `line`. When several symbols enclose it (e.g. a function inside a `mod`
/// block), pick the smallest (most-specific) range. Returns `None` when no
/// local symbol owns the line.
///
/// Used as a fallback when extraction couldn't determine a `from_context`
/// for a ref — without it, every such ref gets attributed to whichever
/// top-of-file decl happens to be the "file anchor," producing phantom
/// edges from unrelated types to the body's actual targets.
fn enclosing_symbol_id(local_symbols: &[SymbolRecord], line: u32) -> Option<SymbolId> {
    let mut best: Option<&SymbolRecord> = None;
    for s in local_symbols {
        if s.line_start > line || s.line_end < line {
            continue;
        }
        // Functions / methods / impls are the symbol kinds that can
        // legitimately own a body-level ref. Type declarations (`struct`,
        // `enum`, `trait`) only own refs that appear inside their own
        // signature / variants / field types — those are extracted with
        // `from_context` already set, so this fallback path shouldn't
        // attribute body-level refs to a same-line enum decl.
        let is_owner_kind = matches!(s.kind.as_str(), "function" | "method" | "impl" | "mod");
        if !is_owner_kind {
            continue;
        }
        let candidate_span = s.line_end.saturating_sub(s.line_start);
        let current_span = best.map(|b| b.line_end.saturating_sub(b.line_start));
        if best.is_none() || candidate_span < current_span.unwrap_or(u32::MAX) {
            best = Some(s);
        }
    }
    best.map(|s| s.id.clone())
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

/// Returns `true` when `source_path` and `target_path` resolve to the
/// same language via the grammar registry.
///
/// Cross-language phantom bindings are never legitimate for the
/// `Calls` / `Uses` / `Imports` ref kinds — a Rust `bundle::header` use
/// site can't reasonably bind to a TSX `<Header>` component, even if
/// both share the bare name. The intentional cross-language paths are
/// handled separately as `RefKind::Bridge` by the bridge linker, which
/// runs language-aware extractors on each end and never traverses
/// `disambiguate_target`.
///
/// Unknown extensions on either side return `false` — a hard "skip" is
/// safer than a wide-open match against arbitrary file kinds.
///
/// JavaScript/TypeScript are a single resolution family. The grammar
/// registry keeps `tsx` as its own grammar (distinct from `typescript`)
/// because TSX needs a separate tree-sitter parser, and JSX collapses
/// into `javascript`. But all of `.ts`/`.tsx`/`.js`/`.jsx`/`.mts`/`.cts`/
/// `.mjs`/`.cjs` share one ES module system and routinely cross-import —
/// a `.tsx` component importing `cn` from a `.ts` util is the canonical
/// case. Treating those distinct grammar names as incompatible silently
/// dropped every `.tsx`→`.ts` cross-file reference (the real-world
/// `cn`/`corpusLabel` "no related files" bug). Collapse them to one
/// family for ref-compatibility so the import edge resolves.
fn languages_compatible(source_path: &str, target_path: &str) -> bool {
    /// Map a canonical grammar name to its ref-resolution family. Only
    /// the JS/TS ecosystem needs collapsing; every other language is its
    /// own family (identity), preserving the strict cross-language guard.
    fn family(lang: &str) -> &str {
        match lang {
            "javascript" | "typescript" | "tsx" => "js_ts",
            other => other,
        }
    }
    fn lang_of(path: &str) -> Option<&'static str> {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())?;
        crate::code::GrammarRegistry::global().language_name_for_extension(ext)
    }
    match (lang_of(source_path), lang_of(target_path)) {
        (Some(a), Some(b)) => family(a) == family(b),
        _ => false,
    }
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
            Some(ctx_name) => {
                // The from_context may be a type name (for impl refs) or a
                // function/method name (for call/use refs extracted from bodies).
                // Try matching against all symbol kinds that can be a "source".
                let found = local_symbols
                    .iter()
                    .find(|s| {
                        s.name == *ctx_name
                            && matches!(
                                s.kind.as_str(),
                                "struct" | "enum" | "type" | "function" | "trait"
                            )
                    })
                    .map(|s| s.id.clone());
                // Fall back to the innermost local symbol whose line range
                // contains this ref, then to the file anchor. The line-range
                // attribution prevents function-body refs from being
                // misattributed to whatever decl happens to sit at the top
                // of the file.
                found
                    .or_else(|| enclosing_symbol_id(local_symbols, raw.line))
                    .or_else(|| file_anchor.clone())
            }
            None => enclosing_symbol_id(local_symbols, raw.line).or_else(|| file_anchor.clone()),
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

        // Drop cross-language candidates before disambiguation. Without
        // this a Rust ref can phantom-bind to a TSX symbol whose only
        // commonality is the bare name (see `languages_compatible`).
        let same_lang: Vec<SymbolRecord> = matches
            .into_iter()
            .filter(|s| languages_compatible(file_path, &s.file_path))
            .collect();

        let primary = filter_primary(&same_lang);

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
        return Ok(ResolveResult { pending });
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

        // Same cross-language guard as the first-pass resolver. A pending
        // ref whose only same-name candidates live in a different
        // language gets dropped instead of phantom-binding.
        let same_lang: Vec<SymbolRecord> = matches
            .into_iter()
            .filter(|s| languages_compatible(&pr.file_path, &s.file_path))
            .collect();

        let primary = filter_primary(&same_lang);

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

/// Re-extract bridge endpoints from every tracked code file in `files`.
///
/// Bridge data is a global view derived from current extractor logic — per-file
/// content hashes do NOT capture extractor-rule changes, so changes to
/// extractor code won't propagate to unchanged files via the incremental
/// ingest path. This helper re-reads and re-parses every code file in
/// `files` and runs `linker.extract_all` over the full set, yielding a
/// complete view that `store_bridge_links` can then link and persist.
///
/// Non-code files (no recognized language) are silently skipped. I/O or
/// parse failures on individual files are logged at DEBUG and the file is
/// dropped from the batch rather than failing the whole pass.
///
/// `files` is a slice of `(absolute_path, relative_path)` pairs. The
/// relative path is what gets stored in bridge_endpoints.file_path so it
/// matches how symbols.file_path is keyed elsewhere.
pub(super) async fn rebuild_bridge_endpoints(
    files: &[(std::path::PathBuf, String)],
    linker: &BridgeLinker,
) -> Vec<BridgeEndpoint> {
    let registry = GrammarRegistry::global();
    let mut endpoints = Vec::new();

    for (abs_path, relative) in files {
        let ext = Path::new(relative.as_str())
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let Some(lang_name) = registry.language_name_for_extension(ext) else {
            continue;
        };
        let content = match tokio::fs::read(abs_path).await {
            Ok(c) => c,
            Err(e) => {
                debug!(path = %relative, error = %e, "bridge rebuild: skipping unreadable file");
                continue;
            }
        };

        let tree = if lang_name == "rust" {
            let mut parser = AstParser::new();
            match parser.parse(&content) {
                Ok(t) => t,
                Err(e) => {
                    debug!(path = %relative, error = %e, "bridge rebuild: rust parse failed");
                    continue;
                }
            }
        } else {
            let Some(ts_lang) = registry.language_for_extension(ext) else {
                continue;
            };
            let Ok(mut parser) = AstParser::with_language(ts_lang) else {
                continue;
            };
            match parser.parse(&content) {
                Ok(t) => t,
                Err(e) => {
                    debug!(path = %relative, lang = %lang_name, error = %e, "bridge rebuild: parse failed");
                    continue;
                }
            }
        };

        let sf = BridgeSourceFile {
            file_path: relative.as_str(),
            language: lang_name,
            tree: &tree,
            source: &content,
        };
        endpoints.extend(linker.extract_all(&[sf]));
    }

    endpoints
}

#[cfg(test)]
mod tests {
    use crate::storage::SqliteStorage;
    use crate::storage::traits::Storage;

    use super::*;

    /// Helper: run the full symbol extraction + ref resolution pipeline on Rust source.
    async fn extract_and_resolve(
        source: &str,
        file_path: &str,
    ) -> (Vec<SymbolRecord>, Vec<SymbolRefRecord>) {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = extract_code_symbols(file_path, source, &storage, None)
            .await
            .unwrap();
        // Result's pending_refs are for targets not yet indexed — we only
        // care about the resolved refs that were stored.
        let _ = result;

        let filter = SymbolFilter::default();
        let symbols = storage.list_symbols(&filter).await.unwrap();

        // Collect all stored refs by querying for each symbol.
        let mut all_refs = Vec::new();
        for sym in &symbols {
            let refs = storage.query_refs(&sym.id, None).await.unwrap();
            all_refs.extend(refs);
        }
        (symbols, all_refs)
    }

    #[tokio::test]
    async fn method_call_ref_resolved_to_function_symbol() {
        let source = r"
struct Config;

impl Config {
    fn validate(&self) -> bool {
        self.check_inner()
    }
    fn check_inner(&self) -> bool {
        true
    }
}
";
        let (symbols, refs) = extract_and_resolve(source, "src/config.rs").await;

        // Both methods should be extracted as symbols.
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "validate" && s.kind == "function"),
            "validate method should be a symbol: {symbols:?}"
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "check_inner" && s.kind == "function"),
            "check_inner method should be a symbol: {symbols:?}"
        );

        // The call from validate → check_inner should be resolved.
        let call_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_kind == RefKind::Calls)
            .collect();
        let validate_id = symbols.iter().find(|s| s.name == "validate").map(|s| &s.id);
        let check_inner_id = symbols
            .iter()
            .find(|s| s.name == "check_inner")
            .map(|s| &s.id);

        assert!(
            call_refs
                .iter()
                .any(|r| Some(&r.from_symbol_id) == validate_id
                    && Some(&r.to_symbol_id) == check_inner_id),
            "should have call ref validate → check_inner.\n\
             call_refs: {call_refs:?}\n\
             validate_id: {validate_id:?}\n\
             check_inner_id: {check_inner_id:?}"
        );
    }

    #[tokio::test]
    async fn free_function_call_ref_resolved() {
        let source = r"
fn main() {
    helper();
}

fn helper() {}
";
        let (symbols, refs) = extract_and_resolve(source, "src/main.rs").await;

        let call_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.ref_kind == RefKind::Calls)
            .collect();
        let main_id = symbols.iter().find(|s| s.name == "main").map(|s| &s.id);
        let helper_id = symbols.iter().find(|s| s.name == "helper").map(|s| &s.id);

        assert!(
            call_refs
                .iter()
                .any(|r| Some(&r.from_symbol_id) == main_id && Some(&r.to_symbol_id) == helper_id),
            "should have call ref main → helper.\n\
             call_refs: {call_refs:?}\n\
             main_id: {main_id:?}\n\
             helper_id: {helper_id:?}"
        );
    }

    #[tokio::test]
    async fn from_context_fallback_to_file_anchor() {
        // When from_context doesn't match any local symbol (e.g., a closure),
        // the ref should fall back to the file anchor instead of being dropped.
        let source = r"
fn run() {
    unknown_function();
}
";
        let (symbols, refs) = extract_and_resolve(source, "src/lib.rs").await;

        // Even if `unknown_function` can't resolve its target, the extraction
        // should at least not panic. The `run` function itself should be indexed.
        assert!(
            symbols.iter().any(|s| s.name == "run"),
            "run function should be indexed"
        );
        // The ref from run → unknown_function should be pending (unresolved
        // target), not silently dropped.
        let _ = refs; // No assertion on refs — target doesn't exist in this file.
    }

    // -- rebuild_bridge_endpoints: full-corpus bridge rebuild path --

    /// Regression: bridge data is a global view derived from current
    /// extractor logic, not per-file content. `rebuild_bridge_endpoints`
    /// must re-read and re-parse every file regardless of content-hash
    /// cache, so extractor changes propagate to unchanged files.
    #[cfg(feature = "lang-typescript")]
    #[tokio::test]
    async fn rebuild_extracts_from_all_tracked_files() {
        use crate::code::bridge::tauri::TauriCommandExtractor;
        use std::fs;
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let root = tmp.path();

        // Rust export side: #[tauri::command] functions.
        let rust_path = root.join("commands.rs");
        fs::write(
            &rust_path,
            "#[tauri::command]\n\
             fn greet(name: &str) -> String { String::new() }\n\
             #[tauri::command]\n\
             fn farewell() -> String { String::new() }\n",
        )
        .unwrap();

        // TSX import side (the path the tsx grammar fix unblocked).
        let tsx_path = root.join("App.tsx");
        fs::write(
            &tsx_path,
            "import { invoke } from '@tauri-apps/api/core';\n\
             export function App() {\n  \
               return invoke('greet', { name: 'world' });\n\
             }\n",
        )
        .unwrap();

        // Plain TS import side.
        let ts_path = root.join("hook.ts");
        fs::write(
            &ts_path,
            "import { invoke } from '@tauri-apps/api/core';\n\
             export async function load() {\n  \
               await invoke('farewell');\n\
             }\n",
        )
        .unwrap();

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));

        let files = vec![
            (rust_path, "commands.rs".to_string()),
            (tsx_path, "App.tsx".to_string()),
            (ts_path, "hook.ts".to_string()),
        ];

        let endpoints = rebuild_bridge_endpoints(&files, &linker).await;

        // Rust side contributes 2 exports (greet, farewell).
        // TSX side contributes 1 import (greet).
        // TS side contributes 1 import (farewell).
        assert_eq!(
            endpoints.len(),
            4,
            "expected 2 rust exports + 1 tsx import + 1 ts import = 4 endpoints, got {endpoints:#?}"
        );
        let keys: Vec<&str> = endpoints.iter().map(|e| e.binding_key.as_str()).collect();
        assert!(keys.contains(&"greet"));
        assert!(keys.contains(&"farewell"));
        assert!(
            endpoints.iter().any(|e| e.language == "tsx"),
            "rebuild must pick up tsx-language endpoints"
        );
    }

    /// Unknown extensions and missing files don't crash the rebuild.
    #[tokio::test]
    async fn rebuild_tolerates_non_code_files_and_missing_paths() {
        use crate::code::bridge::tauri::TauriCommandExtractor;
        use std::fs;
        use tempfile::tempdir;

        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let readme = root.join("README.md");
        fs::write(&readme, "# docs only\nno bridges here").unwrap();

        let missing = root.join("never_created.rs");

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(TauriCommandExtractor));

        let files = vec![
            (readme, "README.md".to_string()),
            (missing, "never_created.rs".to_string()),
        ];

        let endpoints = rebuild_bridge_endpoints(&files, &linker).await;
        assert!(endpoints.is_empty(), "no code → no bridge endpoints");
    }

    #[test]
    fn languages_compatible_matches_within_language_family() {
        // Same language: compatible.
        assert!(languages_compatible("src/a.rs", "src/b.rs"));
        assert!(languages_compatible("src/a.ts", "src/b.ts"));
        assert!(languages_compatible("src/A.tsx", "src/B.tsx"));
        // Different languages: incompatible — this is the case that
        // produced the `core::bundle::append_bytes → app::Header`
        // phantom binding before this fix.
        assert!(!languages_compatible(
            "ministr-core/src/bundle.rs",
            "ministr-app/src/components/Header.tsx"
        ));
        // The JS/TS ecosystem is ONE ref-resolution family even though
        // the grammar registry keeps `.tsx` (own grammar), `.ts`
        // (`typescript`) and `.js`/`.jsx` (`javascript`) as distinct
        // canonical names. These share one ES module system and
        // routinely cross-import (a `.tsx` component importing `cn` from
        // a `.ts` util is the canonical case), so they must be
        // compatible — keeping them separate silently dropped every
        // `.tsx`→`.ts` cross-file reference.
        assert!(languages_compatible("a.tsx", "b.ts"));
        assert!(languages_compatible("a.ts", "b.tsx"));
        assert!(languages_compatible("a.tsx", "b.js"));
        assert!(languages_compatible("a.jsx", "b.ts"));
        assert!(languages_compatible("a.mts", "b.tsx"));
        // …but the family is closed: JS/TS still cannot bind to Rust.
        assert!(!languages_compatible("a.tsx", "b.rs"));
        // Unknown extensions: both sides must resolve → reject (safer
        // than wide-open match).
        assert!(!languages_compatible("README.md", "src/a.rs"));
        assert!(!languages_compatible("a.weird", "b.alsoweird"));
    }

    /// Regression: a Rust ref must not bind to a same-named symbol in
    /// a different language. Pre-fix, an unresolved `header` ref in a
    /// `.rs` file could phantom-resolve to a `Header` symbol in a
    /// `.tsx` file via the global `name_exact` lookup.
    #[tokio::test]
    async fn resolver_rejects_cross_language_phantom_binding() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        // Seed a TSX-only Header symbol so any cross-language phantom
        // would have somewhere to land.
        let tsx_header = SymbolRecord {
            id: SymbolId("sym-app/src/components/Header.tsx::Header".into()),
            file_path: "app/src/components/Header.tsx".into(),
            name: "Header".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "function Header()".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[tsx_header]).await.unwrap();

        // Now extract refs from a Rust file that uses a type named
        // `Header`. With the same-language filter active, this ref
        // should resolve to *no target* (and be queued as pending or
        // dropped), never to the TSX symbol.
        let source = r"
pub struct Bundle;

impl Bundle {
    pub fn build(&self) -> Header {
        Header
    }
}
";
        let _ = extract_code_symbols("core/src/bundle.rs", source, &storage, None)
            .await
            .unwrap();

        // Inspect every stored ref. None should target the TSX Header.
        let all_symbols = storage
            .list_symbols(&SymbolFilter::default())
            .await
            .unwrap();
        let mut cross_lang_phantom_count = 0usize;
        for sym in &all_symbols {
            let refs = storage.query_refs(&sym.id, None).await.unwrap();
            for r in refs {
                if r.to_symbol_id.0.contains(".tsx") {
                    cross_lang_phantom_count += 1;
                }
            }
        }
        assert_eq!(
            cross_lang_phantom_count, 0,
            "no Rust ref should bind to a TSX symbol; resolver phantom-bound across languages"
        );
    }
}
