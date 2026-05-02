//! Logos-driven C++ symbol fallback for files that the tree-sitter
//! parser couldn't handle.
//!
//! `tree-sitter-unreal-cpp` parses well but it's still tree-sitter:
//! pathologically deep templates (recursive `Foo<N-1>` typedef chains,
//! Slate widget headers, expression-template metaprograms) can hit the
//! 5-second per-file budget added in Phase 4. When that fires, the
//! tree-sitter path returns an error and the caller previously dropped
//! the whole file into `failed_files` with zero symbols indexed.
//!
//! This extractor is the safety net. It runs a fast Logos tokenizer
//! over the raw source and a brace-depth walker that recognises
//! top-level shapes:
//!
//! * `class Foo`, `struct Foo`, `enum Foo`, `enum class Foo`,
//!   `union Foo` → coarse type symbols
//! * `template<...> class Foo` / `template<...> struct Foo`
//! * `namespace Foo { ... }` (used as scope context for inner symbols)
//! * `<ret-type> name(...) { ... }` and `<ret-type> name(...);` as
//!   functions
//! * Unreal reflection macros (`UCLASS()`, `USTRUCT()`, `UFUNCTION()`,
//!   `UPROPERTY()`) so we still flag UE-specific symbols even when the
//!   AST didn't materialise
//!
//! The output is a `Vec<SymbolRecord>` ready to insert into the symbol
//! table — no tree, no refs, no bridge endpoints. Marking the
//! extraction as "degraded" is left to the caller.

use logos::Logos;

use crate::storage::SymbolRecord;
use crate::types::SymbolId;

/// Logos token classes for C++ source.
///
/// Whitespace and comments are skipped at the lexer level so the
/// caller can walk meaningful tokens without filtering.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
enum Token {
    #[token("class")]
    KwClass,
    #[token("struct")]
    KwStruct,
    #[token("union")]
    KwUnion,
    #[token("enum")]
    KwEnum,
    #[token("namespace")]
    KwNamespace,
    #[token("template")]
    KwTemplate,
    #[token("typedef")]
    KwTypedef,
    #[token("using")]
    KwUsing,
    #[token("public")]
    KwPublic,
    #[token("private")]
    KwPrivate,
    #[token("protected")]
    KwProtected,
    #[token("const")]
    KwConst,
    #[token("static")]
    KwStatic,
    #[token("virtual")]
    KwVirtual,
    #[token("override")]
    KwOverride,
    #[token("final")]
    KwFinal,
    #[token("inline")]
    KwInline,
    #[token("constexpr")]
    KwConstexpr,
    #[token("noexcept")]
    KwNoexcept,
    #[token("explicit")]
    KwExplicit,
    #[token("operator")]
    KwOperator,

    /// Preprocessor line (`#define`, `#include`, `#if`, `#pragma`,
    /// continuation lines via `\` are NOT joined — Logos sees one
    /// `\\\n` continuation as `\` + newline and the regex stops at
    /// `\n`. That's fine for fallback symbol extraction.
    #[regex(r"#[^\n]*")]
    Preproc,

    /// String literal — recognised so quotes don't break the
    /// identifier stream.
    #[regex(r#""([^"\\]|\\.)*""#)]
    StringLit,

    /// Char literal.
    #[regex(r"'([^'\\]|\\.)*'")]
    CharLit,

    /// Identifier — covers types, names, and keywords we don't model.
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,

    /// Numeric literal — kept out of the identifier stream.
    #[regex(r"[0-9][0-9a-zA-Z._']*")]
    Number,

    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("<")]
    LAngle,
    #[token(">")]
    RAngle,
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token("::")]
    Scope,
    #[token(",")]
    Comma,
    #[token("=")]
    Eq,
    #[token("*")]
    Star,
    #[token("&")]
    Amp,

    /// Anything else — single byte we don't model.
    #[regex(r".", priority = 0)]
    Other,
}

/// Unreal reflection macros that introduce a "top-level" UE-specific
/// declaration. Each fires a separate symbol (in addition to the
/// class/function it decorates) so a `ministr_symbols(query="UCLASS")`
/// search returns them.
const UE_MACROS: &[&str] = &[
    "UCLASS",
    "USTRUCT",
    "UENUM",
    "UINTERFACE",
    "UFUNCTION",
    "UPROPERTY",
    "UPARAM",
    "UDELEGATE",
    "GENERATED_BODY",
    "GENERATED_UCLASS_BODY",
    "GENERATED_USTRUCT_BODY",
    "GENERATED_IINTERFACE_BODY",
];

/// Output of fallback extraction.
#[derive(Debug, Clone, Default)]
pub struct CppFallbackExtraction {
    /// Recovered top-level symbols.
    pub symbols: Vec<SymbolRecord>,
}

/// Extract symbols from a C/C++ source file using the Logos fallback.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extract_cpp_fallback_symbols(content: &str, relative_path: &str) -> CppFallbackExtraction {
    let module_path = module_path_from_relative(relative_path);

    // Lex once, keeping byte spans so we can rebuild signatures and
    // line numbers without re-scanning the source.
    let mut lexer = Token::lexer(content);
    let mut toks: Vec<(Token, std::ops::Range<usize>)> = Vec::new();
    while let Some(tok) = lexer.next() {
        if let Ok(t) = tok {
            toks.push((t, lexer.span()));
        }
    }

    let bytes = content.as_bytes();
    let mut out = CppFallbackExtraction::default();

    // namespace_stack tracks the active `namespace Foo { ... }` scope
    // chain so symbol IDs and module_path strings reflect it.
    let mut namespace_stack: Vec<String> = Vec::new();
    // brace_stack[i] = tag for the i-th open brace (matched against
    // `depth` so we can pop the right namespace when its `}` arrives).
    let mut brace_stack: Vec<BraceTag> = Vec::new();
    let mut depth: i32 = 0;

    let mut i = 0usize;
    while i < toks.len() {
        let (tok, ref span) = toks[i];

        match tok {
            Token::LBrace => {
                brace_stack.push(BraceTag::Other);
                depth += 1;
                i += 1;
                continue;
            }
            Token::RBrace => {
                if let Some(BraceTag::Namespace) = brace_stack.pop() {
                    namespace_stack.pop();
                }
                depth -= 1;
                if depth < 0 {
                    depth = 0;
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        // Beyond the top namespace level we don't try to recover
        // members — too easy to false-positive. Top-level + immediate
        // namespace children are where the bulk of meaningful symbols
        // are anyway.
        if usize::try_from(depth).is_ok_and(|d| d > namespace_stack.len()) {
            i += 1;
            continue;
        }

        match tok {
            Token::KwNamespace => {
                if let Some((name, _, lbrace_idx)) = consume_namespace_header(&toks, i + 1, content)
                {
                    namespace_stack.push(name);
                    // Skip to the namespace's opening `{` and push a
                    // `Namespace` tag so its matching `}` pops the
                    // namespace_stack frame.
                    i = lbrace_idx;
                    if matches!(toks.get(i).map(|t| t.0), Some(Token::LBrace)) {
                        depth += 1;
                        brace_stack.push(BraceTag::Namespace);
                        i += 1;
                    }
                    continue;
                }
                i += 1;
            }
            Token::KwClass | Token::KwStruct | Token::KwUnion => {
                let kw_text = &content[span.clone()];
                let kw_start = span.start;
                if let Some((name, name_span, end)) =
                    consume_class_like(&toks, i + 1, content, kw_text)
                {
                    let kind = match kw_text {
                        "union" => "union",
                        "struct" => "struct",
                        _ => "class",
                    };
                    out.symbols.push(make_record(
                        relative_path,
                        &qualified_module(&namespace_stack, &module_path),
                        &name,
                        kind,
                        signature_for_block(content, kw_start, name_span.end, kind),
                        line_for(bytes, kw_start),
                        line_for(bytes, end),
                    ));
                    i = advance_past_byte(&toks, end);
                    continue;
                }
                i += 1;
            }
            Token::KwEnum => {
                if let Some((name, name_span, end)) = consume_enum_header(&toks, i + 1, content) {
                    out.symbols.push(make_record(
                        relative_path,
                        &qualified_module(&namespace_stack, &module_path),
                        &name,
                        "enum",
                        signature_for_block(content, span.start, name_span.end, "enum"),
                        line_for(bytes, span.start),
                        line_for(bytes, end),
                    ));
                    i = advance_past_byte(&toks, end);
                    continue;
                }
                i += 1;
            }
            Token::Ident => {
                let name = &content[span.clone()];
                if UE_MACROS.contains(&name)
                    && let Some(end) = consume_macro_call(&toks, i + 1)
                {
                    out.symbols.push(make_record(
                        relative_path,
                        &qualified_module(&namespace_stack, &module_path),
                        name,
                        "macro",
                        content[span.start..end.min(content.len())]
                            .trim()
                            .to_string(),
                        line_for(bytes, span.start),
                        line_for(bytes, end),
                    ));
                    i = advance_past_byte(&toks, end);
                    continue;
                }
                if let Some((fn_name, fn_name_span, end)) = try_parse_function(&toks, i, content) {
                    out.symbols.push(make_record(
                        relative_path,
                        &qualified_module(&namespace_stack, &module_path),
                        &fn_name,
                        "function",
                        signature_for_function(content, span.start, fn_name_span.end),
                        line_for(bytes, span.start),
                        line_for(bytes, end),
                    ));
                    i = advance_past_byte(&toks, end);
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    out
}

#[derive(Debug, Clone, Copy)]
enum BraceTag {
    Namespace,
    Other,
}

/// `class Foo : public Bar { ... }` → `Some((name, name_span,
/// byte_offset_after_closing_brace))`. Bails on forward declarations
/// (`class Foo;`) since the brace walker has nothing to consume.
fn consume_class_like(
    toks: &[(Token, std::ops::Range<usize>)],
    mut idx: usize,
    content: &str,
    _kw: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    // Optional `final` / `MYPROJECT_API` / attribute spam between
    // `class` and the name. The LAST identifier seen before
    // `{` / `:` / `;` / `<` is the class name — earlier ones are
    // API-export decorations like `MYPROJECT_API`.
    let mut name: Option<(String, std::ops::Range<usize>)> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident => {
                let s = toks[idx].1.clone();
                let txt = content[s.clone()].to_string();
                name = Some((txt, s));
                idx += 1;
            }
            Token::LAngle => {
                let close = match_close_angle(toks, idx)?;
                idx = close;
            }
            Token::Colon => {
                // Inheritance list — skip until brace or semi.
                idx += 1;
                while idx < toks.len() && !matches!(toks[idx].0, Token::LBrace | Token::Semi) {
                    idx += 1;
                }
            }
            Token::LBrace => {
                let close = match_close_brace(toks, idx)?;
                let name = name?;
                return Some((name.0, name.1, close));
            }
            Token::Semi => return None,
            _ => idx += 1,
        }
    }
    None
}

/// `enum Foo`, `enum class Foo`, optional `: int` underlying type,
/// either `{ ... }` or `;`. Returns the name span + the end byte
/// offset after `}` or `;`.
fn consume_enum_header(
    toks: &[(Token, std::ops::Range<usize>)],
    mut idx: usize,
    content: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    // Optional `class` / `struct` (scoped enum).
    if matches!(
        toks.get(idx).map(|t| t.0),
        Some(Token::KwClass | Token::KwStruct)
    ) {
        idx += 1;
    }
    // Skip API decorations to the name (latest identifier wins).
    let mut name: Option<(String, std::ops::Range<usize>)> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident => {
                let s = toks[idx].1.clone();
                let txt = content[s.clone()].to_string();
                name = Some((txt, s));
                idx += 1;
            }
            Token::Colon => {
                idx += 1;
                while idx < toks.len() && !matches!(toks[idx].0, Token::LBrace | Token::Semi) {
                    idx += 1;
                }
            }
            Token::Semi => {
                return name.map(|(n, s)| (n, s, toks[idx].1.end));
            }
            Token::LBrace => {
                let close = match_close_brace(toks, idx)?;
                let name = name?;
                return Some((name.0, name.1, close));
            }
            _ => idx += 1,
        }
    }
    None
}

/// `namespace Foo { ... }` or `namespace Foo::Bar { ... }`. Returns
/// the deepest name (we don't try to model multi-segment namespaces
/// distinctly — symbol scoping just uses the trailing segment), the
/// span of that name, and the index of the opening `{`.
fn consume_namespace_header(
    toks: &[(Token, std::ops::Range<usize>)],
    mut idx: usize,
    content: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    let mut name: Option<(String, std::ops::Range<usize>)> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident => {
                let s = toks[idx].1.clone();
                name = Some((content[s.clone()].to_string(), s));
                idx += 1;
            }
            Token::LBrace => {
                let n = name?;
                return Some((n.0, n.1, idx));
            }
            Token::Semi | Token::Eq => return None,
            _ => idx += 1,
        }
    }
    None
}

/// `MACRO( ... )` — used for UE reflection macros so we record them
/// even when the underlying class/function lookup fails. Returns the
/// byte offset just after the closing `)`.
fn consume_macro_call(toks: &[(Token, std::ops::Range<usize>)], idx: usize) -> Option<usize> {
    if !matches!(toks.get(idx).map(|t| t.0), Some(Token::LParen)) {
        return None;
    }
    let mut depth = 0i32;
    let mut j = idx;
    while j < toks.len() {
        match toks[j].0 {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(toks[j].1.end);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// `<ret-type-tokens> <name> ( <params> ) <const-noexcept-override?> { <body> }`
/// or trailing `;` (declaration only). We accept the trailing `;`
/// form too — declarations are still useful symbols.
fn try_parse_function(
    toks: &[(Token, std::ops::Range<usize>)],
    start: usize,
    content: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    let mut idx = start;
    let mut last_ident: Option<usize> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident => {
                last_ident = Some(idx);
                idx += 1;
            }
            Token::LAngle => {
                let close = match_close_angle(toks, idx)?;
                idx = close;
            }
            Token::LParen => {
                let name_idx = last_ident?;
                if name_idx + 1 != idx {
                    return None;
                }
                let close_paren = match_close_paren(toks, idx)?;
                let mut after = close_paren;
                // Skip qualifiers / trailing-return-type / member-init
                // until we hit `{` or `;`.
                while after < toks.len() && !matches!(toks[after].0, Token::LBrace | Token::Semi) {
                    after += 1;
                }
                if after >= toks.len() {
                    return None;
                }
                let name_span = toks[name_idx].1.clone();
                let name = content[name_span.clone()].to_string();
                if matches!(toks[after].0, Token::Semi) {
                    return Some((name, name_span, toks[after].1.end));
                }
                let close_brace = match_close_brace(toks, after)?;
                return Some((name, name_span, close_brace));
            }
            Token::Semi | Token::LBrace | Token::RBrace | Token::Comma | Token::Eq => return None,
            _ => idx += 1,
        }
    }
    None
}

fn match_close_brace(toks: &[(Token, std::ops::Range<usize>)], lbrace_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut idx = lbrace_idx;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::LBrace => depth += 1,
            Token::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(toks[idx].1.end);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn match_close_paren(toks: &[(Token, std::ops::Range<usize>)], lparen_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut idx = lparen_idx;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn match_close_angle(toks: &[(Token, std::ops::Range<usize>)], langle_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut idx = langle_idx;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::LAngle => depth += 1,
            Token::RAngle => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            Token::LBrace | Token::Semi => return None,
            _ => {}
        }
        idx += 1;
    }
    None
}

fn advance_past_byte(toks: &[(Token, std::ops::Range<usize>)], byte_end: usize) -> usize {
    toks.iter()
        .position(|(_, span)| span.start >= byte_end)
        .unwrap_or(toks.len())
}

fn line_for(bytes: &[u8], byte_offset: usize) -> u32 {
    let upto = byte_offset.min(bytes.len());
    #[allow(clippy::cast_possible_truncation, clippy::naive_bytecount)]
    let n = bytes[..upto].iter().filter(|&&b| b == b'\n').count() as u32;
    n + 1
}

fn signature_for_block(content: &str, start: usize, name_end: usize, kind: &str) -> String {
    let snippet = content[start..name_end.min(content.len())].trim();
    if snippet.is_empty() {
        format!("{kind} <unknown>")
    } else {
        snippet.to_string()
    }
}

fn signature_for_function(content: &str, start: usize, name_end: usize) -> String {
    let end = name_end.min(content.len());
    let head: String = content[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{head}(...)")
}

fn module_path_from_relative(relative_path: &str) -> String {
    let p = std::path::Path::new(relative_path);
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("translation_unit")
        .to_string()
}

fn qualified_module(namespace_stack: &[String], file_module: &str) -> String {
    if namespace_stack.is_empty() {
        file_module.to_string()
    } else if file_module.is_empty() {
        namespace_stack.join("::")
    } else {
        format!("{file_module}::{}", namespace_stack.join("::"))
    }
}

fn make_record(
    relative_path: &str,
    module_path: &str,
    name: &str,
    kind: &str,
    signature: String,
    line_start: u32,
    line_end: u32,
) -> SymbolRecord {
    let id = if module_path.is_empty() {
        format!("sym-{relative_path}::{name}")
    } else {
        format!("sym-{relative_path}::{module_path}::{name}")
    };
    SymbolRecord {
        id: SymbolId(id),
        file_path: relative_path.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        visibility: "public".to_string(),
        signature,
        doc_comment: None,
        module_path: module_path.to_string(),
        line_start,
        line_end,
        cyclomatic_complexity: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(extraction: &CppFallbackExtraction) -> Vec<&str> {
        extraction.symbols.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn extracts_top_level_class_and_struct() {
        let src = r"
class Foo {
public:
    int x;
};

struct Bar {
    float y;
};
";
        let out = extract_cpp_fallback_symbols(src, "Test.h");
        let n = names(&out);
        assert!(n.contains(&"Foo"), "{n:?}");
        assert!(n.contains(&"Bar"), "{n:?}");
        let foo = out.symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.kind, "class");
        let bar = out.symbols.iter().find(|s| s.name == "Bar").unwrap();
        assert_eq!(bar.kind, "struct");
    }

    #[test]
    fn extracts_enum_and_enum_class() {
        let src = r"
enum Color { Red, Green, Blue };
enum class Direction : int { Up, Down };
enum Shape;  // forward decl
";
        let out = extract_cpp_fallback_symbols(src, "Enums.h");
        let n = names(&out);
        assert!(n.contains(&"Color"), "{n:?}");
        assert!(n.contains(&"Direction"), "{n:?}");
        assert!(n.contains(&"Shape"), "{n:?}");
    }

    #[test]
    fn extracts_top_level_function_definition_and_declaration() {
        let src = r"
int compute(int x, int y) {
    return x + y;
}

void announce(const char* msg);
";
        let out = extract_cpp_fallback_symbols(src, "Free.cpp");
        let n = names(&out);
        assert!(n.contains(&"compute"), "{n:?}");
        assert!(n.contains(&"announce"), "{n:?}");
        for sym in &out.symbols {
            assert_eq!(sym.kind, "function");
        }
    }

    #[test]
    fn unreal_reflection_macros_are_recorded() {
        let src = r"
UCLASS(Blueprintable)
class MYGAME_API AMyActor : public AActor {
    GENERATED_BODY()
public:
    UFUNCTION(BlueprintCallable)
    void DoThing();

    UPROPERTY(EditAnywhere)
    int32 Counter;
};
";
        let out = extract_cpp_fallback_symbols(src, "MyActor.h");
        let n = names(&out);
        assert!(n.contains(&"UCLASS"), "{n:?}");
        assert!(n.contains(&"AMyActor"), "{n:?}");
        // Inner-body reflection macros stay quiet under depth>top-level
        // — that's fine, the class symbol carries them implicitly.
    }

    #[test]
    fn namespace_scopes_inner_symbols() {
        let src = r"
namespace foo {
    class Bar {
    };

    int compute() { return 42; }
}
";
        let out = extract_cpp_fallback_symbols(src, "Scoped.h");
        // Inner symbols get the namespace prefix in their module path.
        let bar = out
            .symbols
            .iter()
            .find(|s| s.name == "Bar")
            .expect("Bar should be recovered");
        assert!(
            bar.module_path.contains("foo"),
            "module_path={}",
            bar.module_path
        );
    }

    #[test]
    fn template_class_recognised() {
        let src = r"
template<typename T>
class Container {
    T value;
};
";
        let out = extract_cpp_fallback_symbols(src, "Container.h");
        let n = names(&out);
        assert!(n.contains(&"Container"), "{n:?}");
    }

    #[test]
    fn comments_and_strings_dont_register_phantom_symbols() {
        let src = r#"
// class Phantom { };
/* struct AlsoPhantom { }; */
const char* msg = "class StringPhantom { };";

class RealOne {};
"#;
        let out = extract_cpp_fallback_symbols(src, "Comments.cpp");
        let n = names(&out);
        assert!(n.contains(&"RealOne"), "{n:?}");
        assert!(!n.contains(&"Phantom"), "{n:?}");
        assert!(!n.contains(&"AlsoPhantom"), "{n:?}");
        assert!(!n.contains(&"StringPhantom"), "{n:?}");
    }

    #[test]
    fn nested_braces_dont_leak_inner_decls() {
        let src = r"
class Outer {
    class Inner {  // member type, not top-level
        int x;
    };
};
";
        let out = extract_cpp_fallback_symbols(src, "Nested.h");
        let n = names(&out);
        assert!(n.contains(&"Outer"), "{n:?}");
        assert!(!n.contains(&"Inner"), "{n:?}");
    }

    #[test]
    fn forward_declaration_is_skipped() {
        let src = r"
class Forward;
struct AlsoForward;

class Real { int x; };
";
        let out = extract_cpp_fallback_symbols(src, "Fwd.h");
        let n = names(&out);
        assert_eq!(n, vec!["Real"]);
    }

    #[test]
    fn empty_input_is_safe() {
        let out = extract_cpp_fallback_symbols("", "empty.cpp");
        assert!(out.symbols.is_empty());
    }
}
