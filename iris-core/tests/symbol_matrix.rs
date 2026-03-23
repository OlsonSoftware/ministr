//! Cross-language test matrix for symbol extraction quality.
//!
//! Validates that each supported language grammar produces the expected
//! symbols when processed through `generic_extract_symbols` (or
//! `extract_symbols` for Rust). Each language has a fixture file under
//! `tests/fixtures/symbols/` with representative constructs.

use iris_core::code::{
    AstParser, GrammarRegistry, ItemKind, Visibility, extract_symbols, generic_extract_symbols,
};

/// Minimal expected-symbol descriptor for assertions.
#[derive(Debug)]
struct Expected {
    name: &'static str,
    kind: ItemKind,
}

/// Parse a fixture file and extract symbols for a given language.
fn extract_fixture(lang: &str, ext: &str) -> Vec<iris_core::code::Symbol> {
    let fixture_path = format!("tests/fixtures/symbols/sample.{ext}");
    let source = std::fs::read(&fixture_path)
        .unwrap_or_else(|e| panic!("cannot read fixture {fixture_path}: {e}"));

    let registry = GrammarRegistry::global();

    if lang == "rust" {
        let mut parser = AstParser::new();
        let tree = parser.parse(&source).unwrap();
        extract_symbols(&tree, &source, &fixture_path, &[])
    } else {
        let ts_lang = registry
            .language_for_extension(ext)
            .unwrap_or_else(|| panic!("no grammar for extension .{ext}"));
        let mut parser =
            AstParser::with_language(ts_lang).unwrap_or_else(|e| panic!("parser init: {e}"));
        let tree = parser.parse(&source).unwrap();
        generic_extract_symbols(&tree, &source, &fixture_path, &[])
    }
}

/// Assert that all expected symbols are found (by name and kind).
/// Extra symbols are allowed — we only check that expected ones are present.
fn assert_has_symbols(lang: &str, symbols: &[iris_core::code::Symbol], expected: &[Expected]) {
    for exp in expected {
        let found = symbols
            .iter()
            .any(|s| s.name == exp.name && s.kind == exp.kind);
        assert!(
            found,
            "[{lang}] expected symbol `{}` ({:?}) not found.\nActual symbols: {:#?}",
            exp.name,
            exp.kind,
            symbols
                .iter()
                .map(|s| format!("{} ({:?})", s.name, s.kind))
                .collect::<Vec<_>>()
        );
    }
}

// ── Rust ──────────────────────────────────────────────────────────────

#[test]
fn rust_symbols() {
    let symbols = extract_fixture("rust", "rs");
    assert_has_symbols(
        "rust",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct,
            },
            Expected {
                name: "Color",
                kind: ItemKind::Enum,
            },
            Expected {
                name: "Serialize",
                kind: ItemKind::Trait,
            },
            Expected {
                name: "MAX_RETRIES",
                kind: ItemKind::Const,
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Impl,
            },
            Expected {
                name: "new",
                kind: ItemKind::Function,
            },
        ],
    );

    // Check visibility on specific symbols
    let greet = symbols.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(greet.visibility, Visibility::Public);
    assert!(greet.doc_comment.is_some());

    let max = symbols.iter().find(|s| s.name == "MAX_RETRIES").unwrap();
    assert_eq!(max.visibility, Visibility::Public);
}

// ── Python ────────────────────────────────────────────────────────────

#[test]
fn python_symbols() {
    let symbols = extract_fixture("python", "py");
    assert_has_symbols(
        "python",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
        ],
    );
    // Verify at least 2 symbols extracted
    assert!(
        symbols.len() >= 2,
        "[python] expected at least 2 symbols, got {}",
        symbols.len()
    );
}

// ── JavaScript ────────────────────────────────────────────────────────

#[test]
fn javascript_symbols() {
    let symbols = extract_fixture("javascript", "js");
    assert_has_symbols(
        "javascript",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
            // Note: JS `const` via lexical_declaration doesn't extract the
            // variable name through the generic extractor — name is nested
            // inside a variable_declarator child node.
        ],
    );
}

// ── TypeScript ────────────────────────────────────────────────────────

#[test]
fn typescript_symbols() {
    let symbols = extract_fixture("typescript", "ts");
    assert_has_symbols(
        "typescript",
        &symbols,
        &[
            Expected {
                name: "Serializable",
                kind: ItemKind::Trait, // interface → Trait
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
        ],
    );
}

// ── TSX ───────────────────────────────────────────────────────────────

#[test]
fn tsx_symbols() {
    let symbols = extract_fixture("tsx", "tsx");
    assert_has_symbols(
        "tsx",
        &symbols,
        &[
            Expected {
                name: "GreetingProps",
                kind: ItemKind::Trait, // interface → Trait
            },
            Expected {
                name: "Greeting",
                kind: ItemKind::Function,
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
        ],
    );
}

// ── Go ────────────────────────────────────────────────────────────────

#[test]
fn go_symbols() {
    let symbols = extract_fixture("go", "go");
    assert_has_symbols(
        "go",
        &symbols,
        &[
            Expected {
                name: "Greet",
                kind: ItemKind::Function,
            },
            Expected {
                name: "NewAppConfig",
                kind: ItemKind::Function,
            },
        ],
    );
    // Go should extract at least the functions + type declarations
    assert!(
        symbols.len() >= 2,
        "[go] expected at least 2 symbols, got {}",
        symbols.len()
    );
}

// ── Java ──────────────────────────────────────────────────────────────

#[test]
fn java_symbols() {
    let symbols = extract_fixture("java", "java");
    assert_has_symbols(
        "java",
        &symbols,
        &[
            Expected {
                name: "Serializable",
                kind: ItemKind::Trait, // interface → Trait
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "Color",
                kind: ItemKind::Enum,
            },
        ],
    );
}

// ── C ─────────────────────────────────────────────────────────────────

#[test]
fn c_symbols() {
    let symbols = extract_fixture("c", "c");
    assert_has_symbols(
        "c",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct,
            },
            Expected {
                name: "Color",
                kind: ItemKind::Enum,
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
            Expected {
                name: "new_config",
                kind: ItemKind::Function,
            },
        ],
    );
}

// ── C++ ───────────────────────────────────────────────────────────────

#[test]
fn cpp_symbols() {
    let symbols = extract_fixture("cpp", "cpp");
    assert_has_symbols(
        "cpp",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "Color",
                kind: ItemKind::Enum,
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
        ],
    );
}

// ── Ruby ──────────────────────────────────────────────────────────────

#[test]
fn ruby_symbols() {
    let symbols = extract_fixture("ruby", "rb");
    assert_has_symbols(
        "ruby",
        &symbols,
        &[
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
        ],
    );
}

// ── C# ────────────────────────────────────────────────────────────────

#[test]
fn csharp_symbols() {
    let symbols = extract_fixture("csharp", "cs");
    assert_has_symbols(
        "csharp",
        &symbols,
        &[
            Expected {
                name: "ISerializable",
                kind: ItemKind::Trait, // interface → Trait
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "Color",
                kind: ItemKind::Enum,
            },
        ],
    );
}

// ── Swift ─────────────────────────────────────────────────────────────

#[test]
fn swift_symbols() {
    let symbols = extract_fixture("swift", "swift");
    assert_has_symbols(
        "swift",
        &symbols,
        &[
            // Note: Swift `protocol` uses `protocol_declaration` node kind
            // which doesn't match the generic extractor's interface patterns.
            // The Swift refinement handles this, but the generic path doesn't.
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct, // class → Struct
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
            Expected {
                name: "Color",
                kind: ItemKind::Struct, // Swift enum_declaration → Struct via class heuristic
            },
        ],
    );
}

// ── Kotlin ────────────────────────────────────────────────────────────

#[test]
fn kotlin_symbols() {
    let symbols = extract_fixture("kotlin", "kt");
    assert_has_symbols(
        "kotlin",
        &symbols,
        &[
            // Kotlin tree-sitter uses `class_declaration` for interfaces too,
            // so the generic extractor maps them to Struct.
            Expected {
                name: "Serializable",
                kind: ItemKind::Struct,
            },
            Expected {
                name: "AppConfig",
                kind: ItemKind::Struct,
            },
            Expected {
                name: "greet",
                kind: ItemKind::Function,
            },
            Expected {
                name: "Color",
                kind: ItemKind::Struct, // enum class → class_declaration → Struct
            },
        ],
    );
}

// ── Cross-cutting: all grammars register and parse ────────────────────

#[test]
fn all_grammars_parse_without_errors() {
    let fixtures: &[(&str, &str)] = &[
        ("rust", "rs"),
        ("python", "py"),
        ("javascript", "js"),
        ("typescript", "ts"),
        ("tsx", "tsx"),
        ("go", "go"),
        ("java", "java"),
        ("c", "c"),
        ("cpp", "cpp"),
        ("ruby", "rb"),
        ("csharp", "cs"),
        ("swift", "swift"),
        ("kotlin", "kt"),
    ];

    for (lang, ext) in fixtures {
        let symbols = extract_fixture(lang, ext);
        assert!(
            !symbols.is_empty(),
            "[{lang}] expected at least 1 symbol from fixture, got 0"
        );
    }
}

#[test]
fn grammar_registry_has_all_expected_languages() {
    let registry = GrammarRegistry::global();
    let expected = [
        "rust",
        "python",
        "javascript",
        "typescript",
        "tsx",
        "go",
        "java",
        "c",
        "cpp",
        "ruby",
        "csharp",
        "swift",
        "kotlin",
    ];

    for lang in &expected {
        assert!(
            registry.language_by_name(lang).is_some(),
            "grammar registry missing language: {lang}"
        );
    }

    assert!(
        registry.language_count() >= 13,
        "expected at least 13 grammars, got {}",
        registry.language_count()
    );
}
