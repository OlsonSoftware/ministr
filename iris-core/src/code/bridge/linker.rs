//! Two-pass bridge linker pipeline.
//!
//! The [`BridgeLinker`] orchestrates multiple [`BridgeExtractor`]s to detect
//! cross-language bridges in a multi-file project:
//!
//! 1. **Pass 1 — Extract**: For each source file, run applicable extractors
//!    to collect [`BridgeEndpoint`]s.
//! 2. **Pass 2 — Join**: Group endpoints by `(BridgeKind, binding_key)` and
//!    pair exports with imports to form [`BridgeLink`]s.

use std::collections::HashMap;

use super::{BridgeEndpoint, BridgeExtractor, BridgeKind, BridgeLink, EndpointRole};

/// A source file prepared for bridge extraction.
///
/// Contains the parsed tree-sitter AST alongside the raw source bytes,
/// file path, and detected language.
pub struct SourceFile<'a> {
    /// File path relative to the corpus root.
    pub file_path: &'a str,
    /// Canonical language name (e.g. `"rust"`, `"typescript"`).
    pub language: &'a str,
    /// Parsed tree-sitter syntax tree.
    pub tree: &'a tree_sitter::Tree,
    /// Raw source bytes.
    pub source: &'a [u8],
}

/// Two-pass pipeline that extracts bridge endpoints and joins them into links.
///
/// Register one or more [`BridgeExtractor`] implementations, then call
/// [`extract_all`](Self::extract_all) to collect endpoints and
/// [`link`](Self::link) to join them.
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::linker::BridgeLinker;
/// use iris_core::code::bridge::{BridgeEndpoint, BridgeKind, EndpointRole};
///
/// let linker = BridgeLinker::new();
/// // With no extractors, extract_all returns nothing:
/// let endpoints = linker.extract_all(&[]);
/// assert!(endpoints.is_empty());
///
/// // link() with no endpoints returns no links:
/// let links = linker.link(&[]);
/// assert!(links.is_empty());
/// ```
pub struct BridgeLinker {
    extractors: Vec<Box<dyn BridgeExtractor>>,
}

impl BridgeLinker {
    /// Create a new linker with no registered extractors.
    #[must_use]
    pub fn new() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }

    /// Register a bridge extractor.
    ///
    /// The linker will invoke this extractor for any source file whose
    /// language is in the extractor's [`applicable_languages`](BridgeExtractor::applicable_languages).
    pub fn register(&mut self, extractor: Box<dyn BridgeExtractor>) {
        self.extractors.push(extractor);
    }

    /// **Pass 1** — Extract endpoints from all source files.
    ///
    /// For each file, determines which registered extractors apply based on
    /// the file's language, and collects all returned endpoints.
    #[must_use]
    pub fn extract_all(&self, files: &[SourceFile<'_>]) -> Vec<BridgeEndpoint> {
        let mut endpoints = Vec::new();

        for file in files {
            for extractor in &self.extractors {
                if extractor.applicable_languages().contains(&file.language) {
                    let mut file_endpoints = extractor.extract_endpoints(
                        file.tree,
                        file.source,
                        file.file_path,
                        file.language,
                    );
                    endpoints.append(&mut file_endpoints);
                }
            }
        }

        endpoints
    }

    /// **Pass 2** — Join endpoints into cross-language links.
    ///
    /// Groups endpoints by `(BridgeKind, binding_key)`, then pairs each
    /// export with each import in the group. When multiple exports or imports
    /// share a key, all combinations are produced (cartesian product).
    ///
    /// Links are sorted by descending confidence for deterministic output.
    #[must_use]
    pub fn link(&self, endpoints: &[BridgeEndpoint]) -> Vec<BridgeLink> {
        type EndpointGroup<'a> = (Vec<&'a BridgeEndpoint>, Vec<&'a BridgeEndpoint>);

        // Group by (kind, binding_key)
        let mut groups: HashMap<(BridgeKind, &str), EndpointGroup<'_>> = HashMap::new();

        for ep in endpoints {
            let key = (ep.kind, ep.binding_key.as_str());
            let entry = groups
                .entry(key)
                .or_insert_with(|| (Vec::new(), Vec::new()));
            match ep.role {
                EndpointRole::Export => entry.0.push(ep),
                EndpointRole::Import => entry.1.push(ep),
            }
        }

        let mut links = Vec::new();

        for (exports, imports) in groups.values() {
            for export in exports {
                for import in imports {
                    links.push(BridgeLink::new((*export).clone(), (*import).clone()));
                }
            }
        }

        // Sort by descending confidence for deterministic, priority-ordered output
        links.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        links
    }

    /// Convenience: run both passes (extract then link) in one call.
    #[must_use]
    pub fn extract_and_link(&self, files: &[SourceFile<'_>]) -> Vec<BridgeLink> {
        let endpoints = self.extract_all(files);
        self.link(&endpoints)
    }
}

impl Default for BridgeLinker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock extractor that returns pre-configured endpoints.
    struct MockExtractor {
        kind: BridgeKind,
        languages: Vec<&'static str>,
        endpoints: Vec<BridgeEndpoint>,
    }

    impl BridgeExtractor for MockExtractor {
        fn bridge_kind(&self) -> BridgeKind {
            self.kind
        }

        fn applicable_languages(&self) -> &[&str] {
            &self.languages
        }

        fn extract_endpoints(
            &self,
            _tree: &tree_sitter::Tree,
            _source: &[u8],
            _file_path: &str,
            _language: &str,
        ) -> Vec<BridgeEndpoint> {
            self.endpoints.clone()
        }
    }

    fn make_endpoint(
        key: &str,
        kind: BridgeKind,
        role: EndpointRole,
        language: &str,
        confidence: f32,
    ) -> BridgeEndpoint {
        BridgeEndpoint {
            binding_key: key.into(),
            kind,
            role,
            language: language.into(),
            file_path: format!("src/test.{}", if language == "rust" { "rs" } else { "ts" }),
            line: 1,
            symbol_name: key.into(),
            confidence,
        }
    }

    fn dummy_tree() -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(b"fn main() {}", None).unwrap()
    }

    #[test]
    fn empty_linker_produces_no_results() {
        let linker = BridgeLinker::new();
        let endpoints = linker.extract_all(&[]);
        assert!(endpoints.is_empty());
        let links = linker.link(&[]);
        assert!(links.is_empty());
    }

    #[test]
    fn matching_endpoints_produce_link() {
        let endpoints = vec![
            make_endpoint(
                "greet",
                BridgeKind::TauriCommand,
                EndpointRole::Export,
                "rust",
                1.0,
            ),
            make_endpoint(
                "greet",
                BridgeKind::TauriCommand,
                EndpointRole::Import,
                "typescript",
                0.9,
            ),
        ];

        let linker = BridgeLinker::new();
        let links = linker.link(&endpoints);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::TauriCommand);
        assert_eq!(links[0].export.binding_key, "greet");
        assert_eq!(links[0].import.binding_key, "greet");
        assert!((links[0].confidence - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn unmatched_endpoints_produce_no_links() {
        let endpoints = vec![
            // Export only — no matching import
            make_endpoint(
                "greet",
                BridgeKind::TauriCommand,
                EndpointRole::Export,
                "rust",
                1.0,
            ),
            // Import only — no matching export
            make_endpoint(
                "save",
                BridgeKind::TauriCommand,
                EndpointRole::Import,
                "typescript",
                0.9,
            ),
        ];

        let linker = BridgeLinker::new();
        let links = linker.link(&endpoints);

        assert!(links.is_empty());
    }

    #[test]
    fn different_kinds_do_not_link() {
        let endpoints = vec![
            make_endpoint(
                "fetch",
                BridgeKind::TauriCommand,
                EndpointRole::Export,
                "rust",
                1.0,
            ),
            make_endpoint(
                "fetch",
                BridgeKind::Napi,
                EndpointRole::Import,
                "typescript",
                1.0,
            ),
        ];

        let linker = BridgeLinker::new();
        let links = linker.link(&endpoints);

        assert!(
            links.is_empty(),
            "different BridgeKinds should not be linked"
        );
    }

    #[test]
    fn multiple_imports_produce_cartesian_links() {
        let endpoints = vec![
            make_endpoint(
                "cmd",
                BridgeKind::TauriCommand,
                EndpointRole::Export,
                "rust",
                1.0,
            ),
            make_endpoint(
                "cmd",
                BridgeKind::TauriCommand,
                EndpointRole::Import,
                "typescript",
                0.9,
            ),
            make_endpoint(
                "cmd",
                BridgeKind::TauriCommand,
                EndpointRole::Import,
                "typescript",
                0.8,
            ),
        ];

        let linker = BridgeLinker::new();
        let links = linker.link(&endpoints);

        assert_eq!(links.len(), 2, "1 export × 2 imports = 2 links");
        // Sorted by descending confidence
        assert!((links[0].confidence - 0.9).abs() < f32::EPSILON);
        assert!((links[1].confidence - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn extract_all_filters_by_language() {
        let tree = dummy_tree();
        let source = b"fn main() {}";

        let extractor = MockExtractor {
            kind: BridgeKind::TauriCommand,
            languages: vec!["rust"],
            endpoints: vec![make_endpoint(
                "greet",
                BridgeKind::TauriCommand,
                EndpointRole::Export,
                "rust",
                1.0,
            )],
        };

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(extractor));

        // Rust file — should match
        let rust_file = SourceFile {
            file_path: "src/lib.rs",
            language: "rust",
            tree: &tree,
            source,
        };
        let endpoints = linker.extract_all(&[rust_file]);
        assert_eq!(endpoints.len(), 1);

        // TypeScript file — should NOT match the rust-only extractor
        let ts_file = SourceFile {
            file_path: "src/app.ts",
            language: "typescript",
            tree: &tree,
            source,
        };
        let endpoints = linker.extract_all(&[ts_file]);
        assert!(endpoints.is_empty());
    }

    #[test]
    fn extract_and_link_end_to_end() {
        let tree = dummy_tree();
        let source = b"fn main() {}";

        // Extractor that returns both an export and an import for any file
        let extractor = MockExtractor {
            kind: BridgeKind::Napi,
            languages: vec!["rust", "typescript"],
            endpoints: vec![
                make_endpoint(
                    "compute",
                    BridgeKind::Napi,
                    EndpointRole::Export,
                    "rust",
                    1.0,
                ),
                make_endpoint(
                    "compute",
                    BridgeKind::Napi,
                    EndpointRole::Import,
                    "typescript",
                    0.85,
                ),
            ],
        };

        let mut linker = BridgeLinker::new();
        linker.register(Box::new(extractor));

        let file = SourceFile {
            file_path: "src/lib.rs",
            language: "rust",
            tree: &tree,
            source,
        };

        let links = linker.extract_and_link(&[file]);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, BridgeKind::Napi);
        assert!((links[0].confidence - 0.85).abs() < f32::EPSILON);
    }

    #[test]
    fn links_sorted_by_descending_confidence() {
        let endpoints = vec![
            make_endpoint("a", BridgeKind::Ffi, EndpointRole::Export, "rust", 0.5),
            make_endpoint("a", BridgeKind::Ffi, EndpointRole::Import, "c", 0.5),
            make_endpoint("b", BridgeKind::Ffi, EndpointRole::Export, "rust", 1.0),
            make_endpoint("b", BridgeKind::Ffi, EndpointRole::Import, "c", 0.9),
        ];

        let linker = BridgeLinker::new();
        let links = linker.link(&endpoints);

        assert_eq!(links.len(), 2);
        assert!(
            links[0].confidence >= links[1].confidence,
            "links should be sorted by descending confidence"
        );
    }

    #[test]
    fn default_linker_is_empty() {
        let linker = BridgeLinker::default();
        assert!(linker.extract_all(&[]).is_empty());
    }
}
