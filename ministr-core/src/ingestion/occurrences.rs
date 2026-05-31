//! Occurrence indexing pass (F-CodeExplorer v2).
//!
//! Opt-in, per-language post-pass over the fully-ingested corpus: for each
//! supported file it extracts every identifier occurrence (via
//! [`crate::code::extract_occurrences`]) and resolves each to a `symbol_id`
//! against the complete symbol table, then persists the resolved sites so the
//! code browser can make any token clickable.
//!
//! **Opt-in:** runs only when `MINISTR_INDEX_OCCURRENCES` is set to a truthy
//! value (`1`/`true`/`yes`). Occurrence rows roughly equal the identifier
//! count of the corpus — order-of-magnitude larger than the symbol table — so
//! it stays off by default and is enabled per-deployment when the desktop
//! code browser wants exact byte-range navigation.
//!
//! **Precision over recall:** an occurrence resolves to a same-file definition
//! first, else to a *uniquely*-named global symbol; ambiguous or unknown names
//! are skipped (the GUI falls back to a `search_symbols` lookup). v1 is Rust.

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::{info, warn};

use crate::code::{AstParser, Occurrence, extract_occurrences};
use crate::storage::traits::{OccurrenceRecord, Storage, SymbolFilter, SymbolRecord};
use crate::types::SymbolId;

/// Env var that opts a corpus into occurrence indexing.
const OPT_IN_ENV: &str = "MINISTR_INDEX_OCCURRENCES";

/// Whether occurrence indexing is enabled for this process.
#[must_use]
pub fn occurrence_indexing_enabled() -> bool {
    matches!(
        std::env::var(OPT_IN_ENV).ok().as_deref(),
        Some("1" | "true" | "yes" | "TRUE" | "YES")
    )
}

/// Map a stored file path to the occurrence-extractor language id, or `None`
/// if occurrences aren't supported for it. v1 = Rust only.
fn occurrence_language(file_path: &str) -> Option<&'static str> {
    match file_path.rsplit('.').next() {
        Some("rs") => Some("rust"),
        _ => None,
    }
}

/// All-symbols filter (no predicates).
fn all_symbols_filter() -> SymbolFilter {
    SymbolFilter {
        name: None,
        name_exact: None,
        kind: None,
        visibility: None,
        module: None,
        file_path: None,
    }
}

/// Build the global name → resolved-symbol index. A name maps to `Some(id)`
/// only when exactly one symbol bears it corpus-wide; names borne by several
/// symbols map to `None` (ambiguous — skipped at resolution).
fn build_global_index(symbols: &[SymbolRecord]) -> HashMap<String, Option<SymbolId>> {
    let mut index: HashMap<String, Option<SymbolId>> = HashMap::new();
    for sym in symbols {
        index
            .entry(sym.name.clone())
            .and_modify(|e| *e = None) // seen more than once → ambiguous
            .or_insert_with(|| Some(sym.id.clone()));
    }
    index
}

/// Resolve a file's occurrences to storable records (pure).
///
/// Same-file definitions win; otherwise a uniquely-named global symbol;
/// otherwise the occurrence is dropped (precision over recall).
#[must_use]
pub(crate) fn resolve_occurrences(
    occurrences: &[Occurrence],
    file_path: &str,
    same_file: &HashMap<String, SymbolId>,
    global: &HashMap<String, Option<SymbolId>>,
) -> Vec<OccurrenceRecord> {
    let mut out = Vec::new();
    for occ in occurrences {
        let resolved = same_file
            .get(&occ.name)
            .cloned()
            .or_else(|| global.get(&occ.name).and_then(Clone::clone));
        if let Some(symbol_id) = resolved {
            out.push(OccurrenceRecord {
                file_path: file_path.to_string(),
                name: occ.name.clone(),
                symbol_id,
                byte_start: occ.byte_start,
                byte_end: occ.byte_end,
                line: occ.line,
                col: occ.col,
            });
        }
    }
    out
}

/// Opt-in occurrence-indexing pass over the full file set, mirroring the
/// bridge rebuild in `finalize_ingestion`. No-op (cheap early return) unless
/// [`occurrence_indexing_enabled`].
pub(super) async fn rebuild_occurrences<S: Storage + ?Sized>(
    all_files: &[(PathBuf, String)],
    storage: &S,
) {
    if !occurrence_indexing_enabled() {
        return;
    }

    let all_symbols = match storage.list_symbols(&all_symbols_filter()).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "occurrence indexing: failed to load symbol table; skipping");
            return;
        }
    };
    let global = build_global_index(&all_symbols);

    let mut parser = AstParser::new();
    let mut files_indexed = 0usize;
    let mut sites = 0usize;

    for (abs_path, relative) in all_files {
        let Some(language) = occurrence_language(relative) else {
            continue;
        };
        let Ok(source) = tokio::fs::read(abs_path).await else {
            continue;
        };
        let Ok(tree) = parser.parse(&source) else {
            continue;
        };
        let occ = extract_occurrences(&tree, &source, language);
        if occ.is_empty() {
            continue;
        }

        let same_file: HashMap<String, SymbolId> = all_symbols
            .iter()
            .filter(|s| &s.file_path == relative)
            .map(|s| (s.name.clone(), s.id.clone()))
            .collect();

        let records = resolve_occurrences(&occ, relative, &same_file, &global);

        if let Err(e) = storage.delete_occurrences_for_file(relative).await {
            warn!(error = %e, path = %relative, "occurrence indexing: delete failed");
            continue;
        }
        if !records.is_empty() {
            sites += records.len();
            if let Err(e) = storage.insert_occurrences(&records).await {
                warn!(error = %e, path = %relative, "occurrence indexing: insert failed");
                continue;
            }
        }
        files_indexed += 1;
    }

    info!(
        files = files_indexed,
        sites, "occurrence indexing complete (opt-in)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(name: &str, byte_start: u32) -> Occurrence {
        Occurrence {
            name: name.to_string(),
            byte_start,
            byte_end: byte_start + u32::try_from(name.len()).unwrap(),
            line: 1,
            col: 0,
        }
    }

    #[test]
    fn same_file_definition_wins_over_global() {
        let occs = [occ("Foo", 10)];
        let mut same_file = HashMap::new();
        same_file.insert("Foo".to_string(), SymbolId("sym-local::Foo".into()));
        let mut global = HashMap::new();
        global.insert("Foo".to_string(), Some(SymbolId("sym-other::Foo".into())));

        let out = resolve_occurrences(&occs, "src/a.rs", &same_file, &global);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbol_id.0, "sym-local::Foo");
        assert_eq!(out[0].byte_start, 10);
    }

    #[test]
    fn unique_global_resolves_ambiguous_and_unknown_skip() {
        let occs = [occ("Uniq", 0), occ("Ambig", 5), occ("Nope", 10)];
        let same_file = HashMap::new();
        let mut global = HashMap::new();
        global.insert("Uniq".to_string(), Some(SymbolId("sym-x::Uniq".into())));
        global.insert("Ambig".to_string(), None); // ambiguous

        let out = resolve_occurrences(&occs, "src/a.rs", &same_file, &global);
        // Only the uniquely-named global symbol resolves.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Uniq");
    }

    #[test]
    fn global_index_marks_duplicates_ambiguous() {
        let symbols = vec![
            SymbolRecord {
                id: SymbolId("sym-a::Foo".into()),
                file_path: "a.rs".into(),
                name: "Foo".into(),
                kind: "struct".into(),
                visibility: "pub".into(),
                signature: String::new(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
            SymbolRecord {
                id: SymbolId("sym-b::Foo".into()),
                file_path: "b.rs".into(),
                name: "Foo".into(),
                kind: "struct".into(),
                visibility: "pub".into(),
                signature: String::new(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
            SymbolRecord {
                id: SymbolId("sym-c::Bar".into()),
                file_path: "c.rs".into(),
                name: "Bar".into(),
                kind: "fn".into(),
                visibility: "pub".into(),
                signature: String::new(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
        ];
        let index = build_global_index(&symbols);
        assert_eq!(index.get("Foo"), Some(&None)); // duplicate → ambiguous
        assert_eq!(index.get("Bar"), Some(&Some(SymbolId("sym-c::Bar".into()))));
    }
}
