//! AST-aware code parser using tree-sitter.
//!
//! Splits source files at function/struct/enum/trait/impl boundaries, producing
//! one [`Section`] per top-level symbol with correct byte ranges. Generates
//! multi-resolution chunks: a file-level overview section and per-symbol sections.
//!
//! Supports multiple languages via the [`GrammarRegistry`]. For Rust source,
//! uses the dedicated Rust extractor; for other languages, uses the generic
//! extractor with language-agnostic heuristics.

use std::path::Path;

use crate::code::{AstParser, GrammarRegistry, Symbol, extract_symbols, generic_extract_symbols};
use crate::error::ParseError;
use crate::parser::section_id::{generate_code_section_id, generate_section_id};
use crate::token::count_tokens;
use crate::types::{ContentId, DocumentTree, Section, SectionId};

/// Token budget for a single code chunk before it is split at inner AST
/// boundaries (cAST-style). Set to the default embedder's max sequence length:
/// a symbol whose source exceeds this was previously
/// emitted whole and then silently truncated by the embedder. Counted in
/// [`count_tokens`] (cl100k BPE) units — the same proxy the rest of the pipeline
/// budgets against. Per-embedder budgeting (per-corpus routing) can make
/// this dynamic later.
const CODE_CHUNK_BUDGET: usize = 256;

/// A document parser for source code files.
///
/// Uses tree-sitter to parse source into an AST, then extracts symbols
/// and produces multi-resolution sections: a file-level overview and one
/// section per top-level symbol.
///
/// Supports multiple languages via the [`GrammarRegistry`]. For Rust files,
/// the dedicated Rust extractor is used for maximum fidelity. For other
/// languages with available grammars, the generic extractor provides
/// language-agnostic symbol extraction using node kind heuristics.
///
/// # Examples
///
/// ```
/// use ministr_core::parser::{CodeParser, DocumentParser};
/// use std::path::Path;
///
/// let parser = CodeParser::new();
/// let source = "/// A greeting.\npub fn hello() -> String { String::new() }\n";
/// let tree = parser.parse(Path::new("lib.rs"), source).unwrap();
/// assert!(!tree.sections.is_empty());
/// ```
pub struct CodeParser;

impl CodeParser {
    /// Create a new code parser.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

impl super::DocumentParser for CodeParser {
    fn parse(&self, path: &Path, content: &str) -> Result<DocumentTree, ParseError> {
        let source = content.as_bytes();

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let registry = GrammarRegistry::global();
        let lang_name = registry.language_name_for_extension(ext);
        let is_rust = lang_name == Some("rust");

        // Select parser: use the grammar registry to find the right language.
        // On parse failure (most commonly the per-file parse-budget
        // timeout enforced inside `AstParser::parse`) we degrade
        // gracefully to the text-level fallback tree rather than
        // dropping the file entirely. Pathologically deep templates
        // (Slate widgets, recursive UE traits) shouldn't make a
        // header invisible.
        let tree = if is_rust {
            let mut ast_parser = AstParser::try_new()?;
            match ast_parser.parse(source) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "tree-sitter parse failed, degrading to text fallback"
                    );
                    return Ok(build_fallback_tree(path, content));
                }
            }
        } else if let Some(ts_lang) = registry.language_for_extension(ext) {
            let mut ast_parser = AstParser::with_language(ts_lang)?;
            match ast_parser.parse(source) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        ext = %ext,
                        error = %e,
                        "tree-sitter parse failed, degrading to text fallback"
                    );
                    return Ok(build_fallback_tree(path, content));
                }
            }
        } else {
            // No tree-sitter grammar available.
            // Use heuristic assembly parser for assembly files.
            if super::assembly::is_assembly_extension(ext) {
                return Ok(super::assembly::parse_assembly(path, content));
            }
            return Ok(build_fallback_tree(path, content));
        };

        // Derive module path from file stem (e.g. "config" from "src/config.rs")
        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let module_path: Vec<&str> = if file_stem == "lib"
            || file_stem == "main"
            || file_stem == "mod"
            || file_stem == "index"
            || file_stem == "__init__"
        {
            // Common "root" file names don't add a module segment
            vec![]
        } else {
            vec![file_stem]
        };

        let source_path = path.to_string_lossy();

        // Use the dedicated Rust extractor for Rust, generic for everything else
        let symbols = if is_rust {
            extract_symbols(&tree, source, &source_path, &module_path)
        } else {
            generic_extract_symbols(&tree, source, &source_path, &module_path)
        };

        // Build file-level overview section (depth 1)
        let file_overview = build_file_overview(&source_path, &module_path, content, &symbols);

        // Build per-symbol sections (depth 2). Over-budget symbols are split at
        // inner AST boundaries into contiguous <=budget sibling sub-sections.
        let symbol_sections: Vec<Section> = symbols
            .iter()
            .flat_map(|sym| build_symbol_sections(&source_path, content, sym, &tree))
            .collect();

        // File-level section with symbol sections as children
        let root_section = Section {
            children: symbol_sections,
            ..file_overview
        };

        let title = format!("{} (source)", path.display());
        let doc_id = ContentId(source_path.to_string());

        Ok(DocumentTree {
            id: doc_id,
            title,
            source_path: source_path.to_string(),
            sections: vec![root_section],
            summary: None,
        })
    }
}

/// Build the file-level overview section.
///
/// Contains the module-level doc comment (if any) and a list of public symbol
/// signatures as the section text.
fn build_file_overview(
    source_path: &str,
    module_path: &[&str],
    content: &str,
    symbols: &[Symbol],
) -> Section {
    let id = SectionId(generate_section_id(source_path, &[]));

    // Extract module-level doc comment (//! lines at the top of the file)
    let module_doc = extract_module_doc(content);

    // Build public symbol summary (signatures only)
    let pub_symbols: Vec<&Symbol> = symbols
        .iter()
        .filter(|s| s.visibility.is_public())
        .collect();

    let mut text_parts = Vec::new();
    if let Some(doc) = &module_doc {
        text_parts.push(doc.clone());
    }
    if !pub_symbols.is_empty() {
        let mut sig_lines = Vec::with_capacity(pub_symbols.len());
        for sym in &pub_symbols {
            sig_lines.push(sym.signature.clone());
        }
        text_parts.push(sig_lines.join("\n"));
    }

    let heading = if module_path.is_empty() {
        source_path.to_string()
    } else {
        module_path.join("::")
    };

    Section {
        id,
        heading_path: vec![heading],
        depth: 1,
        text: text_parts.join("\n\n"),
        structural_nodes: Vec::new(),
        children: Vec::new(),
        claims: Vec::new(),
        summary: None,
    }
}

/// Build the section(s) for a single symbol.
///
/// A symbol whose source fits within [`CODE_CHUNK_BUDGET`] yields exactly one
/// section — byte-for-byte the same as before this cAST split was added. An
/// over-budget symbol (a long function, a large struct/enum) is split at inner
/// AST boundaries into contiguous `<=budget` sibling sub-sections that together
/// cover the whole symbol with no text loss, so each sub-chunk embeds without
/// being truncated. Sub-section ids keep the symbol id as a prefix
/// (`<id>#part0`, `#part1`, …) so id-substring lookups still resolve.
fn build_symbol_sections(
    source_path: &str,
    content: &str,
    symbol: &Symbol,
    tree: &tree_sitter::Tree,
) -> Vec<Section> {
    let module_refs: Vec<&str> = symbol.module_path.iter().map(String::as_str).collect();
    // Include item kind in the section ID to disambiguate (e.g. struct Foo vs impl Foo).
    // For impl blocks, append the byte offset to handle multiple impls for the same type
    // (e.g. `impl AstParser` and `impl Default for AstParser`).
    let qualified_name = if symbol.kind == crate::code::ItemKind::Impl {
        format!("impl-{}-{}", symbol.name, symbol.byte_range.start)
    } else {
        symbol.name.clone()
    };
    let base_id = generate_code_section_id(source_path, &module_refs, &qualified_name);

    // Build heading: "kind Name" (e.g. "struct MinistrConfig", "fn hello")
    let kind_label = symbol.kind.as_str();
    let heading = format!("{kind_label} {}", symbol.name);

    // Full source text of the symbol (including doc comments, attributes, body).
    let full_source = &content[symbol.byte_range.clone()];

    // Text exactly as a single section would embed it: doc comment + full source.
    let single_text = match &symbol.doc_comment {
        Some(doc) => format!("{doc}\n\n{full_source}"),
        None => full_source.to_string(),
    };

    // Fits the budget: emit one section, identical to the pre-split behaviour.
    if count_tokens(&single_text) <= CODE_CHUNK_BUDGET {
        return vec![make_symbol_section(
            base_id,
            source_path,
            &heading,
            single_text,
        )];
    }

    // Over budget: split at inner AST boundaries into contiguous byte ranges.
    let ranges = split_symbol_ranges(symbol, tree, content.as_bytes());
    if ranges.len() <= 1 {
        // Could not split further (e.g. a single huge leaf with no inner
        // structure): emit whole. The embedder still truncates, but that is no
        // worse than before.
        return vec![make_symbol_section(
            base_id,
            source_path,
            &heading,
            single_text,
        )];
    }

    let n = ranges.len();
    ranges
        .into_iter()
        .enumerate()
        .map(|(i, range)| {
            let chunk_src = &content[range];
            // Prepend the doc comment to the first part only (mirrors the
            // single-section convention).
            let text = match (&symbol.doc_comment, i) {
                (Some(doc), 0) => format!("{doc}\n\n{chunk_src}"),
                _ => chunk_src.to_string(),
            };
            let part_heading = format!("{heading} (part {}/{n})", i + 1);
            make_symbol_section(
                format!("{base_id}#part{i}"),
                source_path,
                &part_heading,
                text,
            )
        })
        .collect()
}

/// Assemble one depth-2 symbol section from its id, heading and text.
fn make_symbol_section(id: String, source_path: &str, heading: &str, text: String) -> Section {
    Section {
        id: SectionId(id),
        heading_path: vec![source_path.to_string(), heading.to_string()],
        depth: 2,
        text,
        structural_nodes: Vec::new(),
        children: Vec::new(),
        claims: Vec::new(),
        summary: None,
    }
}

/// Compute contiguous byte sub-ranges of an over-budget symbol, cut at inner
/// AST boundaries so each range stays within [`CODE_CHUNK_BUDGET`] where the
/// structure allows. The ranges tile `symbol.byte_range` exactly (no overlap,
/// no gap), so concatenating their slices reproduces the symbol verbatim.
fn split_symbol_ranges(
    symbol: &Symbol,
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> Vec<std::ops::Range<usize>> {
    let start = symbol.byte_range.start;
    let end = symbol.byte_range.end;
    let Some(node) = tree
        .root_node()
        .descendant_for_byte_range(start, end.saturating_sub(1))
    else {
        return vec![symbol.byte_range.clone()];
    };

    let mut cuts = Vec::new();
    collect_cut_points(node, source, CODE_CHUNK_BUDGET, &mut cuts);
    // Keep only cut points strictly inside this symbol's span (the descendant
    // node may be a shared parent covering sibling items too).
    cuts.retain(|&c| c > start && c < end);
    cuts.sort_unstable();
    cuts.dedup();

    let mut ranges = Vec::with_capacity(cuts.len() + 1);
    let mut prev = start;
    for c in cuts {
        ranges.push(prev..c);
        prev = c;
    }
    ranges.push(prev..end);
    ranges
}

/// Walk a node's named children, recording byte offsets where a new chunk
/// should begin so that no chunk exceeds `budget` tokens. Oversized children are
/// recursed into (their inner boundaries become finer cuts); a small leading
/// sibling such as a doc comment stays attached to the chunk that follows it.
fn collect_cut_points(node: tree_sitter::Node, source: &[u8], budget: usize, out: &mut Vec<usize>) {
    let mut acc = 0usize;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let text = String::from_utf8_lossy(&source[child.start_byte()..child.end_byte()]);
        let tokens = count_tokens(&text);

        if tokens > budget {
            // Recurse: the child's own inner boundaries chunk it. Leave `acc`
            // alone so a small preceding sibling (doc comment, attribute) merges
            // into the child's first chunk rather than becoming a stub chunk.
            collect_cut_points(child, source, budget, out);
            acc = 0;
        } else if acc > 0 && acc + tokens > budget {
            out.push(child.start_byte());
            acc = tokens;
        } else {
            acc += tokens;
        }
    }
}

/// Extract the module-level doc comment (`//!` lines) from the start of a file.
fn extract_module_doc(content: &str) -> Option<String> {
    let mut doc_lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("//!") {
            let doc_text = rest.strip_prefix(' ').unwrap_or(rest);
            doc_lines.push(doc_text.to_string());
        } else if trimmed.is_empty() && !doc_lines.is_empty() {
            // Allow blank lines within the module doc block
            doc_lines.push(String::new());
        } else if !trimmed.is_empty() && !trimmed.starts_with("//!") {
            break;
        }
    }

    // Trim trailing empty lines
    while doc_lines.last().is_some_and(String::is_empty) {
        doc_lines.pop();
    }

    if doc_lines.is_empty() {
        return None;
    }
    Some(doc_lines.join("\n"))
}

/// Build a minimal document tree for files without a tree-sitter grammar.
///
/// Returns a single section containing the file content as-is, with no
/// symbol-level children. This allows files with unsupported languages
/// to still be indexed for text search.
fn build_fallback_tree(path: &Path, content: &str) -> DocumentTree {
    let source_path = path.to_string_lossy();
    let id = SectionId(generate_section_id(&source_path, &[]));

    let root_section = Section {
        id,
        heading_path: vec![source_path.to_string()],
        depth: 1,
        text: content.to_string(),
        structural_nodes: Vec::new(),
        children: Vec::new(),
        claims: Vec::new(),
        summary: None,
    };

    DocumentTree {
        id: ContentId(source_path.to_string()),
        title: format!("{} (source)", path.display()),
        source_path: source_path.to_string(),
        sections: vec![root_section],
        summary: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::DocumentParser;

    const SAMPLE_RUST: &str = r#"//! A sample module.

use std::path::Path;

/// Configuration for the app.
#[derive(Debug)]
pub struct Config {
    pub name: String,
    pub port: u16,
}

impl Config {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self {
            name: "app".into(),
            port: 8080,
        }
    }
}

/// Start the server.
pub fn start(config: &Config) {
    println!("Starting {} on {}", config.name, config.port);
}

fn internal_helper() {}
"#;

    #[test]
    fn parse_produces_document_tree() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();

        assert_eq!(tree.source_path, "src/config.rs");
        assert!(tree.title.contains("config.rs"));
        assert_eq!(tree.sections.len(), 1, "should have one root section");
    }

    #[test]
    fn root_section_has_file_overview() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];

        assert_eq!(root.depth, 1);
        // Module doc should be present
        assert!(
            root.text.contains("A sample module"),
            "root text should contain module doc"
        );
        // Public signatures should be listed
        assert!(
            root.text.contains("pub struct Config"),
            "root text should contain Config signature"
        );
        assert!(
            root.text.contains("pub fn start"),
            "root text should contain start signature"
        );
    }

    #[test]
    fn symbol_sections_at_depth_2() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];

        // Should have children: Config (struct), Config (impl), new (method), start (fn), internal_helper (fn)
        assert_eq!(
            root.children.len(),
            5,
            "expected 5 symbol sections, got {}: {:?}",
            root.children.len(),
            root.children
                .iter()
                .map(|c| &c.heading_path)
                .collect::<Vec<_>>()
        );

        for child in &root.children {
            assert_eq!(child.depth, 2);
        }
    }

    #[test]
    fn section_ids_follow_code_pattern() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];

        // Root section ID is the file root
        assert_eq!(root.id.0, "src/config.rs#root");

        // Find the Config struct section
        let config_section = root
            .children
            .iter()
            .find(|c| {
                c.heading_path
                    .last()
                    .is_some_and(|h| h.contains("struct Config"))
            })
            .expect("should have Config struct section");
        assert_eq!(config_section.id.0, "src/config.rs#config::Config");

        // Find the start function section (heading uses ItemKind::as_str = "function")
        let start_section = root
            .children
            .iter()
            .find(|c| {
                c.heading_path
                    .last()
                    .is_some_and(|h| h.contains("function start"))
            })
            .expect("should have start function section");
        assert_eq!(start_section.id.0, "src/config.rs#config::start");
    }

    #[test]
    fn symbol_sections_contain_full_source() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];

        let config_section = root
            .children
            .iter()
            .find(|c| {
                c.heading_path
                    .last()
                    .is_some_and(|h| h.contains("struct Config"))
            })
            .expect("should have Config struct section");

        // Full source should include the struct body
        assert!(config_section.text.contains("pub name: String"));
        assert!(config_section.text.contains("pub port: u16"));
    }

    #[test]
    fn chunk_boundaries_align_with_ast() {
        let parser = CodeParser::new();
        let tree = parser
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];

        // Verify no function is split mid-body by checking each symbol section
        // contains balanced braces (the full body)
        for child in &root.children {
            let text = &child.text;
            let open_braces = text.chars().filter(|&c| c == '{').count();
            let close_braces = text.chars().filter(|&c| c == '}').count();
            assert_eq!(
                open_braces, close_braces,
                "unbalanced braces in section {:?}: opens={open_braces}, closes={close_braces}",
                child.heading_path
            );
        }
    }

    #[test]
    fn lib_rs_has_no_module_path_segment() {
        let parser = CodeParser::new();
        let source = "pub fn foo() {}\n";
        let tree = parser.parse(Path::new("src/lib.rs"), source).unwrap();
        let root = &tree.sections[0];

        let foo = &root.children[0];
        // lib.rs symbols should not have a module path prefix
        assert_eq!(foo.id.0, "src/lib.rs#foo");
    }

    #[test]
    fn extract_module_doc_basic() {
        let doc = extract_module_doc("//! Hello.\n//! World.\n\nuse std::io;\n");
        assert_eq!(doc.as_deref(), Some("Hello.\nWorld."));
    }

    #[test]
    fn extract_module_doc_none_when_absent() {
        let doc = extract_module_doc("use std::io;\nfn main() {}\n");
        assert!(doc.is_none());
    }

    #[test]
    fn parse_empty_file() {
        let parser = CodeParser::new();
        let tree = parser.parse(Path::new("empty.rs"), "").unwrap();
        assert_eq!(tree.sections.len(), 1);
        assert!(tree.sections[0].children.is_empty());
    }

    /// Shader files (HLSL / GLSL / MSL / WGSL) have no tree-sitter
    /// grammar, so they fall through to `build_fallback_tree` and get
    /// indexed at text level — better than being silently dropped.
    /// This is the entry-level Phase 5 win; richer symbol extraction
    /// is a follow-up.
    #[test]
    fn shader_files_fall_back_to_text_indexing() {
        let parser = CodeParser::new();
        let usf = "#include \"/Engine/Public/Platform.ush\"\n\n\
                   Texture2D SourceTexture;\n\
                   SamplerState BilinearSampler;\n\n\
                   float4 MainPS(float2 UV : TEXCOORD0) : SV_Target0\n\
                   {\n    return SourceTexture.Sample(BilinearSampler, UV);\n}\n";
        for ext in [
            "shader.usf",
            "shader.ush",
            "shader.hlsl",
            "shader.glsl",
            "shader.frag",
            "shader.metal",
            "shader.wgsl",
        ] {
            let tree = parser.parse(Path::new(ext), usf).unwrap();
            assert_eq!(tree.sections.len(), 1, "{ext}: should yield one section");
            let root = &tree.sections[0];
            assert!(
                root.text.contains("MainPS") || root.text.contains("Texture2D"),
                "{ext}: full source should be in the fallback section text"
            );
            assert!(
                root.children.is_empty(),
                "{ext}: no symbol-level children expected from fallback path"
            );
        }
    }

    // C3.4: Chunk a real ministr-core source file
    #[test]
    fn chunk_real_config_rs() {
        let source = std::fs::read_to_string("src/config.rs").expect("cannot read config.rs");
        let parser = CodeParser::new();
        let tree = parser.parse(Path::new("src/config.rs"), &source).unwrap();
        let root = &tree.sections[0];

        // Should have MinistrConfig struct as a child section
        let ministr_config = root
            .children
            .iter()
            .find(|c| {
                c.id.0.contains("MinistrConfig")
                    && c.heading_path.last().is_some_and(|h| h.contains("struct"))
            })
            .expect("should have MinistrConfig struct section");
        assert_eq!(ministr_config.depth, 2);
        // Section should contain the full struct body
        assert!(ministr_config.text.contains("MinistrConfig"));

        // A symbol that fits CODE_CHUNK_BUDGET stays whole (balanced braces). A
        // symbol that exceeds it is split into `#partN` fragments at inner AST
        // boundaries — individually unbalanced, but balanced when rejoined.
        let mut joined_parts = String::new();
        for child in &root.children {
            if child.id.0.contains("#part") {
                joined_parts.push_str(&child.text);
                continue;
            }
            let open = child.text.chars().filter(|&c| c == '{').count();
            let close = child.text.chars().filter(|&c| c == '}').count();
            assert_eq!(open, close, "unbalanced braces in {:?}", child.heading_path);
        }
        let open = joined_parts.chars().filter(|&c| c == '{').count();
        let close = joined_parts.chars().filter(|&c| c == '}').count();
        assert_eq!(
            open, close,
            "rejoined split-symbol fragments should balance"
        );

        // Section IDs follow the code pattern
        for child in &root.children {
            assert!(
                child.id.0.starts_with("src/config.rs#"),
                "section ID should start with file path: {}",
                child.id.0
            );
        }
    }

    #[test]
    fn doc_comments_included_in_byte_range() {
        let parser = CodeParser::new();
        let source = "/// Doc comment.\npub fn documented() {}\n";
        let tree = parser.parse(Path::new("src/lib.rs"), source).unwrap();
        let root = &tree.sections[0];
        let child = &root.children[0];

        // The section text should contain the doc comment
        assert!(child.text.contains("Doc comment"));
        // And the function itself
        assert!(child.text.contains("pub fn documented"));
    }

    // cAST size-aware splitting of over-budget symbols.

    /// A function with enough statements to exceed [`CODE_CHUNK_BUDGET`].
    /// Intentionally has NO doc comment so the part texts tile the source
    /// exactly (no prepended doc on part 0).
    fn big_fn_source() -> String {
        use std::fmt::Write as _;
        let mut s = String::from("pub fn huge() {\n");
        for i in 0..120 {
            let _ = writeln!(s, "    let value_{i} = combine(value_{i}_a, value_{i}_b);");
        }
        s.push_str("}\n");
        s
    }

    #[test]
    fn oversized_symbol_splits_into_parts() {
        let src = big_fn_source();
        assert!(
            count_tokens(&src) > CODE_CHUNK_BUDGET,
            "fixture must exceed the budget to exercise splitting"
        );
        let tree = CodeParser::new()
            .parse(Path::new("src/huge.rs"), &src)
            .unwrap();
        let root = &tree.sections[0];

        assert!(
            root.children.len() >= 2,
            "huge fn should split into >=2 parts, got {}",
            root.children.len()
        );
        for child in &root.children {
            assert!(
                child.id.0.contains("src/huge.rs#huge::huge"),
                "part id keeps the symbol id as a prefix: {}",
                child.id.0
            );
            assert!(
                child.id.0.contains("#part"),
                "split id marked: {}",
                child.id.0
            );
            assert!(!child.text.trim().is_empty(), "no empty part");
            assert_eq!(child.depth, 2, "parts stay depth-2 siblings");
        }
    }

    #[test]
    fn split_tiles_the_symbol_losslessly() {
        let src = big_fn_source();
        let tree = CodeParser::new()
            .parse(Path::new("src/huge.rs"), &src)
            .unwrap();
        let root = &tree.sections[0];

        // No doc comment => part texts concatenate back to the exact symbol
        // source: every statement present once, nothing dropped or duplicated.
        let joined: String = root.children.iter().map(|c| c.text.clone()).collect();
        assert_eq!(
            joined.matches("let value_").count(),
            120,
            "every statement should survive exactly once"
        );
        assert!(joined.contains("pub fn huge()"), "signature retained");
        assert!(joined.contains("value_0_a") && joined.contains("value_119_b"));
    }

    #[test]
    fn each_part_stays_near_budget() {
        let src = big_fn_source();
        let tree = CodeParser::new()
            .parse(Path::new("src/huge.rs"), &src)
            .unwrap();
        let root = &tree.sections[0];
        // Each fragment is far below the whole; allow a little slack over the
        // budget for the per-part signature/prefix the token-proxy can't see.
        for child in &root.children {
            let tokens = count_tokens(&child.text);
            assert!(
                tokens <= CODE_CHUNK_BUDGET + 96,
                "part should stay near budget, got {tokens} tokens"
            );
        }
    }

    #[test]
    fn small_symbols_are_not_split() {
        let tree = CodeParser::new()
            .parse(Path::new("src/config.rs"), SAMPLE_RUST)
            .unwrap();
        let root = &tree.sections[0];
        for child in &root.children {
            assert!(
                !child.id.0.contains("#part"),
                "a budget-fitting symbol must stay whole: {}",
                child.id.0
            );
        }
    }

    // C7: Multi-language integration tests

    #[cfg(feature = "lang-python")]
    #[test]
    fn parse_python_source() {
        let parser = CodeParser::new();
        let source = "def hello(name: str) -> str:\n    return f'Hello {name}'\n\nclass Greeter:\n    def greet(self):\n        pass\n";
        let tree = parser.parse(Path::new("hello.py"), source).unwrap();
        let root = &tree.sections[0];

        assert!(
            !root.children.is_empty(),
            "Python source should produce symbol sections"
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn parse_typescript_source() {
        let parser = CodeParser::new();
        let source = "export function hello(name: string): string {\n  return `Hello ${name}`;\n}\n\nexport interface Greeter {\n  greet(): string;\n}\n";
        let tree = parser.parse(Path::new("hello.ts"), source).unwrap();
        let root = &tree.sections[0];

        assert!(
            !root.children.is_empty(),
            "TypeScript source should produce symbol sections"
        );
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn parse_go_source() {
        let parser = CodeParser::new();
        let source = "package main\n\nfunc Hello(name string) string {\n\treturn \"Hello \" + name\n}\n\ntype Greeter struct {\n\tName string\n}\n";
        let tree = parser.parse(Path::new("main.go"), source).unwrap();
        let root = &tree.sections[0];

        assert!(
            !root.children.is_empty(),
            "Go source should produce symbol sections"
        );
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn parse_java_source() {
        let parser = CodeParser::new();
        let source = "public class Greeter {\n    public String greet(String name) {\n        return \"Hello \" + name;\n    }\n}\n";
        let tree = parser.parse(Path::new("Greeter.java"), source).unwrap();
        let root = &tree.sections[0];

        assert!(
            !root.children.is_empty(),
            "Java source should produce symbol sections"
        );
    }

    // === C / C++ integration tests ===
    //
    // These mirror the parse_python_source / parse_typescript_source style
    // and assert that real-world C/C++ shapes (header forward decls, in-class
    // methods, namespaces, templates, out-of-class definitions, unions)
    // produce non-empty symbol sections at the parser/code.rs level.

    #[cfg(feature = "lang-c")]
    #[test]
    fn parse_c_source() {
        let parser = CodeParser::new();
        let source = "// Greet someone.\nint hello(const char *name) {\n    return 0;\n}\n\nstruct Greeter {\n    char *name;\n};\n";
        let tree = parser.parse(Path::new("hello.c"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("hello")),
            "expected hello fn section, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("Greeter")),
            "expected Greeter struct section, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn parse_c_header_declarations() {
        // Header file: only forward declarations + struct decl.
        // Without `declaration` handling, this produces zero function symbols.
        let parser = CodeParser::new();
        let source = "#ifndef HELLO_H\n#define HELLO_H\n\nstruct Greeter;\n\nint hello(const char *name);\nvoid farewell(void);\n\n#endif\n";
        let tree = parser.parse(Path::new("hello.h"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("hello")),
            "expected hello declaration, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("farewell")),
            "expected farewell declaration, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn parse_c_union() {
        let parser = CodeParser::new();
        let source = "union Tagged {\n    int i;\n    float f;\n};\n";
        let tree = parser.parse(Path::new("tagged.c"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("Tagged")),
            "expected Tagged union section, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_source() {
        let parser = CodeParser::new();
        let source = "// A greeting.\nint hello(const char *name) {\n    return 0;\n}\n\nclass Greeter {\npublic:\n    void greet();\n};\n";
        let tree = parser.parse(Path::new("hello.cpp"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("hello")),
            "expected hello fn section, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("Greeter")),
            "expected Greeter class section, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_class_methods() {
        // In-class method declarations are field_declaration nodes.
        // Must extract `greet` and `farewell` as nested members of Greeter.
        let parser = CodeParser::new();
        let source = "class Greeter {\npublic:\n    void greet(const char *name);\n    int farewell();\nprivate:\n    int counter;\n};\n";
        let tree = parser.parse(Path::new("greeter.hpp"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("Greeter")),
            "expected Greeter class section, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("greet")),
            "expected greet method section, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("farewell")),
            "expected farewell method section, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_namespace_members() {
        let parser = CodeParser::new();
        let source = "namespace mylib {\n\nint compute(int x) { return x * 2; }\n\nclass Helper {\npublic:\n    void run();\n};\n\n}\n";
        let tree = parser.parse(Path::new("mylib.cpp"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("mylib")),
            "expected mylib namespace, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("compute")),
            "expected compute fn inside namespace, got: {names:?}"
        );
        assert!(
            names.iter().any(|h| h.contains("Helper")),
            "expected Helper class inside namespace, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_out_of_class_definition() {
        // void Foo::bar() {} should preserve the Foo:: qualifier in the symbol name.
        let parser = CodeParser::new();
        let source =
            "class Foo {\npublic:\n    void bar();\n};\n\nvoid Foo::bar() {\n    // body\n}\n";
        let tree = parser.parse(Path::new("foo.cpp"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("Foo::bar")),
            "expected qualified Foo::bar definition, got: {names:?}"
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_function_template() {
        let parser = CodeParser::new();
        let source = "template <typename T>\nT max(T a, T b) {\n    return a > b ? a : b;\n}\n";
        let tree = parser.parse(Path::new("max.hpp"), source).unwrap();
        let root = &tree.sections[0];

        let max_section = root
            .children
            .iter()
            .find(|c| c.heading_path.last().is_some_and(|h| h.contains("max")))
            .unwrap_or_else(|| {
                let names: Vec<_> = root.children.iter().map(|c| &c.heading_path).collect();
                panic!("expected max template fn, got: {names:?}");
            });
        assert!(
            max_section
                .heading_path
                .last()
                .unwrap()
                .contains("function"),
            "function template should be classified as function, got heading: {:?}",
            max_section.heading_path
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn parse_cpp_alias_template() {
        let parser = CodeParser::new();
        let source = "template <typename T>\nusing Vec = std::vector<T>;\n";
        let tree = parser.parse(Path::new("alias.hpp"), source).unwrap();
        let root = &tree.sections[0];

        let names: Vec<&str> = root
            .children
            .iter()
            .filter_map(|c| c.heading_path.last().map(String::as_str))
            .collect();
        assert!(
            names.iter().any(|h| h.contains("Vec")),
            "expected Vec alias template, got: {names:?}"
        );
    }

    #[test]
    fn parse_unknown_extension_produces_fallback() {
        let parser = CodeParser::new();
        let source = "some content in an unknown language";
        // `.asm` is routed to the Code parser but has no tree-sitter
        // grammar registered, so it must hit the text fallback path.
        let tree = parser.parse(Path::new("file.asm"), source).unwrap();

        // Fallback: single section with full content, no symbol children
        assert_eq!(tree.sections.len(), 1);
        assert!(tree.sections[0].text.contains("some content"));
    }
}
