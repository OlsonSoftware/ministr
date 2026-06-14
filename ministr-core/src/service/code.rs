//! Code intelligence operations for [`QueryService`].
//!
//! Symbol search, definition lookup, cross-reference queries, bridge link
//! queries, and source file resolution helpers.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tracing::instrument;

use crate::storage::{BridgeLinkDetail, Storage, SymbolFilter, SymbolRecord};
use crate::types::{RefKind, RootKind, SymbolId};

use super::{
    CallDirection, DeadSymbol, DiffChangeAuthor, DiffChangedSymbol, DiffImpactResult, ImpactCaller,
    ImpactResult, ImpactRisk, QueryError, QueryService, SymbolDefinition, SymbolRefResult,
};

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
    use super::{compute_risk, is_entry_point, is_test_path};
    use crate::service::ImpactRisk;

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
