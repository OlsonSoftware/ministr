//! Symbol extraction for HLSL / GLSL / MSL / WGSL shader source.
//!
//! Shader languages don't have an ergonomic tree-sitter grammar in the
//! Rust ecosystem (and writing one isn't on the roadmap). The shapes
//! we care about — `cbuffer`, `Texture2D`, `SamplerState`, function
//! signatures, `struct`s, `#include`s — are also simple enough that a
//! Logos tokenizer plus a brace-depth walk catches them in a few
//! hundred lines.
//!
//! The output mirrors the tree-sitter path: a `Vec<SymbolRecord>`
//! plus a list of include targets, which the caller wires into the
//! same SQL tables the rest of the indexer uses.
//!
//! Coverage today: HLSL idioms (which dominate the Unreal Engine
//! shader corpus). GLSL / MSL / WGSL files share enough syntactic
//! ground that the same extractor catches their basic shapes —
//! per-language refinements (e.g. WGSL's `var<storage>`, MSL's
//! `[[buffer(0)]]`) can layer on later.
//!
//! Out of scope here: function-body symbol references. Adding HLSL
//! refs into the `RawRef` cross-symbol resolution path is a follow-up.

use logos::Logos;

use crate::storage::SymbolRecord;
use crate::types::SymbolId;

/// File extensions routed to the HLSL extractor.
///
/// Kept in sync with the shader entries in
/// [`crate::code::grammar::ALL_CODE_EXTENSIONS`].
pub const HLSL_EXTENSIONS: &[&str] = &[
    // HLSL — Direct3D / Unreal
    "hlsl", "usf", "ush", "fx", "fxh", "shader", // GLSL — OpenGL / Vulkan
    "glsl", "vert", "frag", "geom", "comp", "tesc", "tese", "mesh", "task", "rgen", "rmiss",
    "rchit", "rahit", "rint", "rcall", // Metal Shading Language — Apple
    "metal", // WebGPU Shading Language
    "wgsl",
];

/// Returns true if the given extension routes to the HLSL extractor.
#[must_use]
pub fn is_shader_extension(ext: &str) -> bool {
    HLSL_EXTENSIONS.contains(&ext)
}

/// Logos token classes for shader source.
///
/// Whitespace and comments are skipped at the lexer level so the
/// caller can walk the meaningful tokens without filtering.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n\f]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
enum Token {
    #[token("cbuffer")]
    KwCbuffer,
    #[token("tbuffer")]
    KwTbuffer,
    #[token("struct")]
    KwStruct,
    #[token("class")]
    KwClass,
    #[token("groupshared")]
    KwGroupshared,
    #[token("static")]
    KwStatic,
    #[token("const")]
    KwConst,

    /// `#include "foo.ush"` or `#include <foo.ush>`. The captured
    /// slice includes the leading `#include`; the caller extracts the
    /// inner path.
    #[regex(r#"#\s*include\s+[<"][^>"]+[>"]"#)]
    IncludeDirective,

    /// Other preprocessor lines we ignore (`#define`, `#if`, `#pragma`).
    #[regex(r"#[a-zA-Z_]+[^\n]*")]
    OtherPreproc,

    /// HLSL attribute like `[numthreads(8,8,1)]` or `[earlydepthstencil]`.
    /// Treated as a single token so it doesn't confuse the brace walker.
    #[regex(r"\[[^\]\n]*\]")]
    Attribute,

    /// String literal — skipped semantically but recognized so the
    /// regex for `IncludeDirective` doesn't get confused by stray
    /// quotes inside other text.
    #[regex(r#""([^"\\]|\\.)*""#)]
    StringLit,

    /// Identifier — covers HLSL type prefixes (`Texture2D`,
    /// `RWStructuredBuffer`, `SamplerState`), user names, and
    /// keywords we don't care about (`float4`, `void`, etc.).
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,

    /// Numeric literal — skipped semantically, recognized so it
    /// doesn't poison the identifier stream.
    #[regex(r"[0-9]+(\.[0-9]*)?([eE][+-]?[0-9]+)?[fFhHdD]?")]
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
    #[token(",")]
    Comma,
    #[token("=")]
    Eq,

    /// Anything else — a single character we don't model.
    #[regex(r".", priority = 0)]
    Other,
}

/// Resource type prefixes that introduce a top-level resource binding.
///
/// `Texture2D Diffuse;` / `RWStructuredBuffer<float> Counters;` etc.
/// Recognising these by name keeps us out of a full type grammar.
const RESOURCE_TYPES: &[&str] = &[
    "Texture1D",
    "Texture1DArray",
    "Texture2D",
    "Texture2DArray",
    "Texture2DMS",
    "Texture2DMSArray",
    "Texture3D",
    "TextureCube",
    "TextureCubeArray",
    "RWTexture1D",
    "RWTexture1DArray",
    "RWTexture2D",
    "RWTexture2DArray",
    "RWTexture3D",
    "Buffer",
    "RWBuffer",
    "ByteAddressBuffer",
    "RWByteAddressBuffer",
    "StructuredBuffer",
    "RWStructuredBuffer",
    "AppendStructuredBuffer",
    "ConsumeStructuredBuffer",
    "SamplerState",
    "SamplerComparisonState",
    "ConstantBuffer",
    "RaytracingAccelerationStructure",
];

/// Output of HLSL extraction.
#[derive(Debug, Clone, Default)]
pub struct HlslExtraction {
    /// Top-level shader symbols (`cbuffer` / resource binding /
    /// function / `struct`).
    pub symbols: Vec<SymbolRecord>,
    /// `#include` targets — paths as written, no resolution.
    pub includes: Vec<String>,
}

/// Extract symbols + includes from a shader source file.
///
/// Mirrors the per-file inputs the tree-sitter path uses — caller
/// handles namespacing the symbol IDs and storing them.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extract_hlsl_symbols(content: &str, relative_path: &str) -> HlslExtraction {
    let module_path = module_path_from_relative(relative_path);

    // First pass: collect tokens with byte spans so we can recover
    // signatures and line numbers without re-scanning the source.
    // Lex errors are dropped: Logos returns Err for bytes that match
    // no rule under our skip set; a partial symbol list beats bailing
    // on a single weird char.
    let mut lexer = Token::lexer(content);
    let mut toks: Vec<(Token, std::ops::Range<usize>)> = Vec::new();
    while let Some(tok) = lexer.next() {
        if let Ok(t) = tok {
            toks.push((t, lexer.span()));
        }
    }

    let mut out = HlslExtraction::default();
    let bytes = content.as_bytes();
    let mut i = 0usize;
    let mut depth: i32 = 0;

    while i < toks.len() {
        let (tok, ref span) = toks[i];

        // Track brace depth so we only fire top-level rules.
        match tok {
            Token::LBrace => {
                depth += 1;
                i += 1;
                continue;
            }
            Token::RBrace => {
                depth -= 1;
                if depth < 0 {
                    depth = 0;
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        if depth != 0 {
            i += 1;
            continue;
        }

        match tok {
            Token::IncludeDirective => {
                if let Some(path) = parse_include_path(&content[span.clone()]) {
                    out.includes.push(path);
                }
                i += 1;
            }
            Token::KwCbuffer | Token::KwTbuffer => {
                let kw_span = span.clone();
                if let Some((name, name_span, body_end)) =
                    consume_named_block(&toks, i + 1, content)
                {
                    let kind = if matches!(tok, Token::KwCbuffer) {
                        "cbuffer"
                    } else {
                        "tbuffer"
                    };
                    out.symbols.push(make_record(
                        relative_path,
                        &module_path,
                        &name,
                        kind,
                        signature_for_block(content, kw_span.start, name_span.end, kind),
                        line_for(bytes, kw_span.start),
                        line_for(bytes, body_end),
                    ));
                    i = advance_past_byte(&toks, body_end);
                    continue;
                }
                i += 1;
            }
            Token::KwStruct | Token::KwClass => {
                let kw_span = span.clone();
                if let Some((name, name_span, body_end)) =
                    consume_named_block(&toks, i + 1, content)
                {
                    out.symbols.push(make_record(
                        relative_path,
                        &module_path,
                        &name,
                        "struct",
                        signature_for_block(content, kw_span.start, name_span.end, "struct"),
                        line_for(bytes, kw_span.start),
                        line_for(bytes, body_end),
                    ));
                    i = advance_past_byte(&toks, body_end);
                    continue;
                }
                i += 1;
            }
            Token::Ident => {
                let name = &content[span.clone()];
                if RESOURCE_TYPES.contains(&name)
                    && let Some((var_name, semi_end)) = consume_resource_decl(&toks, i + 1, content)
                {
                    out.symbols.push(make_record(
                        relative_path,
                        &module_path,
                        &var_name,
                        "static",
                        signature_for_resource(content, span.start, semi_end),
                        line_for(bytes, span.start),
                        line_for(bytes, semi_end),
                    ));
                    i = advance_past_byte(&toks, semi_end);
                    continue;
                }
                // Could also be a function: `<ret_type> <name> ( ... ) { ... }`.
                if let Some((fn_name, fn_name_span, body_end)) =
                    try_parse_function(&toks, i, content)
                {
                    out.symbols.push(make_record(
                        relative_path,
                        &module_path,
                        &fn_name,
                        "function",
                        signature_for_function(content, span.start, fn_name_span.end),
                        line_for(bytes, span.start),
                        line_for(bytes, body_end),
                    ));
                    i = advance_past_byte(&toks, body_end);
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    out
}

/// Look ahead for `<name> {` (skipping inheritance / packoffset
/// noise). Returns `(name, span_of_name, byte_offset_after_closing_brace)`.
fn consume_named_block(
    toks: &[(Token, std::ops::Range<usize>)],
    mut idx: usize,
    content: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    // Skip optional `: register(b0)`-style suffix until we find a
    // bare identifier (the name) and then a `{`.
    let mut name: Option<(String, std::ops::Range<usize>)> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident if name.is_none() => {
                let s = toks[idx].1.clone();
                name = Some((content[s.clone()].to_string(), s));
                idx += 1;
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

/// Resource declaration shape: `<TypeAlreadyConsumed>` `<...>?` `Name`
/// `(: register(...))?` `;`. Returns `(var_name, byte_offset_after_semi)`.
fn consume_resource_decl(
    toks: &[(Token, std::ops::Range<usize>)],
    mut idx: usize,
    content: &str,
) -> Option<(String, usize)> {
    // Optional `<...>` template params.
    if idx < toks.len() && matches!(toks[idx].0, Token::LAngle) {
        let close = match_close_angle(toks, idx)?;
        idx = close;
    }
    if idx >= toks.len() {
        return None;
    }
    let Token::Ident = toks[idx].0 else {
        return None;
    };
    let name_span = toks[idx].1.clone();
    let name = content[name_span].to_string();
    idx += 1;
    // Walk to the next `;` (allow `: register(...)` or array `[N]`).
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Semi => return Some((name, toks[idx].1.end)),
            Token::LBrace => return None, // Got steered into a function body.
            _ => idx += 1,
        }
    }
    None
}

/// `<ret> <name> ( <params> ) <semantics?> { <body> }`.
fn try_parse_function(
    toks: &[(Token, std::ops::Range<usize>)],
    start: usize,
    content: &str,
) -> Option<(String, std::ops::Range<usize>, usize)> {
    // Skip return-type tokens until we see `<ident> (`.
    let mut idx = start;
    let mut last_ident: Option<usize> = None;
    while idx < toks.len() {
        match toks[idx].0 {
            Token::Ident => {
                last_ident = Some(idx);
                idx += 1;
            }
            Token::LParen => {
                let name_idx = last_ident?;
                // Bail if the most recent identifier wasn't right
                // before this `(`.
                if name_idx + 1 != idx {
                    return None;
                }
                let close_paren = match_close_paren(toks, idx)?;
                let mut after = close_paren;
                // Skip `: SV_Target0`-style semantics.
                while after < toks.len() && !matches!(toks[after].0, Token::LBrace | Token::Semi) {
                    after += 1;
                }
                if after >= toks.len() {
                    return None;
                }
                if matches!(toks[after].0, Token::Semi) {
                    return None;
                }
                let close_brace = match_close_brace(toks, after)?;
                let name_span = toks[name_idx].1.clone();
                let name = content[name_span.clone()].to_string();
                return Some((name, name_span, close_brace));
            }
            Token::Semi | Token::LBrace | Token::RBrace => return None,
            // `<...>` / `(...)` / `[...]` between return type tokens
            // would push us out of "looking at return type" state — keep walking.
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
            // Ignore `<` / `>` inside parens — likely comparisons.
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

fn parse_include_path(directive: &str) -> Option<String> {
    let bytes = directive.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'<' && bytes[i] != b'"' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let close = if bytes[i] == b'<' { b'>' } else { b'"' };
    i += 1;
    let start = i;
    while i < bytes.len() && bytes[i] != close {
        i += 1;
    }
    if i > start {
        Some(directive[start..i].to_string())
    } else {
        None
    }
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

fn signature_for_resource(content: &str, start: usize, semi_end: usize) -> String {
    let end = semi_end.min(content.len());
    content[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn signature_for_function(content: &str, start: usize, name_end: usize) -> String {
    // Caller passes the start of the return-type span and the end of
    // the function-name span. The signature is the whitespace-
    // collapsed text in between, plus "(...)".
    let end = name_end.min(content.len());
    let head: String = content[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("{head}(...)")
}

fn module_path_from_relative(relative_path: &str) -> String {
    // Mirror the convention used by tree-sitter symbol IDs: the file
    // stem becomes the module segment unless it's a magic name. For
    // shaders that's just the bare stem (no `lib`/`mod`/`main`
    // exclusion list to apply).
    let p = std::path::Path::new(relative_path);
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("shader")
        .to_string()
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

    fn names(extraction: &HlslExtraction) -> Vec<&str> {
        extraction.symbols.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn extracts_cbuffer() {
        let src = r"
cbuffer ViewUniforms : register(b0) {
    float4x4 ViewMatrix;
    float4 ViewOrigin;
}
";
        let out = extract_hlsl_symbols(src, "Engine.usf");
        assert!(names(&out).contains(&"ViewUniforms"), "{:?}", names(&out));
        let sym = &out.symbols[0];
        assert_eq!(sym.kind, "cbuffer");
        assert!(sym.signature.contains("ViewUniforms"));
    }

    #[test]
    fn extracts_resource_bindings() {
        let src = r"
Texture2D SourceTexture;
SamplerState BilinearSampler;
RWTexture2D<float4> Output;
StructuredBuffer<MyData> InputBuffer : register(t3);
";
        let out = extract_hlsl_symbols(src, "Resources.usf");
        let n = names(&out);
        assert!(n.contains(&"SourceTexture"), "{n:?}");
        assert!(n.contains(&"BilinearSampler"), "{n:?}");
        assert!(n.contains(&"Output"), "{n:?}");
        assert!(n.contains(&"InputBuffer"), "{n:?}");
        for sym in &out.symbols {
            assert_eq!(sym.kind, "static");
        }
    }

    #[test]
    fn extracts_function() {
        let src = r#"
#include "/Engine/Public/Platform.ush"

Texture2D SourceTexture;
SamplerState BilinearSampler;

float4 MainPS(float2 UV : TEXCOORD0) : SV_Target0
{
    return SourceTexture.Sample(BilinearSampler, UV);
}
"#;
        let out = extract_hlsl_symbols(src, "DownsamplePS.usf");
        let n = names(&out);
        assert!(n.contains(&"MainPS"), "got {n:?}");
        let main = out.symbols.iter().find(|s| s.name == "MainPS").unwrap();
        assert_eq!(main.kind, "function");
        assert!(main.signature.contains("MainPS"));
        assert!(
            out.includes
                .contains(&"/Engine/Public/Platform.ush".to_string())
        );
    }

    #[test]
    fn extracts_struct_and_compute_shader() {
        let src = r"
struct Particle {
    float3 Position;
    float3 Velocity;
};

RWStructuredBuffer<Particle> Particles;

[numthreads(64, 1, 1)]
void MainCS(uint3 DTid : SV_DispatchThreadID) {
    Particles[DTid.x].Position += Particles[DTid.x].Velocity;
}
";
        let out = extract_hlsl_symbols(src, "ParticleCS.usf");
        let n = names(&out);
        assert!(n.contains(&"Particle"), "{n:?}");
        assert!(n.contains(&"Particles"), "{n:?}");
        assert!(n.contains(&"MainCS"), "{n:?}");
        let particle = out.symbols.iter().find(|s| s.name == "Particle").unwrap();
        assert_eq!(particle.kind, "struct");
        let cs = out.symbols.iter().find(|s| s.name == "MainCS").unwrap();
        assert_eq!(cs.kind, "function");
    }

    #[test]
    fn nested_braces_dont_leak_inner_decls() {
        // A struct field that happens to look like a top-level resource
        // shouldn't double-register. We track brace depth so only
        // top-level matches fire.
        let src = r"
struct Outer {
    Texture2D NestedTexture; // nested — should NOT become a global symbol
};
";
        let out = extract_hlsl_symbols(src, "Outer.usf");
        let n = names(&out);
        assert_eq!(n, vec!["Outer"]);
    }

    #[test]
    fn comments_dont_register_as_symbols() {
        let src = r"
// Texture2D ShouldNotMatch;
/* RWTexture2D<float4> AlsoShouldNotMatch; */
Texture2D RealOne;
";
        let out = extract_hlsl_symbols(src, "Comments.usf");
        let n = names(&out);
        assert_eq!(n, vec!["RealOne"]);
    }

    #[test]
    fn includes_collected_with_both_quote_styles() {
        let src = r#"
#include "/Engine/Private/Common.ush"
#include <Foo.hlsl>
"#;
        let out = extract_hlsl_symbols(src, "Includes.usf");
        assert!(
            out.includes
                .contains(&"/Engine/Private/Common.ush".to_string())
        );
        assert!(out.includes.contains(&"Foo.hlsl".to_string()));
    }

    #[test]
    fn empty_input_is_safe() {
        let out = extract_hlsl_symbols("", "empty.usf");
        assert!(out.symbols.is_empty());
        assert!(out.includes.is_empty());
    }

    #[test]
    fn is_shader_extension_recognises_common_exts() {
        assert!(is_shader_extension("usf"));
        assert!(is_shader_extension("hlsl"));
        assert!(is_shader_extension("frag"));
        assert!(is_shader_extension("metal"));
        assert!(is_shader_extension("wgsl"));
        assert!(!is_shader_extension("rs"));
        assert!(!is_shader_extension(""));
    }

    #[test]
    fn module_path_from_filename_uses_stem() {
        let out = extract_hlsl_symbols("Texture2D Foo;\n", "Engine/Source/Renderer/MyShader.usf");
        let sym = &out.symbols[0];
        assert_eq!(sym.module_path, "MyShader");
        assert!(sym.id.0.contains("MyShader::Foo"));
    }
}
