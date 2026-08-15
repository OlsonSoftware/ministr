//! Mechanical language gate over every string literal in `src/`
//! (GUI-BLUEPRINT-v8 §4): the console says project / engine / rebuild /
//! needs update — never the internal vocabulary — and ships zero emoji
//! and zero exclamation marks.
//!
//! The scanner strips comments, then walks the remaining double-quoted
//! and raw string literals. Identifiers and comments may use internal
//! names freely (`ensure_daemon_spawned` is API truth); only what could
//! reach the screen is gated. Known limitation: byte-raw strings
//! (`br"…"`) are parsed as ordinary strings — don't use them for UI text.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Internal vocabulary that must never appear in a string literal.
/// Matched case-insensitively on word boundaries, so "unregister" does
/// not double-report as "register" and "assess" never trips "sse".
const BANNED_WORDS: &[&str] = &[
    "corpus",
    "corpora",
    "daemon",
    "ingest",
    "ingestion",
    "indexing",
    "reindex",
    "stale",
    "freshness",
    "register",
    "unregister",
    "sse",
    "uds",
    "socket",
    "http",
];

/// One rule violation: (line number, offending literal, reason).
type Violation = (usize, String, String);

#[test]
fn ui_strings_use_plain_words_only() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src, &mut files);
    assert!(!files.is_empty(), "no source files found under src/");

    let mut report = String::new();
    for file in &files {
        let source = fs::read_to_string(file).expect("read source file");
        for (line, literal, reason) in scan_source(&source) {
            writeln!(report, "{}:{line}: {reason} in {literal:?}", file.display())
                .expect("write to report");
        }
    }
    assert!(report.is_empty(), "banned UI language found:\n{report}");
}

#[test]
fn gate_catches_internal_words() {
    let sample = r#"fn f() { let s = "3 Corpora indexed"; }"#;
    let violations = scan_source(sample);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].2.contains("corpora"));
}

#[test]
fn gate_catches_exclamation_marks_and_emoji() {
    let sample = "fn f() { let s = \"You're all set! \u{1f389}\"; }";
    let reasons: Vec<String> = scan_source(sample).into_iter().map(|v| v.2).collect();
    assert!(reasons.iter().any(|r| r.contains("exclamation")));
    assert!(reasons.iter().any(|r| r.contains("emoji")));
}

#[test]
fn gate_ignores_comments_and_identifiers() {
    let sample = "// the daemon side spawns via ensure_daemon_spawned\n\
                  /* corpus registration is internal */\n\
                  fn ensure_daemon_spawned() { let s = \"engine \u{25cf} running\"; }";
    assert!(scan_source(sample).is_empty());
}

#[test]
fn gate_respects_word_boundaries() {
    // Substring hits inside larger words are not violations.
    let sample = r#"fn f() { let s = "assess the index and update"; }"#;
    assert!(scan_source(sample).is_empty());
}

#[test]
fn gate_reads_raw_strings() {
    let sample = r##"fn f() { let s = r#"reindex now"#; }"##;
    assert_eq!(scan_source(sample).len(), 1);
}

/// Recursively collect `.rs` files under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Extract string literals from Rust source (comments stripped) and
/// check each against the language rules.
fn scan_source(source: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (line, literal) in string_literals(source) {
        for reason in check_literal(&literal) {
            violations.push((line, literal.clone(), reason));
        }
    }
    violations
}

/// The rule check for one literal's text.
fn check_literal(text: &str) -> Vec<String> {
    let mut reasons = Vec::new();
    let lower = text.to_lowercase();
    for word in BANNED_WORDS {
        if contains_word(&lower, word) {
            reasons.push(format!("banned word {word:?}"));
        }
    }
    if text.contains('!') {
        reasons.push("exclamation mark".to_owned());
    }
    if text.chars().any(is_emoji) {
        reasons.push("emoji".to_owned());
    }
    reasons
}

/// Case-sensitive word-boundary search (`haystack` is pre-lowercased).
fn contains_word(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(word) {
        let begin = start + pos;
        let end = begin + word.len();
        let boundary_before = begin == 0 || !is_word_byte(bytes[begin - 1]);
        let boundary_after = end == bytes.len() || !is_word_byte(bytes[end]);
        if boundary_before && boundary_after {
            return true;
        }
        start = begin + 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Emoji and emoji-adjacent codepoints. Box-drawing, geometric shapes
/// (● U+25CF, ○ U+25CB), ellipsis, and angle quotes are NOT banned —
/// they are structure in the blueprint's world.
fn is_emoji(c: char) -> bool {
    matches!(u32::from(c),
        0x1F000..=0x1FAFF // emoticons, symbols, transport, supplemental
        | 0x2600..=0x27BF // misc symbols + dingbats
        | 0x2B00..=0x2BFF // misc symbols and arrows (stars)
        | 0xFE00..=0xFE0F // variation selectors
    )
}

/// Walk the source and yield `(line_number, literal_text)` for every
/// ordinary and raw string literal outside comments.
#[allow(clippy::too_many_lines)]
fn string_literals(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut i = 0;
    let mut line = 1;

    while i < chars.len() {
        match chars[i] {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut depth = 1;
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            'r' if !prev_is_ident(&chars, i) && raw_string_hashes(&chars, i).is_some() => {
                let hashes = raw_string_hashes(&chars, i).expect("checked above");
                // Skip `r`, the hashes, and the opening quote.
                i += 1 + hashes + 1;
                let start_line = line;
                let mut text = String::new();
                while i < chars.len() {
                    if chars[i] == '"' && closes_raw(&chars, i, hashes) {
                        i += 1 + hashes;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                literals.push((start_line, text));
            }
            '"' => {
                i += 1;
                let start_line = line;
                let mut text = String::new();
                while i < chars.len() {
                    match chars[i] {
                        '\\' => {
                            // Keep escapes out of the checked text; the
                            // words that matter are the unescaped ones.
                            i += 2;
                        }
                        '"' => {
                            i += 1;
                            break;
                        }
                        c => {
                            if c == '\n' {
                                line += 1;
                            }
                            text.push(c);
                            i += 1;
                        }
                    }
                }
                literals.push((start_line, text));
            }
            '\'' => {
                // Char literal or lifetime. A char literal is short and
                // closed by a quote; a lifetime has no closing quote.
                if chars.get(i + 1) == Some(&'\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    literals
}

/// Is the char before `i` part of an identifier (making a leading `r`
/// a name suffix rather than a raw-string sigil)?
fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
}

/// If `chars[i]` starts a raw string (`r"`, `r#"`, …), the hash count.
fn raw_string_hashes(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    (chars.get(j) == Some(&'"')).then_some(hashes)
}

/// Does the quote at `i` close a raw string with `hashes` hashes?
fn closes_raw(chars: &[char], i: usize, hashes: usize) -> bool {
    (1..=hashes).all(|k| chars.get(i + k) == Some(&'#'))
}
