//! Code intelligence operations for [`QueryService`].
//!
//! Symbol search, definition lookup, cross-reference queries, bridge link
//! queries, and source file resolution helpers.

use tracing::instrument;

use crate::storage::{BridgeLinkDetail, Storage, SymbolFilter, SymbolRecord};
use crate::types::{RefKind, SymbolId};

use super::{QueryError, QueryService, SymbolDefinition, SymbolRefResult};

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

        // Read source file and extract context lines
        let source_context = self
            .read_source_context(&symbol.file_path, symbol.line_start, symbol.line_end)
            .await;

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
            source_context,
        })
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

            for r in refs {
                let from = self.storage.get_symbol(&r.from_symbol_id).await?;
                let to = self.storage.get_symbol(&r.to_symbol_id).await?;

                if let (Some(from_sym), Some(to_sym)) = (from, to) {
                    results.push(SymbolRefResult {
                        from_symbol_id: from_sym.id.0,
                        from_name: from_sym.name,
                        from_file: from_sym.file_path,
                        from_line: from_sym.line_start,
                        to_symbol_id: to_sym.id.0,
                        to_name: to_sym.name,
                        to_file: to_sym.file_path,
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

            for link in bridge_links {
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
        if let Some(relative) = crate::ingestion::strip_root_prefix(file_path) {
            // Extract root ID (everything before the first '/')
            let root_id = &file_path[..file_path.len() - relative.len() - 1];
            if let Ok(Some(root)) = self.storage.get_corpus_root(root_id).await {
                let mut resolved = std::path::PathBuf::from(&root.path);
                resolved.push(relative);
                return resolved.to_string_lossy().to_string();
            }
        }
        file_path.to_string()
    }

    /// Read source file lines for symbol context display.
    ///
    /// Returns the symbol's source lines with 3 lines of surrounding context.
    /// Falls back to a placeholder if the file cannot be read.
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

        // 3 lines of context before and after, clamped to file bounds
        let ctx = 3;
        let start = (line_start as usize).saturating_sub(1).saturating_sub(ctx);
        let end = (line_end as usize)
            .min(total)
            .saturating_add(ctx)
            .min(total);

        lines[start..end].join("\n")
    }
}
