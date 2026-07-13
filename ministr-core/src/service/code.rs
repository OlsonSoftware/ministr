//! Code intelligence operations for [`QueryService`].
//!
//! Symbol search, definition lookup, cross-reference queries, bridge link
//! queries, and source file resolution helpers.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tracing::instrument;

use crate::storage::{BridgeLinkDetail, Storage, SymbolFilter, SymbolRecord};
use crate::types::{Pagination, RefKind, ResultLocator, RootKind, SymbolId};

use super::{
    CallDirection, DeadSymbol, DefinitionChild, DefinitionContinuation, DefinitionOptions,
    DiffChangeAuthor, DiffChangedSymbol, DiffImpactResult, ImpactCaller, ImpactResult, ImpactRisk,
    InspectImpactSummary, InspectInclude, InspectNextAction, InspectOptions, InspectPartialError,
    InspectReferenceGroup, InspectResult, QueryError, QueryService, SourceLineRange,
    SymbolDefinition, SymbolRefResult,
};

#[derive(Debug)]
struct SourceSlice {
    text: String,
    truncated: bool,
    omitted_lines: usize,
    original: SourceLineRange,
    returned: SourceLineRange,
    continuation: Option<DefinitionContinuation>,
    source_error: Option<String>,
}

fn bounded_reference_group(
    mut items: Vec<SymbolRefResult>,
    max_per_group: usize,
) -> InspectReferenceGroup {
    shape_reference_results(&mut items);
    let total = items.len();
    items.truncate(max_per_group);
    let returned = items.len();
    let has_more = returned < total;
    let next_cursor = has_more
        .then(|| items.last().map(symbol_reference_cursor))
        .flatten();
    InspectReferenceGroup {
        items,
        total,
        omitted_count: total.saturating_sub(returned),
        pagination: Pagination {
            limit: max_per_group,
            offset: Some(0),
            cursor: None,
            next_cursor,
            total,
            has_more,
            omitted_count: total.saturating_sub(returned),
        },
    }
}

const MAX_INSPECT_RESPONSE_BYTES: usize = 65_536;
const MAX_DEFINITION_SOURCE_BYTES: usize = 32_768;

/// Stable opaque cursor for a reference edge.
#[must_use]
pub fn symbol_reference_cursor(reference: &SymbolRefResult) -> String {
    let identity = serde_json::json!([
        reference.from_symbol_id,
        reference.from_file,
        reference.from_line,
        reference.to_symbol_id,
        reference.to_file,
        reference.to_line,
        reference.ref_kind,
    ]);
    let encoded = serde_json::to_vec(&identity).unwrap_or_default();
    format!("ref:{}", blake3::hash(&encoded).to_hex())
}

fn refresh_inspect_group(group: &mut InspectReferenceGroup) {
    let returned = group.items.len();
    group.omitted_count = group.total.saturating_sub(returned);
    group.pagination.total = group.total;
    group.pagination.has_more = returned < group.total;
    group.pagination.omitted_count = group.omitted_count;
    group.pagination.next_cursor = group
        .pagination
        .has_more
        .then(|| group.items.last().map(symbol_reference_cursor))
        .flatten();
}

fn serialized_inspect_bytes(result: &InspectResult) -> usize {
    serde_json::to_vec(result).map_or(usize::MAX, |value| value.len())
}

fn truncate_unicode(value: &mut String, max_bytes: usize) -> bool {
    if value.len() <= max_bytes {
        return false;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    true
}

fn clip_definition_for_inspect(definition: &mut SymbolDefinition) -> bool {
    if definition.source_context.len() <= 1_024 {
        return false;
    }
    let mut target = definition.source_context.len() * 3 / 4;
    while target > 0 && !definition.source_context.is_char_boundary(target) {
        target -= 1;
    }
    let boundary = definition.source_context[..target]
        .rfind('\n')
        .unwrap_or(target);
    if boundary == 0 {
        return false;
    }
    definition.source_context.truncate(boundary);
    let returned_lines = definition.source_context.lines().count().max(1);
    let start = definition.returned_line_range.start;
    let end = start
        .saturating_add(u32::try_from(returned_lines).unwrap_or(u32::MAX))
        .saturating_sub(1)
        .min(definition.original_line_range.end);
    definition.returned_line_range.end = end;
    let original_lines = usize::try_from(
        definition
            .original_line_range
            .end
            .saturating_sub(definition.original_line_range.start)
            .saturating_add(1),
    )
    .unwrap_or(usize::MAX);
    definition.omitted_lines = original_lines.saturating_sub(returned_lines);
    definition.truncated = true;
    definition.continuation =
        (end < definition.original_line_range.end).then(|| DefinitionContinuation {
            symbol_id: definition.id.clone(),
            start_line: end.saturating_add(1),
            max_lines: returned_lines,
            start_byte: None,
            max_bytes: None,
        });
    true
}

fn bound_inspect_response(mut result: InspectResult) -> InspectResult {
    // Measure the unbounded wire representation with its size metadata included.
    // The fixed point normally converges in two iterations (only the decimal digit
    // count can change), while the guard keeps pathological serializers harmless.
    for _ in 0..8 {
        let bytes = serialized_inspect_bytes(&result);
        if result.original_bytes == bytes && result.returned_bytes == bytes {
            break;
        }
        result.original_bytes = bytes;
        result.returned_bytes = bytes;
    }
    while serialized_inspect_bytes(&result) > MAX_INSPECT_RESPONSE_BYTES.saturating_sub(128) {
        let lengths = [
            result.callers.items.len(),
            result.callees.items.len(),
            result.implementors.items.len(),
            result.imports.items.len(),
            result.tests.items.len(),
            result.bridges.items.len(),
        ];
        if let Some((largest, _)) = lengths
            .iter()
            .enumerate()
            .filter(|(_, length)| **length > 0)
            .max_by_key(|(_, length)| **length)
        {
            let group = match largest {
                0 => &mut result.callers,
                1 => &mut result.callees,
                2 => &mut result.implementors,
                3 => &mut result.imports,
                4 => &mut result.tests,
                _ => &mut result.bridges,
            };
            // Trim geometrically so adversarially large groups cannot turn
            // response shaping into an O(n²) serialization loop.
            let keep = group.items.len() / 2;
            group.items.truncate(keep);
            refresh_inspect_group(group);
            result.truncated = true;
            continue;
        }
        if result
            .definition
            .as_mut()
            .is_some_and(clip_definition_for_inspect)
        {
            result.truncated = true;
            continue;
        }
        if let Some(definition) = result.definition.as_mut() {
            let mut clipped = truncate_unicode(&mut definition.signature, 2_048);
            if let Some(doc) = definition.doc_comment.as_mut() {
                clipped |= truncate_unicode(doc, 2_048);
            }
            for heading in &mut definition.heading_path {
                clipped |= truncate_unicode(heading, 512);
            }
            if definition.child_symbols.len() > 50 {
                definition.child_symbols.truncate(50);
                clipped = true;
            }
            if clipped {
                result.truncated = true;
                continue;
            }
        }
        break;
    }
    for _ in 0..8 {
        let bytes = serialized_inspect_bytes(&result);
        if result.returned_bytes == bytes {
            break;
        }
        result.returned_bytes = bytes;
    }
    result
}

fn is_large_container(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "impl" | "class" | "module" | "mod" | "trait" | "interface"
    )
}

fn bridge_mentions_symbol(link: &BridgeLinkDetail, symbol: &SymbolRecord) -> bool {
    let id_matches = link.export_symbol_id.as_deref() == Some(symbol.id.0.as_str())
        || link.import_symbol_id.as_deref() == Some(symbol.id.0.as_str());
    let name_matches = link.export_symbol.eq_ignore_ascii_case(&symbol.name)
        || link.import_symbol.eq_ignore_ascii_case(&symbol.name);
    let export_span_matches = link.export_file == symbol.file_path
        && (symbol.line_start..=symbol.line_end).contains(&link.export_line);
    let import_span_matches = link.import_file == symbol.file_path
        && (symbol.line_start..=symbol.line_end).contains(&link.import_line);
    id_matches || name_matches || export_span_matches || import_span_matches
}

fn shape_reference_results(results: &mut Vec<SymbolRefResult>) {
    let mut seen = HashSet::new();
    results.retain(|result| {
        seen.insert((
            result.from_symbol_id.clone(),
            result.from_file.clone(),
            result.from_line,
            result.to_symbol_id.clone(),
            result.to_file.clone(),
            result.to_line,
            result.ref_kind.clone(),
        ))
    });
    let kind_rank = |kind: &str| match kind {
        "calls" => 0,
        "implements" => 1,
        "uses" => 2,
        "imports" => 3,
        "bridge" => 4,
        _ => 5,
    };
    results.sort_by(|a, b| {
        let a_test = is_test_path(&a.from_file);
        let b_test = is_test_path(&b.from_file);
        a_test
            .cmp(&b_test)
            .then_with(|| kind_rank(&a.ref_kind).cmp(&kind_rank(&b.ref_kind)))
            .then_with(|| a.from_file.cmp(&b.from_file))
            .then_with(|| a.from_line.cmp(&b.from_line))
    });
}

impl QueryService {
    /// Search the symbol index with optional filters.
    ///
    /// Returns symbols matching the given filter criteria. All filter fields
    /// are optional — omitting all fields returns all symbols.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn search_symbols(
        &self,
        filter: &SymbolFilter,
    ) -> Result<Vec<SymbolRecord>, QueryError> {
        Ok(self.storage.list_symbols(filter).await?)
    }

    /// Resolve a file position to the symbol id of the identifier under it.
    ///
    /// Maps a 1-based `line` / 0-based byte `col` through the occurrence index
    /// (FL1) to the resolved `symbol_id` — the position→symbol bridge that lets
    /// `ministr_definition`/`ministr_references` be position-addressable
    /// (FL2-equivalent of LSP `textDocument/definition`). Returns `None` when
    /// no occurrence covers the position (cursor on whitespace/punctuation, or
    /// the corpus was indexed without the occurrence index).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn symbol_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Option<String>, QueryError> {
        let occurrences = self.storage.list_occurrences(file_path).await?;
        Ok(
            crate::storage::traits::occurrence_at(&occurrences, line, col)
                .map(|o| o.symbol_id.0.clone()),
        )
    }

    /// Get the full definition of a symbol with surrounding source context.
    ///
    /// Returns the symbol metadata plus the source code lines covering
    /// the symbol with 3 lines of surrounding context, and a heading path
    /// showing the module hierarchy.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if no symbol with the given ID
    /// exists, or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn get_symbol_definition(
        &self,
        symbol_id: &str,
    ) -> Result<SymbolDefinition, QueryError> {
        self.get_symbol_definition_with_options(symbol_id, DefinitionOptions::default())
            .await
    }

    /// Get a definition using explicit source-body, outline, and range bounds.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if no symbol with the given ID
    /// exists, or [`QueryError::Storage`] on database errors.
    #[allow(clippy::too_many_lines)] // one bounded-definition assembly path keeps range metadata coherent
    #[instrument(skip(self))]
    pub async fn get_symbol_definition_with_options(
        &self,
        symbol_id: &str,
        mut options: DefinitionOptions,
    ) -> Result<SymbolDefinition, QueryError> {
        options.max_lines = options.max_lines.clamp(1, 1_000);
        options.context_lines = options.context_lines.min(32);
        let sid = SymbolId(symbol_id.to_string());
        let symbol =
            self.storage
                .get_symbol(&sid)
                .await?
                .ok_or_else(|| QueryError::SymbolNotFound {
                    id: symbol_id.to_string(),
                })?;

        // Build heading path from module path + symbol name
        let mut heading_path: Vec<String> = symbol
            .module_path
            .split("::")
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        heading_path.push(symbol.name.clone());

        let symbol_lines = symbol
            .line_end
            .saturating_sub(symbol.line_start)
            .saturating_add(1) as usize;
        let auto_outline = options.start_line.is_none()
            && symbol_lines > options.max_lines
            && is_large_container(&symbol.kind);
        let outline_only = options.outline_only || !options.include_body || auto_outline;
        let child_symbols = if outline_only || auto_outline {
            self.definition_children(&symbol, 50).await?
        } else {
            Vec::new()
        };
        let source = if outline_only {
            let original = SourceLineRange {
                start: symbol.line_start,
                end: symbol.line_end,
            };
            let returned = SourceLineRange {
                start: symbol.line_start,
                end: symbol.line_start,
            };
            SourceSlice {
                text: symbol.signature.clone(),
                truncated: symbol.line_end > symbol.line_start,
                omitted_lines: symbol_lines.saturating_sub(1),
                original,
                returned,
                continuation: (symbol.line_end > symbol.line_start).then(|| {
                    DefinitionContinuation {
                        symbol_id: symbol.id.0.clone(),
                        start_line: symbol.line_start,
                        max_lines: options.max_lines,
                        start_byte: None,
                        max_bytes: None,
                    }
                }),
                source_error: None,
            }
        } else {
            self.read_source_slice(
                &symbol.file_path,
                symbol.line_start,
                symbol.line_end,
                symbol.id.0.as_str(),
                options,
            )
            .await?
        };

        let delivery_resolution = if outline_only {
            "symbol_outline".to_string()
        } else if source.truncated {
            format!(
                "symbol_slice:{}:{}:{}",
                source.returned.start,
                source.returned.end,
                options.start_byte.unwrap_or(0)
            )
        } else {
            "symbol_full".to_string()
        };
        Ok(SymbolDefinition {
            id: symbol.id.0.clone(),
            name: symbol.name,
            kind: symbol.kind,
            visibility: symbol.visibility,
            signature: symbol.signature,
            doc_comment: symbol.doc_comment,
            file_path: symbol.file_path,
            line_start: symbol.line_start,
            line_end: symbol.line_end,
            heading_path,
            source_context: source.text,
            truncated: source.truncated,
            omitted_lines: source.omitted_lines,
            original_line_range: source.original,
            returned_line_range: source.returned,
            continuation: source.continuation,
            outline_only,
            child_symbols,
            locator: ResultLocator::primary(symbol_id, delivery_resolution),
            source_error: source.source_error,
        })
    }

    async fn definition_children(
        &self,
        parent: &SymbolRecord,
        limit: usize,
    ) -> Result<Vec<DefinitionChild>, QueryError> {
        let filter = SymbolFilter {
            file_path: Some(parent.file_path.clone()),
            ..SymbolFilter::default()
        };
        let mut children: Vec<SymbolRecord> = self
            .storage
            .list_symbols(&filter)
            .await?
            .into_iter()
            .filter(|child| {
                child.id != parent.id
                    && child.line_start >= parent.line_start
                    && child.line_end <= parent.line_end
            })
            .collect();
        children.sort_by(|a, b| {
            a.line_start
                .cmp(&b.line_start)
                .then_with(|| a.line_end.cmp(&b.line_end))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        children.truncate(limit);
        Ok(children
            .into_iter()
            .map(|child| DefinitionChild {
                locator: ResultLocator::primary(child.id.0.clone(), "symbol_stub"),
                id: child.id.0,
                name: child.name,
                kind: child.kind,
                file_path: child.file_path,
                line_start: child.line_start,
                line_end: child.line_end,
            })
            .collect())
    }

    /// Get all references for a symbol, optionally filtered by reference kind.
    ///
    /// Returns cross-references where the given symbol is the target (i.e.
    /// callers, implementors, importers of the symbol).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if the symbol does not exist,
    /// or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn get_symbol_references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> Result<Vec<SymbolRefResult>, QueryError> {
        let sid = SymbolId(symbol_id.to_string());

        // Verify symbol exists and get its file path for bridge queries
        let symbol =
            self.storage
                .get_symbol(&sid)
                .await?
                .ok_or_else(|| QueryError::SymbolNotFound {
                    id: symbol_id.to_string(),
                })?;

        let mut results = Vec::new();

        // Include standard symbol refs unless we're filtering to bridge-only
        if ref_kind != Some(RefKind::Bridge) {
            let refs = self.storage.query_refs(&sid, ref_kind).await?;
            let symbol_ids: Vec<SymbolId> = refs
                .iter()
                .flat_map(|reference| {
                    [
                        reference.from_symbol_id.clone(),
                        reference.to_symbol_id.clone(),
                    ]
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            let symbols: HashMap<SymbolId, SymbolRecord> = self
                .storage
                .get_symbols(&symbol_ids)
                .await?
                .into_iter()
                .map(|symbol| (symbol.id.clone(), symbol))
                .collect();
            for r in refs {
                if let (Some(from_sym), Some(to_sym)) =
                    (symbols.get(&r.from_symbol_id), symbols.get(&r.to_symbol_id))
                {
                    results.push(SymbolRefResult {
                        from_symbol_id: from_sym.id.0.clone(),
                        from_name: from_sym.name.clone(),
                        from_file: from_sym.file_path.clone(),
                        from_line: from_sym.line_start,
                        to_symbol_id: to_sym.id.0.clone(),
                        to_name: to_sym.name.clone(),
                        to_file: to_sym.file_path.clone(),
                        to_line: to_sym.line_start,
                        ref_kind: r.ref_kind.to_string(),
                    });
                }
            }
        }

        // Include bridge links when ref_kind is None or Bridge
        if ref_kind.is_none() || ref_kind == Some(RefKind::Bridge) {
            let bridge_links = self
                .storage
                .query_bridge_links(Some(&symbol.file_path), None)
                .await?;

            for link in bridge_links
                .into_iter()
                .filter(|link| bridge_mentions_symbol(link, &symbol))
            {
                // Map bridge links to SymbolRefResult: export → from, import → to
                results.push(SymbolRefResult {
                    from_symbol_id: String::new(),
                    from_name: link.export_symbol,
                    from_file: link.export_file,
                    from_line: link.export_line,
                    to_symbol_id: String::new(),
                    to_name: link.import_symbol,
                    to_file: link.import_file,
                    to_line: link.import_line,
                    ref_kind: "bridge".to_string(),
                });
            }
        }

        shape_reference_results(&mut results);
        Ok(results)
    }

    /// Inspect a symbol and its direct navigation neighbourhood in one bounded call.
    ///
    /// The method deliberately reuses definition and reference operations so
    /// grouping semantics cannot drift from the granular tools.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] when the target is absent.
    #[allow(clippy::similar_names, clippy::too_many_lines)]
    #[instrument(skip(self, options))]
    pub async fn inspect_symbol(
        &self,
        symbol_id: &str,
        options: &InspectOptions,
    ) -> Result<InspectResult, QueryError> {
        let includes: HashSet<InspectInclude> = options.include.iter().copied().collect();
        let max_per_group = options.max_per_group.clamp(1, 50);
        let locator = ResultLocator::primary(symbol_id, "symbol_full");

        // Verify the target even when the caller omitted the definition group.
        if self
            .storage
            .get_symbol(&SymbolId(symbol_id.to_string()))
            .await?
            .is_none()
        {
            return Err(QueryError::SymbolNotFound {
                id: symbol_id.to_string(),
            });
        }

        let definition = if includes.contains(&InspectInclude::Definition) {
            Some(
                self.get_symbol_definition_with_options(
                    symbol_id,
                    DefinitionOptions {
                        max_lines: options.max_source_lines.clamp(1, 1_000),
                        ..DefinitionOptions::default()
                    },
                )
                .await?,
            )
        } else {
            None
        };

        let mut partial_errors = Vec::new();
        #[cfg(test)]
        let references_result = if let Some(message) = &self.inspect_reference_failure {
            Err(QueryError::Storage(crate::error::StorageError::Database {
                reason: message.clone(),
            }))
        } else {
            self.get_symbol_references(symbol_id, None).await
        };
        #[cfg(not(test))]
        let references_result = self.get_symbol_references(symbol_id, None).await;
        let references = match references_result {
            Ok(references) => references,
            Err(error) => {
                partial_errors.push(InspectPartialError {
                    group: "references".to_string(),
                    message: error.to_string(),
                });
                Vec::new()
            }
        };

        let select = |include: InspectInclude, predicate: &dyn Fn(&SymbolRefResult) -> bool| {
            if includes.contains(&include) {
                references
                    .iter()
                    .filter(|reference| predicate(reference))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            }
        };

        let callers_all = select(InspectInclude::Callers, &|reference| {
            reference.ref_kind == "calls" && reference.to_symbol_id == symbol_id
        });
        let callees_all = select(InspectInclude::Callees, &|reference| {
            reference.ref_kind == "calls" && reference.from_symbol_id == symbol_id
        });
        let implementors_all = select(InspectInclude::Implementors, &|reference| {
            reference.ref_kind == "implements"
        });
        let imports_all = select(InspectInclude::Imports, &|reference| {
            matches!(reference.ref_kind.as_str(), "imports" | "uses")
        });
        let tests_all = select(InspectInclude::Tests, &|reference| {
            is_test_path(&reference.from_file)
        });
        let bridges_all = select(InspectInclude::Bridges, &|reference| {
            reference.ref_kind == "bridge"
        });

        let direct_callers = callers_all.len();
        let direct_callees = callees_all.len();
        let relevant_tests = tests_all.len();
        let impact_edges: Vec<&SymbolRefResult> = callers_all
            .iter()
            .chain(&callees_all)
            .chain(&implementors_all)
            .chain(&imports_all)
            .chain(&tests_all)
            .chain(&bridges_all)
            .collect();
        let unique_relationships: HashSet<String> = impact_edges
            .iter()
            .map(|reference| symbol_reference_cursor(reference))
            .collect();
        let affected_files: HashSet<&str> = impact_edges
            .iter()
            .flat_map(|reference| [reference.from_file.as_str(), reference.to_file.as_str()])
            .filter(|file| !file.is_empty())
            .collect();
        let impact = InspectImpactSummary {
            direct_callers,
            direct_callees,
            affected_files: affected_files.len(),
            relevant_tests,
            risk: compute_risk(
                unique_relationships.len(),
                affected_files.len(),
                relevant_tests,
            ),
        };

        let callers = bounded_reference_group(callers_all, max_per_group);
        let callees = bounded_reference_group(callees_all, max_per_group);
        let implementors = bounded_reference_group(implementors_all, max_per_group);
        let imports = bounded_reference_group(imports_all, max_per_group);
        let tests = bounded_reference_group(tests_all, max_per_group);
        let bridges = bounded_reference_group(bridges_all, max_per_group);

        let omitted = callers.omitted_count
            + callees.omitted_count
            + implementors.omitted_count
            + imports.omitted_count
            + tests.omitted_count
            + bridges.omitted_count;
        let mut next_actions = Vec::new();
        if omitted > 0 {
            next_actions.push(InspectNextAction {
                action: "ministr_references".to_string(),
                locator: locator.clone(),
                reason: format!("{omitted} direct relationships were omitted by group bounds"),
            });
        }
        if definition.as_ref().is_some_and(|item| item.truncated) {
            next_actions.push(InspectNextAction {
                action: "ministr_definition".to_string(),
                locator: locator.clone(),
                reason: "definition source is truncated; follow its continuation range".to_string(),
            });
        }

        Ok(bound_inspect_response(InspectResult {
            symbol_id: symbol_id.to_string(),
            locator,
            definition,
            callers,
            callees,
            implementors,
            imports,
            tests,
            bridges,
            impact,
            partial_errors,
            next_actions,
            truncated: false,
            original_bytes: 0,
            returned_bytes: 0,
        }))
    }

    /// Position-addressed variant of [`Self::inspect_symbol`].
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] when no occurrence covers the
    /// supplied position.
    pub async fn inspect_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
        options: &InspectOptions,
    ) -> Result<InspectResult, QueryError> {
        let symbol_id = self
            .symbol_at_position(file_path, line, col)
            .await?
            .ok_or_else(|| QueryError::SymbolNotFound {
                id: format!("{file_path}:{line}:{col}"),
            })?;
        self.inspect_symbol(&symbol_id, options).await
    }

    /// Type-hierarchy-aware references (FL3b): the normal references of
    /// `symbol_id`, PLUS — when it is a method on a trait/interface-related
    /// type — the callers of the same-named method on every co-implementor.
    ///
    /// ministr's `Implements` graph is *type-level* (`class implements trait`,
    /// not `method overrides method`), so this approximates LSP "find
    /// references including overrides" with a bounded, name-based heuristic:
    ///
    /// 1. Split `symbol_id` (`…::Type::method`) into its container type + name.
    /// 2. From the container's (bidirectional) `Implements` edges, gather
    ///    *peer types*: the container's own implementors (if it is a trait) and
    ///    the co-implementors of every trait the container implements.
    /// 3. For each peer type `P`, if `P::method` exists, append its callers.
    ///
    /// Bounded by `max_implementors` peer methods; a single `Implements` hop +
    /// direct callers only, never a transitive / full-graph walk. With no peers
    /// (free function, non-trait type, or no same-named method) the result is
    /// exactly [`Self::get_symbol_references`].
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if `symbol_id` does not exist, or
    /// [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn get_symbol_references_through_implementors(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        max_implementors: usize,
    ) -> Result<Vec<SymbolRefResult>, QueryError> {
        let mut results = self.get_symbol_references(symbol_id, ref_kind).await?;

        // The peer hop surfaces *callers* (Calls edges), so it is meaningful
        // only when callers are in scope; for a non-call `ref_kind` filter this
        // is a no-op (keeps the Local + daemon code paths consistent).
        if !matches!(ref_kind, None | Some(RefKind::Calls)) {
            return Ok(results);
        }

        // `…::Container::method` → (container_id, method_name). A free function
        // or top-level symbol has no usable container hop.
        let Some((container_id, method_name)) = symbol_id.rsplit_once("::") else {
            return Ok(results);
        };
        let container = SymbolId(container_id.to_string());

        // Peer implementor TYPES from the type-level `Implements` graph.
        // `query_refs` is bidirectional, so one query yields both directions:
        // edges INTO the container (it is a trait → implementors) and edges OUT
        // of it (it is a concrete type → the traits it implements).
        let mut peer_types: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::from([container_id.to_string()]);
        for edge in self
            .storage
            .query_refs(&container, Some(RefKind::Implements))
            .await?
        {
            if edge.to_symbol_id.0 == container_id {
                // Something implements the container → the container is a trait.
                let implementor = edge.from_symbol_id.0;
                if seen.insert(implementor.clone()) {
                    peer_types.push(implementor);
                }
            } else if edge.from_symbol_id.0 == container_id {
                // Container implements a trait → gather the trait's other impls.
                let trait_sid = edge.to_symbol_id;
                for t in self
                    .storage
                    .query_refs(&trait_sid, Some(RefKind::Implements))
                    .await?
                {
                    if t.to_symbol_id == trait_sid && seen.insert(t.from_symbol_id.0.clone()) {
                        peer_types.push(t.from_symbol_id.0);
                    }
                }
            }
        }

        // Append callers of each peer type's same-named method, deduped against
        // the base results.
        let mut seen_refs: std::collections::HashSet<(String, String)> = results
            .iter()
            .map(|r| (r.from_symbol_id.clone(), r.to_symbol_id.clone()))
            .collect();
        for peer in peer_types.into_iter().take(max_implementors) {
            let peer_method = SymbolId(format!("{peer}::{method_name}"));
            let Some(method_sym) = self.storage.get_symbol(&peer_method).await? else {
                continue;
            };
            for c in self
                .storage
                .query_refs(&peer_method, Some(RefKind::Calls))
                .await?
            {
                if c.to_symbol_id != peer_method {
                    continue; // incoming callers of the peer method only
                }
                let Some(caller) = self.storage.get_symbol(&c.from_symbol_id).await? else {
                    continue;
                };
                if !seen_refs.insert((caller.id.0.clone(), method_sym.id.0.clone())) {
                    continue;
                }
                results.push(SymbolRefResult {
                    from_symbol_id: caller.id.0,
                    from_name: caller.name,
                    from_file: caller.file_path,
                    from_line: caller.line_start,
                    to_symbol_id: method_sym.id.0.clone(),
                    to_name: method_sym.name.clone(),
                    to_file: method_sym.file_path.clone(),
                    to_line: method_sym.line_start,
                    ref_kind: RefKind::Calls.to_string(),
                });
            }
        }

        Ok(results)
    }

    /// Compute transitive caller counts for a batch of symbols.
    ///
    /// Delegates to storage-level recursive CTE query. Returns a map from
    /// symbol ID to the number of unique transitive callers.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    pub async fn transitive_caller_counts(
        &self,
        symbol_ids: &[SymbolId],
    ) -> Result<std::collections::HashMap<SymbolId, u32>, QueryError> {
        Ok(self.storage.transitive_caller_counts(symbol_ids).await?)
    }

    /// Compute the transitive call hierarchy of a symbol in one direction.
    ///
    /// Walks the `Calls` edge graph from the target up to `max_depth` levels,
    /// depth-bounded and cycle-safe (a `visited` set), collecting distinct
    /// reached nodes, the files they live in, and a heuristic risk score.
    ///
    /// `direction` selects which edge endpoint to follow:
    /// - [`CallDirection::Incoming`] — transitive *callers* (the blast radius:
    ///   who reaches this symbol). This is the historical behavior.
    /// - [`CallDirection::Outgoing`] — transitive *callees* (what this symbol
    ///   reaches). `query_refs` is bidirectional, so only the endpoint differs.
    ///
    /// `tests_only` restricts the returned nodes to those living in test files
    /// (per [`is_test_path`]). Combined with [`CallDirection::Incoming`] this is
    /// the FL6 test↔code mapping — "which tests transitively exercise this
    /// symbol" — powering the minimal-test-set step of the verify loop. (The
    /// inverse, "what a test covers", is [`CallDirection::Outgoing`] on the test
    /// symbol with `tests_only = false`.) Intermediate non-test hops are still
    /// traversed; only the final node set is filtered.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SymbolNotFound`] if the target does not exist,
    /// or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn compute_impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> Result<ImpactResult, QueryError> {
        let sid = SymbolId(symbol_id.to_string());

        self.storage
            .get_symbol(&sid)
            .await?
            .ok_or_else(|| QueryError::SymbolNotFound {
                id: symbol_id.to_string(),
            })?;

        let depth_cap = max_depth.clamp(1, 10);
        let mut visited: std::collections::HashSet<SymbolId> = std::collections::HashSet::new();
        visited.insert(sid.clone());
        let mut callers: Vec<ImpactCaller> = Vec::new();
        let mut frontier: Vec<SymbolId> = vec![sid];

        for depth in 1..=depth_cap {
            let mut next: Vec<SymbolId> = Vec::new();
            for target in &frontier {
                let refs = self
                    .storage
                    .query_refs(target, Some(RefKind::Calls))
                    .await?;
                for r in refs {
                    // `query_refs` returns edges touching `target` on EITHER
                    // side, so pick the neighbor by orientation: incoming
                    // follows edges that point INTO target (collect the
                    // caller); outgoing follows edges that leave target
                    // (collect the callee). Edges on the wrong side are
                    // skipped, not mis-attributed.
                    let neighbor = match direction {
                        CallDirection::Incoming if r.to_symbol_id == *target => r.from_symbol_id,
                        CallDirection::Outgoing if r.from_symbol_id == *target => r.to_symbol_id,
                        _ => continue,
                    };
                    if visited.insert(neighbor.clone())
                        && let Some(sym) = self.storage.get_symbol(&neighbor).await?
                    {
                        callers.push(ImpactCaller {
                            symbol_id: sym.id.0.clone(),
                            name: sym.name,
                            kind: sym.kind,
                            file: sym.file_path,
                            line: sym.line_start,
                            depth,
                        });
                        next.push(neighbor);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        // FL6 — test↔code mapping: keep only nodes in test files. The walk
        // still traverses non-test intermediaries above; only the answer set
        // (the tests that transitively reach the target) is filtered.
        if tests_only {
            callers.retain(|c| is_test_path(&c.file));
        }

        callers.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.name.cmp(&b.name))
        });

        let mut distinct_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut test_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &callers {
            distinct_files.insert(c.file.clone());
            if is_test_path(&c.file) {
                test_files.insert(c.file.clone());
            }
        }

        let risk = compute_risk(callers.len(), distinct_files.len(), test_files.len());

        Ok(ImpactResult {
            target_symbol_id: symbol_id.to_string(),
            direction,
            depth: depth_cap,
            symbols: callers.len(),
            files: distinct_files.len(),
            tests: test_files.len(),
            risk,
            callers,
        })
    }

    /// Map a parsed diff's changed lines to the enclosing indexed symbols (the
    /// seed set), attaching per-symbol git blame. Returns the bounded seeds plus
    /// the count of files that yielded a symbol.
    async fn diff_seed_set(
        &self,
        changed: &[crate::git::ChangedFile],
        dir_roots: &[(std::path::PathBuf, String)],
        max_seeds: usize,
    ) -> Result<(Vec<DiffChangedSymbol>, usize), QueryError> {
        let mut seeds: Vec<DiffChangedSymbol> = Vec::new();
        let mut seed_ids: HashSet<String> = HashSet::new();
        let mut changed_files = 0usize;
        for file in changed {
            // `file.path` is git-toplevel-ABSOLUTE; the stored symbol key is
            // relative/namespaced (or, for pre-decouple corpora, absolute).
            // Try each candidate key until one resolves to symbols.
            let mut symbols = Vec::new();
            for key in crate::ingestion::symbol_key_candidates(&file.path, dir_roots) {
                let filter = SymbolFilter {
                    file_path: Some(key),
                    ..SymbolFilter::default()
                };
                symbols = self.storage.list_symbols(&filter).await?;
                if !symbols.is_empty() {
                    break;
                }
            }
            let mut file_hit = false;
            for s in symbols {
                let touched = file
                    .ranges
                    .iter()
                    .any(|r| r.overlaps(s.line_start, s.line_end));
                if touched && seed_ids.insert(s.id.0.clone()) {
                    file_hit = true;
                    // Reconstruct the absolute path for git blame from the
                    // stored (possibly relative/namespaced) key.
                    let abs = self.resolve_source_path(&s.file_path).await;
                    let blame =
                        crate::git::blame::blame_range(Path::new(&abs), s.line_start, s.line_end)
                            .ok()
                            .flatten();
                    let (authors, last_author) = blame.map_or((Vec::new(), None), |b| {
                        (
                            b.authors
                                .into_iter()
                                .take(4)
                                .map(|a| DiffChangeAuthor {
                                    name: a.name,
                                    lines: a.lines,
                                })
                                .collect(),
                            b.last_author,
                        )
                    });
                    seeds.push(DiffChangedSymbol {
                        symbol_id: s.id.0,
                        name: s.name,
                        kind: s.kind,
                        file: s.file_path,
                        line: s.line_start,
                        authors,
                        last_author,
                    });
                }
            }
            if file_hit {
                changed_files += 1;
            }
            if seeds.len() >= max_seeds {
                break;
            }
        }
        seeds.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        seeds.truncate(max_seeds);
        Ok((seeds, changed_files))
    }

    /// Diff-aware blast radius (FL7): resolve a `range` (e.g. `main..HEAD`) to
    /// the indexed symbols it touched (the seed set, with git blame), then union
    /// their impact (what the change can break).
    ///
    /// The repo is the corpus's first on-disk local root; git runs there. The
    /// `direction`/`tests_only` knobs are forwarded to the per-seed impact walk
    /// (callers default to `Incoming`). Results are bounded for token safety.
    /// Returns an empty result (never an error) when the corpus has no local git
    /// root, the range is unresolvable, or it touched no indexed symbols.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] only on a database failure.
    #[instrument(skip(self))]
    pub async fn compute_diff_impact(
        &self,
        range: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> Result<DiffImpactResult, QueryError> {
        /// Cap on the diff seed set (token safety).
        const MAX_SEEDS: usize = 200;
        /// Cap on unioned impacted nodes (token safety).
        const MAX_IMPACTED: usize = 500;

        let empty = || DiffImpactResult {
            range: range.to_string(),
            changed_files: 0,
            changed_symbols: Vec::new(),
            impacted_symbols: 0,
            impacted_files: 0,
            impacted_tests: 0,
            risk: ImpactRisk::Low,
            impacted: Vec::new(),
        };

        // The repo dir is the first on-disk local corpus root.
        let roots = self.storage.list_corpus_roots().await?;
        // Dir roots (abs path + id) used to rebuild each changed file's stored
        // index key from its absolute path (ingest-key-locator-decouple).
        let dir_roots: Vec<(std::path::PathBuf, String)> = roots
            .iter()
            .filter(|r| matches!(r.kind, RootKind::Local) && Path::new(&r.path).is_dir())
            .map(|r| (std::path::PathBuf::from(&r.path), r.id.clone()))
            .collect();
        let Some(repo) = roots
            .iter()
            .find(|r| matches!(r.kind, RootKind::Local) && Path::new(&r.path).is_dir())
        else {
            return Ok(empty());
        };
        let Ok(changed) = crate::git::diff::changed_lines(Path::new(&repo.path), range) else {
            return Ok(empty());
        };

        let (seeds, changed_files) = self.diff_seed_set(&changed, &dir_roots, MAX_SEEDS).await?;

        // Union the blast radius across the seeds (shallowest depth wins).
        let mut impacted_map: HashMap<String, ImpactCaller> = HashMap::new();
        for seed in &seeds {
            if let Ok(ir) = self
                .compute_impact(&seed.symbol_id, max_depth, direction, tests_only)
                .await
            {
                for c in ir.callers {
                    impacted_map
                        .entry(c.symbol_id.clone())
                        .and_modify(|e| {
                            if c.depth < e.depth {
                                e.depth = c.depth;
                            }
                        })
                        .or_insert(c);
                }
            }
        }
        let mut impacted: Vec<ImpactCaller> = impacted_map.into_values().collect();
        impacted.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.name.cmp(&b.name))
        });
        impacted.truncate(MAX_IMPACTED);

        let mut files: HashSet<&str> = HashSet::new();
        let mut tests: HashSet<&str> = HashSet::new();
        for c in &impacted {
            files.insert(c.file.as_str());
            if is_test_path(&c.file) {
                tests.insert(c.file.as_str());
            }
        }
        let (impacted_symbols, impacted_files, impacted_tests) =
            (impacted.len(), files.len(), tests.len());
        let risk = compute_risk(impacted_symbols, impacted_files, impacted_tests);

        Ok(DiffImpactResult {
            range: range.to_string(),
            changed_files,
            changed_symbols: seeds,
            impacted_symbols,
            impacted_files,
            impacted_tests,
            risk,
            impacted,
        })
    }

    /// Find symbols that have zero references — candidates for safe deletion.
    ///
    /// Filters out `pub` symbols (since external callers can't be seen),
    /// entry points (`main`, `_main`), and `#[test]` items by name heuristic.
    /// `min_lines` skips trivial helpers below that length.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn find_dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> Result<Vec<DeadSymbol>, QueryError> {
        let filter = SymbolFilter {
            name: None,
            name_exact: None,
            kind: kind.map(String::from),
            module: module.map(String::from),
            visibility: None,
            file_path: None,
        };
        let symbols = self.storage.list_symbols(&filter).await?;

        let mut out: Vec<DeadSymbol> = Vec::new();
        for sym in symbols {
            if sym.visibility.starts_with("pub") {
                continue;
            }
            if is_entry_point(&sym.name) {
                continue;
            }
            let lines = sym
                .line_end
                .saturating_sub(sym.line_start)
                .saturating_add(1);
            if lines < min_lines {
                continue;
            }
            let refs = self.storage.query_refs(&sym.id, None).await?;
            if !refs.is_empty() {
                continue;
            }
            out.push(DeadSymbol {
                symbol_id: sym.id.0,
                name: sym.name,
                kind: sym.kind,
                visibility: sym.visibility,
                file: sym.file_path,
                line: sym.line_start,
                lines,
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Query cross-language bridge links with optional filters.
    ///
    /// Returns bridge links (export↔import pairs) matching the given criteria.
    /// Filters by file path, bridge kind, and/or language. When `query` is
    /// provided, filters links where the binding key contains the query string.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    pub async fn query_bridges(
        &self,
        query: Option<&str>,
        bridge_kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> Result<Vec<BridgeLinkDetail>, QueryError> {
        let mut links = self
            .storage
            .query_bridge_links(file_path, bridge_kind)
            .await?;

        // Apply additional filters not supported by the storage layer
        if let Some(q) = query {
            let q_lower = q.to_lowercase();
            links.retain(|l| {
                l.export_binding_key.to_lowercase().contains(&q_lower)
                    || l.import_binding_key.to_lowercase().contains(&q_lower)
                    || l.export_symbol.to_lowercase().contains(&q_lower)
                    || l.import_symbol.to_lowercase().contains(&q_lower)
            });
        }

        if let Some(lang) = language {
            let lang_lower = lang.to_lowercase();
            links.retain(|l| {
                l.export_language.to_lowercase() == lang_lower
                    || l.import_language.to_lowercase() == lang_lower
            });
        }

        Ok(links)
    }

    /// Resolve a stored file path to an absolute filesystem path.
    ///
    /// Paths from cloned repos are namespaced as `{root_id}/{relative_path}`.
    /// This method detects the root prefix, looks up the corpus root's
    /// absolute directory, and joins with the relative path. For local
    /// (un-namespaced) paths, returns the path as-is.
    pub(super) async fn resolve_source_path(&self, file_path: &str) -> String {
        // Legacy absolute keys (pre-decouple corpora) and bare file sources are
        // already usable as on-disk paths.
        if std::path::Path::new(file_path).is_absolute() {
            return file_path.to_string();
        }
        // Namespaced `{root_id}/{relative}` (multi-source corpora) → join that
        // root's absolute dir.
        if let Some(relative) = crate::ingestion::strip_root_prefix(file_path) {
            let root_id = &file_path[..file_path.len() - relative.len() - 1];
            if let Ok(Some(root)) = self.storage.get_corpus_root(root_id).await {
                return std::path::PathBuf::from(&root.path)
                    .join(relative)
                    .to_string_lossy()
                    .into_owned();
            }
        } else if let Ok(roots) = self.storage.list_corpus_roots().await {
            // Bare-relative (single-source corpus) → join the sole local root.
            if let Some(root) = roots.iter().find(|r| matches!(r.kind, RootKind::Local)) {
                return std::path::PathBuf::from(&root.path)
                    .join(file_path)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        file_path.to_string()
    }

    /// The corpus's local directory roots paired with their `root_id`, for
    /// rebuilding a stored index key from a file's absolute path (diff-impact
    /// over the `Backend` abstraction; ingest-key-locator-decouple).
    pub async fn local_dir_roots(&self) -> Vec<(std::path::PathBuf, String)> {
        self.storage
            .list_corpus_roots()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| matches!(r.kind, RootKind::Local) && Path::new(&r.path).is_dir())
            .map(|r| (std::path::PathBuf::from(&r.path), r.id))
            .collect()
    }

    /// Read the full UTF-8 contents of an indexed source file.
    ///
    /// Resolves the stored (possibly root-namespaced) `file_path` to an
    /// absolute filesystem path via [`Self::resolve_source_path`], then reads
    /// the entire file. Unlike [`Self::read_source_context`] — a best-effort
    /// context window for one symbol that swallows I/O errors into a
    /// placeholder string — this returns the whole file and surfaces a read
    /// failure as [`QueryError::FileUnavailable`], so callers (e.g. the desktop
    /// code browser) can distinguish a missing file from an empty one.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::FileUnavailable`] if the resolved path cannot be
    /// read (missing, permission denied, or not valid UTF-8).
    pub async fn read_file_content(&self, file_path: &str) -> Result<String, QueryError> {
        let resolved = self.resolve_source_path(file_path).await;
        tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|source| QueryError::FileUnavailable {
                path: file_path.to_string(),
                source,
            })
    }

    /// Legacy full symbol context used by compression and existing callers.
    /// Definition responses use [`Self::read_source_slice`] for bounded output.
    pub(super) async fn read_source_context(
        &self,
        file_path: &str,
        line_start: u32,
        line_end: u32,
    ) -> String {
        let resolved = self.resolve_source_path(file_path).await;
        let Ok(content) = tokio::fs::read_to_string(&resolved).await else {
            return format!("[source unavailable: {file_path}]");
        };
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let start = (line_start as usize).saturating_sub(1).saturating_sub(3);
        let end = (line_end as usize).min(total).saturating_add(3).min(total);
        lines[start..end].join("\n")
    }

    /// Read a bounded, line-safe definition slice with continuation metadata.
    #[allow(clippy::too_many_lines)] // line and byte continuations share one UTF-8-safe slicing path
    async fn read_source_slice(
        &self,
        file_path: &str,
        line_start: u32,
        line_end: u32,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> Result<SourceSlice, QueryError> {
        let resolved = self.resolve_source_path(file_path).await;
        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => content,
            Err(source) => {
                let range = SourceLineRange {
                    start: line_start,
                    end: line_end.max(line_start),
                };
                let error_code = if source.kind() == std::io::ErrorKind::PermissionDenied {
                    "permission_denied"
                } else {
                    "file_unavailable"
                };
                return Ok(SourceSlice {
                    text: format!("[source unavailable: {file_path}]"),
                    truncated: true,
                    omitted_lines: usize::try_from(
                        line_end.saturating_sub(line_start).saturating_add(1),
                    )
                    .unwrap_or(usize::MAX),
                    original: range,
                    returned: SourceLineRange {
                        start: line_start,
                        end: line_start,
                    },
                    continuation: None,
                    source_error: Some(error_code.to_string()),
                });
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let original_start = (line_start as usize)
            .saturating_sub(1)
            .saturating_sub(options.context_lines);
        let original_end_exclusive = (line_end as usize)
            .min(total)
            .saturating_add(options.context_lines)
            .min(total);
        let requested_start = options
            .start_line
            .map_or(original_start, |line| line.saturating_sub(1) as usize)
            .clamp(original_start, original_end_exclusive.saturating_sub(1));
        let returned_end_exclusive = requested_start
            .saturating_add(options.max_lines)
            .min(original_end_exclusive);
        let original_count = original_end_exclusive.saturating_sub(original_start);
        let returned_count = returned_end_exclusive.saturating_sub(requested_start);
        let line_number = |line: usize| u32::try_from(line).unwrap_or(u32::MAX);
        let original = SourceLineRange {
            start: line_number(original_start.saturating_add(1)),
            end: line_number(original_end_exclusive.max(original_start + 1)),
        };
        let returned = SourceLineRange {
            start: line_number(requested_start.saturating_add(1)),
            end: line_number(returned_end_exclusive.max(requested_start + 1)),
        };
        let full_text = lines[requested_start..returned_end_exclusive].join("\n");
        let mut byte_start = options.start_byte.unwrap_or(0).min(full_text.len());
        while byte_start < full_text.len() && !full_text.is_char_boundary(byte_start) {
            byte_start += 1;
        }
        let mut byte_end = byte_start
            .saturating_add(MAX_DEFINITION_SOURCE_BYTES)
            .min(full_text.len());
        while byte_end > byte_start && !full_text.is_char_boundary(byte_end) {
            byte_end -= 1;
        }
        let byte_has_more = byte_end < full_text.len();
        let line_has_more = returned_end_exclusive < original_end_exclusive;
        let continuation = if byte_has_more {
            Some(DefinitionContinuation {
                symbol_id: symbol_id.to_string(),
                start_line: line_number(requested_start.saturating_add(1)),
                max_lines: options.max_lines,
                start_byte: Some(byte_end),
                max_bytes: Some(MAX_DEFINITION_SOURCE_BYTES),
            })
        } else {
            line_has_more.then(|| DefinitionContinuation {
                symbol_id: symbol_id.to_string(),
                start_line: line_number(returned_end_exclusive.saturating_add(1)),
                max_lines: options.max_lines,
                start_byte: None,
                max_bytes: None,
            })
        };
        Ok(SourceSlice {
            text: full_text[byte_start..byte_end].to_string(),
            truncated: returned_count < original_count
                || requested_start > original_start
                || byte_start > 0
                || byte_has_more,
            omitted_lines: original_count.saturating_sub(returned_count),
            original,
            returned,
            continuation,
            source_error: None,
        })
    }
}

pub(super) fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("/tests/")
        || lower.contains("\\tests\\")
        || lower.contains("/test/")
        || lower.contains("\\test\\")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".test.js")
        || lower.ends_with(".test.jsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with(".spec.js")
        || lower.ends_with(".spec.jsx")
        || lower.ends_with("_spec.rb")
}

fn compute_risk(symbols: usize, files: usize, tests: usize) -> ImpactRisk {
    let score = symbols
        .saturating_add(files.saturating_mul(2))
        .saturating_add(tests.saturating_mul(3));
    if score > 50 || files > 10 {
        ImpactRisk::High
    } else if score > 15 || files > 3 {
        ImpactRisk::Medium
    } else {
        ImpactRisk::Low
    }
}

fn is_entry_point(name: &str) -> bool {
    matches!(name, "main" | "_main" | "_start")
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_DEFINITION_SOURCE_BYTES, MAX_INSPECT_RESPONSE_BYTES, bound_inspect_response,
        bounded_reference_group, compute_risk, is_entry_point, is_large_container, is_test_path,
        shape_reference_results,
    };
    use crate::embedding::Embedder;
    use crate::error::IndexError;
    use crate::index::HnswIndex;
    use crate::service::{
        DefinitionOptions, ImpactRisk, InspectImpactSummary, InspectInclude, InspectOptions,
        InspectReferenceGroup, InspectResult, QueryService, SymbolRefResult,
    };
    use crate::storage::{SqliteStorage, Storage, SymbolRecord, SymbolRefRecord};
    use crate::types::{RefKind, ResultLocator, SymbolId};
    use std::sync::Arc;

    struct ZeroEmbedder;

    impl Embedder for ZeroEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(vec![vec![0.0; 4]; texts.len()])
        }

        fn dimension(&self) -> usize {
            4
        }
    }

    fn query_service(storage: SqliteStorage) -> QueryService {
        QueryService::new(
            storage,
            Arc::new(ZeroEmbedder),
            Arc::new(HnswIndex::new(4, 100).unwrap()),
        )
    }

    fn stored_symbol(
        id: &str,
        name: &str,
        kind: &str,
        file: &str,
        start: u32,
        end: u32,
    ) -> SymbolRecord {
        SymbolRecord {
            id: SymbolId(id.to_string()),
            file_path: file.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            visibility: "pub".to_string(),
            signature: format!("pub {kind} {name}"),
            doc_comment: None,
            module_path: "fixture".to_string(),
            line_start: start,
            line_end: end,
            cyclomatic_complexity: None,
        }
    }

    fn reference(from_file: &str, from_line: u32, kind: &str) -> SymbolRefResult {
        SymbolRefResult {
            from_symbol_id: format!("sym-{from_file}-{from_line}"),
            from_name: "caller".into(),
            from_file: from_file.into(),
            from_line,
            to_symbol_id: "sym-target".into(),
            to_name: "target".into(),
            to_file: "src/target.rs".into(),
            to_line: 1,
            ref_kind: kind.into(),
        }
    }

    #[test]
    fn container_kind_classifier_covers_impl_class_and_module() {
        for kind in ["impl", "class", "module", "mod", "trait", "interface"] {
            assert!(is_large_container(kind), "{kind} should outline when large");
        }
        assert!(!is_large_container("function"));
    }

    #[tokio::test]
    async fn definitions_bound_large_bodies_outline_containers_and_continue_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated_bindings.rs");
        let source = (1..=1_000)
            .map(|line| format!("// 行🙂 {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, source).unwrap();
        let file = path.to_string_lossy().into_owned();
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[
                stored_symbol("sym-ordinary", "ordinary", "function", &file, 1, 5),
                stored_symbol("sym-impl", "ServiceImpl", "impl", &file, 10, 220),
                stored_symbol("sym-impl::run", "run", "function", &file, 20, 30),
                stored_symbol("sym-class", "Controller", "class", &file, 230, 440),
                stored_symbol("sym-class::send", "send", "method", &file, 240, 250),
                stored_symbol("sym-module", "runtime", "module", &file, 450, 660),
                stored_symbol("sym-module::tick", "tick", "function", &file, 460, 470),
                stored_symbol(
                    "sym-generated",
                    "generated_call",
                    "function",
                    &file,
                    670,
                    950,
                ),
            ])
            .await
            .unwrap();
        let service = query_service(storage);

        let ordinary = service.get_symbol_definition("sym-ordinary").await.unwrap();
        assert!(!ordinary.truncated);
        assert!(!ordinary.outline_only);
        assert!(ordinary.source_context.contains("行🙂"));

        for id in ["sym-impl", "sym-class", "sym-module"] {
            let definition = service.get_symbol_definition(id).await.unwrap();
            assert!(definition.truncated, "{id}");
            assert!(definition.outline_only, "{id}");
            assert!(!definition.child_symbols.is_empty(), "{id}");
            assert!(definition.continuation.is_some(), "{id}");
        }

        let generated = service
            .get_symbol_definition("sym-generated")
            .await
            .unwrap();
        assert!(generated.truncated);
        assert!(!generated.outline_only);
        assert_eq!(
            generated.returned_line_range.end - generated.returned_line_range.start + 1,
            160
        );
        let continuation = generated.continuation.clone().unwrap();
        let next = service
            .get_symbol_definition_with_options(
                "sym-generated",
                DefinitionOptions {
                    max_lines: 80,
                    start_line: Some(continuation.start_line),
                    ..DefinitionOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(next.returned_line_range.start, continuation.start_line);
        assert!(std::str::from_utf8(next.source_context.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn definitions_bound_minified_unicode_lines_and_continue_by_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generated.min.rs");
        let source = "資料🙂".repeat(20_000);
        std::fs::write(&path, &source).unwrap();
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[stored_symbol(
                "sym-minified",
                "minified",
                "function",
                &path.to_string_lossy(),
                1,
                1,
            )])
            .await
            .unwrap();
        let service = query_service(storage);

        let first = service.get_symbol_definition("sym-minified").await.unwrap();
        assert!(first.truncated);
        assert!(first.source_context.len() <= MAX_DEFINITION_SOURCE_BYTES);
        assert!(std::str::from_utf8(first.source_context.as_bytes()).is_ok());
        let continuation = first.continuation.unwrap();
        assert!(continuation.start_byte.is_some());
        let second = service
            .get_symbol_definition_with_options(
                "sym-minified",
                DefinitionOptions {
                    start_line: Some(continuation.start_line),
                    start_byte: continuation.start_byte,
                    ..DefinitionOptions::default()
                },
            )
            .await
            .unwrap();
        assert_ne!(first.source_context, second.source_context);
        assert_ne!(
            first.locator.identity.resolution, second.locator.identity.resolution,
            "continuation pages must account as distinct deliveries"
        );
        assert!(second.source_context.len() <= MAX_DEFINITION_SOURCE_BYTES);
    }

    #[tokio::test]
    async fn definition_reports_missing_source_as_explicit_partial_data() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[stored_symbol(
                "sym-missing-source",
                "missing",
                "function",
                "/definitely/missing/source.rs",
                1,
                2,
            )])
            .await
            .unwrap();
        let definition = query_service(storage)
            .get_symbol_definition("sym-missing-source")
            .await
            .unwrap();
        assert_eq!(definition.source_error.as_deref(), Some("file_unavailable"));
        assert!(definition.truncated);
        assert!(definition.source_context.contains("source unavailable"));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // one fixture proves all inspect group classifications together
    async fn inspect_groups_direct_edges_deterministically_and_reports_omissions() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let symbol = |id: &str, file: &str| stored_symbol(id, id, "function", file, 1, 2);
        storage
            .insert_symbols(&[
                symbol("target", "src/target.rs"),
                symbol("caller-a", "src/a.rs"),
                symbol("caller-b", "src/b.rs"),
                symbol("test-caller", "tests/target_test.rs"),
                symbol("callee", "src/callee.rs"),
                symbol("implementor", "src/impl.rs"),
                symbol("importer", "src/importer.rs"),
                symbol("bridge-client", "web/client.ts"),
            ])
            .await
            .unwrap();
        storage
            .insert_symbol_refs(&[
                SymbolRefRecord {
                    from_symbol_id: SymbolId("caller-a".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Calls,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("caller-b".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Calls,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("test-caller".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Calls,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("target".to_string()),
                    to_symbol_id: SymbolId("callee".to_string()),
                    ref_kind: RefKind::Calls,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("implementor".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Implements,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("importer".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Imports,
                },
                SymbolRefRecord {
                    from_symbol_id: SymbolId("bridge-client".to_string()),
                    to_symbol_id: SymbolId("target".to_string()),
                    ref_kind: RefKind::Bridge,
                },
            ])
            .await
            .unwrap();
        let service = query_service(storage);
        let result = service
            .inspect_symbol(
                "target",
                &InspectOptions {
                    include: vec![
                        InspectInclude::Callers,
                        InspectInclude::Callees,
                        InspectInclude::Implementors,
                        InspectInclude::Imports,
                        InspectInclude::Tests,
                        InspectInclude::Bridges,
                    ],
                    max_per_group: 1,
                    max_source_lines: 10,
                },
            )
            .await
            .unwrap();

        assert_eq!(result.callers.total, 3);
        assert_eq!(result.callers.items.len(), 1);
        assert_eq!(result.callers.omitted_count, 2);
        assert_eq!(result.callers.pagination.total, 3);
        assert!(result.callers.pagination.has_more);
        assert_eq!(result.callers.pagination.omitted_count, 2);
        assert!(
            result
                .callers
                .pagination
                .next_cursor
                .as_deref()
                .is_some_and(|cursor| cursor.starts_with("ref:"))
        );
        assert_eq!(result.callees.total, 1);
        assert_eq!(result.implementors.total, 1);
        assert_eq!(result.imports.total, 1);
        assert_eq!(result.tests.total, 1);
        assert_eq!(result.bridges.total, 1);
        assert_eq!(result.impact.direct_callers, 3);
        assert!(result.impact.affected_files >= 7);
        assert!(!matches!(result.impact.risk, ImpactRisk::Low));
        assert!(!result.next_actions.is_empty());
        assert!(result.partial_errors.is_empty());
        assert!(!result.truncated);
        assert_eq!(
            result.returned_bytes,
            serde_json::to_vec(&result).unwrap().len()
        );
        assert_eq!(result.original_bytes, result.returned_bytes);
    }

    #[tokio::test]
    async fn inspect_preserves_definition_when_reference_group_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.rs");
        std::fs::write(&path, "pub fn target() {}\n").unwrap();
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[stored_symbol(
                "target",
                "target",
                "function",
                &path.to_string_lossy(),
                1,
                1,
            )])
            .await
            .unwrap();
        let service = query_service(storage).with_inspect_reference_failure("injected failure");

        let result = service
            .inspect_symbol(
                "target",
                &InspectOptions {
                    include: vec![InspectInclude::Definition, InspectInclude::Callers],
                    max_per_group: 10,
                    max_source_lines: 20,
                },
            )
            .await
            .unwrap();

        assert!(result.definition.is_some());
        assert!(result.callers.items.is_empty());
        assert_eq!(result.partial_errors.len(), 1);
        assert_eq!(result.partial_errors[0].group, "references");
        assert!(
            result.partial_errors[0]
                .message
                .contains("injected failure")
        );
    }

    #[test]
    fn inspect_overall_budget_is_unicode_safe_and_preserves_group_continuation() {
        let long = "路径🙂".repeat(1_024);
        let references = (1..=200)
            .map(|line| SymbolRefResult {
                from_symbol_id: format!("caller-{line}-{long}"),
                from_name: long.clone(),
                from_file: format!("src/{long}/{line}.rs"),
                from_line: line,
                to_symbol_id: "target".to_string(),
                to_name: "target".to_string(),
                to_file: "src/target.rs".to_string(),
                to_line: 1,
                ref_kind: "calls".to_string(),
            })
            .collect();
        let result = bound_inspect_response(InspectResult {
            symbol_id: "target".to_string(),
            locator: ResultLocator::primary("target", "symbol_full"),
            definition: None,
            callers: bounded_reference_group(references, 200),
            callees: InspectReferenceGroup::default(),
            implementors: InspectReferenceGroup::default(),
            imports: InspectReferenceGroup::default(),
            tests: InspectReferenceGroup::default(),
            bridges: InspectReferenceGroup::default(),
            impact: InspectImpactSummary {
                direct_callers: 200,
                direct_callees: 0,
                affected_files: 200,
                relevant_tests: 0,
                risk: ImpactRisk::High,
            },
            partial_errors: Vec::new(),
            next_actions: Vec::new(),
            truncated: false,
            original_bytes: 0,
            returned_bytes: 0,
        });

        let serialized = serde_json::to_vec(&result).unwrap();
        assert!(result.truncated);
        assert!(serialized.len() <= MAX_INSPECT_RESPONSE_BYTES);
        assert_eq!(result.returned_bytes, serialized.len());
        assert!(result.original_bytes > result.returned_bytes);
        assert!(result.callers.omitted_count > 0);
        assert!(result.callers.pagination.has_more);
        assert!(
            result
                .callers
                .pagination
                .next_cursor
                .as_deref()
                .is_some_and(|cursor| cursor.starts_with("ref:"))
        );
        assert!(std::str::from_utf8(&serialized).is_ok());
    }

    #[test]
    fn references_are_deduplicated_and_production_callers_rank_first() {
        let call = reference("src/caller.rs", 10, "calls");
        let mut results = vec![
            reference("tests/caller_test.rs", 2, "calls"),
            reference("src/importer.rs", 1, "imports"),
            call.clone(),
            call,
        ];
        shape_reference_results(&mut results);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].from_file, "src/caller.rs");
        assert_eq!(results[1].from_file, "src/importer.rs");
        assert_eq!(results[2].from_file, "tests/caller_test.rs");
    }

    #[test]
    fn test_path_recognises_common_test_layouts() {
        assert!(is_test_path("crate/tests/integration.rs"));
        assert!(is_test_path("src/foo_test.go"));
        assert!(is_test_path("app/components/Button.test.tsx"));
        assert!(is_test_path("lib/parser.spec.js"));
        assert!(!is_test_path("src/lib.rs"));
        assert!(!is_test_path("docs/architecture.md"));
    }

    #[test]
    fn risk_scales_with_breadth() {
        assert!(matches!(compute_risk(1, 1, 0), ImpactRisk::Low));
        assert!(matches!(compute_risk(8, 4, 1), ImpactRisk::Medium));
        assert!(matches!(compute_risk(40, 12, 5), ImpactRisk::High));
    }

    #[test]
    fn entry_point_excludes_main_only() {
        assert!(is_entry_point("main"));
        assert!(is_entry_point("_start"));
        assert!(!is_entry_point("run"));
        assert!(!is_entry_point("helper"));
    }

    /// FL6 — `compute_impact(incoming, tests_only=true)` returns only the
    /// transitive callers living in test files (test↔code mapping), while
    /// the unfiltered walk returns every caller.
    #[tokio::test]
    async fn impact_tests_only_keeps_only_test_callers() {
        use crate::embedding::Embedder;
        use crate::error::IndexError;
        use crate::index::{HnswIndex, VectorIndex};
        use crate::service::{CallDirection, QueryService};
        use crate::storage::{SqliteStorage, Storage, SymbolRecord, SymbolRefRecord};
        use crate::types::{RefKind, SymbolId};
        use std::sync::Arc;

        struct ZeroEmbedder;
        impl Embedder for ZeroEmbedder {
            fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
                Ok(vec![vec![0.0; 4]; texts.len()])
            }
            fn dimension(&self) -> usize {
                4
            }
        }

        let sym = |id: &str, file: &str| SymbolRecord {
            id: SymbolId(id.into()),
            file_path: file.into(),
            name: id.rsplit("::").next().unwrap().into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: String::new(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 2,
            cyclomatic_complexity: None,
        };
        let calls = |from: &str, to: &str| SymbolRefRecord {
            from_symbol_id: SymbolId(from.into()),
            to_symbol_id: SymbolId(to.into()),
            ref_kind: RefKind::Calls,
        };

        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[
                sym("sym-svc::run", "src/svc.rs"),
                sym("sym-svc::helper", "src/helper.rs"),
                sym("sym-tests::it_runs", "tests/svc_test.rs"),
            ])
            .await
            .unwrap();
        // Both a production helper and a test transitively call `run`.
        storage
            .insert_symbol_refs(&[
                calls("sym-svc::helper", "sym-svc::run"),
                calls("sym-tests::it_runs", "sym-svc::run"),
            ])
            .await
            .unwrap();

        let embedder: Arc<dyn Embedder> = Arc::new(ZeroEmbedder);
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(4, 16).unwrap());
        let svc = QueryService::new(storage, embedder, index);

        // Unfiltered incoming: both callers.
        let all = svc
            .compute_impact("sym-svc::run", 3, CallDirection::Incoming, false)
            .await
            .unwrap();
        assert_eq!(all.callers.len(), 2, "helper + the test both call run");

        // tests_only: just the test.
        let tests = svc
            .compute_impact("sym-svc::run", 3, CallDirection::Incoming, true)
            .await
            .unwrap();
        assert_eq!(tests.callers.len(), 1, "only the test-file caller survives");
        assert_eq!(tests.callers[0].symbol_id, "sym-tests::it_runs");
        assert_eq!(
            tests.tests, 1,
            "the test-file count reflects the filtered set"
        );
    }

    /// FL3b — `get_symbol_references_through_implementors` adds callers of the
    /// same-named method on co-implementor types. With `English` and `Spanish`
    /// both implementing `Greeter`, querying `English::greet` surfaces the
    /// caller of `Spanish::greet` (the type-hierarchy hop) — which plain
    /// `get_symbol_references` does not.
    #[tokio::test]
    async fn references_through_implementors_spans_co_implementors() {
        use crate::embedding::Embedder;
        use crate::error::IndexError;
        use crate::index::{HnswIndex, VectorIndex};
        use crate::service::QueryService;
        use crate::storage::{SqliteStorage, Storage, SymbolRecord, SymbolRefRecord};
        use crate::types::{RefKind, SymbolId};
        use std::sync::Arc;

        struct ZeroEmbedder;
        impl Embedder for ZeroEmbedder {
            fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
                Ok(vec![vec![0.0; 4]; texts.len()])
            }
            fn dimension(&self) -> usize {
                4
            }
        }

        let sym = |id: &str, kind: &str| SymbolRecord {
            id: SymbolId(id.into()),
            file_path: "src/lib.rs".into(),
            name: id.rsplit("::").next().unwrap().into(),
            kind: kind.into(),
            visibility: "pub".into(),
            signature: String::new(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 2,
            cyclomatic_complexity: None,
        };
        let edge = |from: &str, to: &str, kind: RefKind| SymbolRefRecord {
            from_symbol_id: SymbolId(from.into()),
            to_symbol_id: SymbolId(to.into()),
            ref_kind: kind,
        };

        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .insert_symbols(&[
                sym("sym-g::Greeter", "trait"),
                sym("sym-g::English", "struct"),
                sym("sym-g::English::greet", "function"),
                sym("sym-g::Spanish", "struct"),
                sym("sym-g::Spanish::greet", "function"),
                sym("sym-g::call_en", "function"),
                sym("sym-g::call_es", "function"),
            ])
            .await
            .unwrap();
        storage
            .insert_symbol_refs(&[
                edge("sym-g::English", "sym-g::Greeter", RefKind::Implements),
                edge("sym-g::Spanish", "sym-g::Greeter", RefKind::Implements),
                edge("sym-g::call_en", "sym-g::English::greet", RefKind::Calls),
                edge("sym-g::call_es", "sym-g::Spanish::greet", RefKind::Calls),
            ])
            .await
            .unwrap();

        let embedder: Arc<dyn Embedder> = Arc::new(ZeroEmbedder);
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(4, 16).unwrap());
        let svc = QueryService::new(storage, embedder, index);

        // Plain references of English::greet: only its own direct caller.
        let plain = svc
            .get_symbol_references("sym-g::English::greet", Some(RefKind::Calls))
            .await
            .unwrap();
        assert!(
            plain.iter().any(|r| r.from_symbol_id == "sym-g::call_en"),
            "plain refs include the direct caller"
        );
        assert!(
            !plain.iter().any(|r| r.from_symbol_id == "sym-g::call_es"),
            "plain refs must NOT cross to the sibling implementor"
        );

        // Through implementors: the Spanish caller surfaces via the hop.
        let hier = svc
            .get_symbol_references_through_implementors(
                "sym-g::English::greet",
                Some(RefKind::Calls),
                16,
            )
            .await
            .unwrap();
        assert!(
            hier.iter().any(|r| r.from_symbol_id == "sym-g::call_en"),
            "hierarchy refs still include the direct caller"
        );
        let peer = hier
            .iter()
            .find(|r| r.from_symbol_id == "sym-g::call_es")
            .expect("hierarchy refs surface the co-implementor's caller");
        assert_eq!(
            peer.to_symbol_id, "sym-g::Spanish::greet",
            "the peer caller is attributed to the sibling method"
        );
    }
}
