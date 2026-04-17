//! Heuristic assembly language parser.
//!
//! Splits assembly files into sections at top-level labels (lines matching
//! `^[A-Za-z_][A-Za-z0-9_@$.]*::?`), extracts preceding comment blocks as
//! doc comments, and produces a [`DocumentTree`] with per-label child sections.
//!
//! Works across assembly dialects (RGBDS, NASM, GAS, ARM, x86) without
//! requiring a tree-sitter grammar.

use std::path::Path;

use crate::parser::section_id::{generate_code_section_id, generate_section_id};
use crate::types::{ContentId, DocumentTree, Section, SectionId};

/// Characters that start a comment in common assembly dialects.
const COMMENT_CHARS: &[char] = &[';', '@'];

/// Assembly SECTION/SEGMENT directive prefixes (case-insensitive).
const SECTION_DIRECTIVES: &[&str] = &["section", "segment", ".section", ".text", ".data", ".bss"];

/// Check whether a file extension is an assembly language extension.
pub(super) fn is_assembly_extension(ext: &str) -> bool {
    matches!(ext, "asm" | "s" | "S" | "inc")
}

/// An extracted assembly label with its associated code block.
struct AsmLabel {
    /// The label name (e.g. `Main`, `ReadJoypad`).
    name: String,
    /// Whether the label is exported (e.g. `Main::` in RGBDS).
    exported: bool,
    /// Comment block preceding the label, if any.
    doc_comment: Option<String>,
    /// The full text of this label's code block (label line + body).
    text: String,
    /// 0-based line number where the label starts.
    line_start: usize,
}

/// Parse an assembly file into a structured [`DocumentTree`].
///
/// Splits on top-level labels and builds per-label sections as children
/// of a file-level root section. Lines before the first label become the
/// file header section.
pub(super) fn parse_assembly(path: &Path, content: &str) -> DocumentTree {
    let source_path = path.to_string_lossy();
    let lines: Vec<&str> = content.lines().collect();

    let labels = extract_labels(&lines);
    let (header_text, header_doc) = extract_header(&lines, &labels);

    // Build child sections from labels
    let child_sections: Vec<Section> = labels
        .iter()
        .map(|label| build_label_section(&source_path, label))
        .collect();

    // Build overview text: header doc + label listing
    let mut overview_parts = Vec::new();
    if let Some(doc) = &header_doc {
        overview_parts.push(doc.clone());
    }
    if !header_text.is_empty() {
        overview_parts.push(header_text);
    }
    if !labels.is_empty() {
        let listing: Vec<String> = labels
            .iter()
            .map(|l| {
                let suffix = if l.exported { " (exported)" } else { "" };
                format!("{}:{suffix}", l.name)
            })
            .collect();
        overview_parts.push(listing.join("\n"));
    }

    let root_section = Section {
        id: SectionId(generate_section_id(&source_path, &[])),
        heading_path: vec![source_path.to_string()],
        depth: 1,
        text: overview_parts.join("\n\n"),
        structural_nodes: Vec::new(),
        children: child_sections,
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

/// Extract all top-level labels from the assembly source.
///
/// A top-level label is a line with no leading whitespace that matches:
/// `[A-Za-z_][A-Za-z0-9_@$.]*::?`
///
/// Local labels (prefixed with `.`) are not treated as top-level.
fn extract_labels(lines: &[&str]) -> Vec<AsmLabel> {
    let mut labels = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if let Some((name, exported)) = parse_label_line(lines[i]) {
            // Gather preceding comment block
            let doc_comment = extract_preceding_comment(lines, i);

            // Gather body: all lines until the next top-level label
            let body_start = i;
            i += 1;
            while i < lines.len() && parse_label_line(lines[i]).is_none() {
                i += 1;
            }

            let text = lines[body_start..i].join("\n");

            labels.push(AsmLabel {
                name,
                exported,
                doc_comment,
                text,
                line_start: body_start,
            });
        } else {
            i += 1;
        }
    }

    labels
}

/// Try to parse a line as a top-level assembly label.
///
/// Returns `(name, is_exported)` if the line is a label.
fn parse_label_line(line: &str) -> Option<(String, bool)> {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return None;
    }

    // Must start at column 0 (no leading whitespace)
    let first_char = trimmed.as_bytes().first().copied()?;
    if first_char == b'.' || first_char.is_ascii_whitespace() {
        return None;
    }

    // Must start with a letter or underscore
    if !first_char.is_ascii_alphabetic() && first_char != b'_' {
        return None;
    }

    // Find the colon(s)
    let colon_pos = trimmed.find(':')?;
    let name_part = &trimmed[..colon_pos];

    // Validate name characters: alphanumeric, _, @, $, .
    if !name_part
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'@' || b == b'$' || b == b'.')
    {
        return None;
    }

    // Skip SECTION-like directives (e.g., `SECTION "name", ROM0`)
    let lower = name_part.to_ascii_lowercase();
    if SECTION_DIRECTIVES.iter().any(|d| lower == *d) {
        return None;
    }

    // Check for double colon (exported in RGBDS)
    let exported = trimmed.as_bytes().get(colon_pos + 1) == Some(&b':');

    Some((name_part.to_string(), exported))
}

/// Extract the comment block immediately preceding a label.
///
/// Walks backwards from `label_line` collecting consecutive comment lines
/// (lines where the first non-whitespace character is in `COMMENT_CHARS`).
fn extract_preceding_comment(lines: &[&str], label_line: usize) -> Option<String> {
    if label_line == 0 {
        return None;
    }

    let mut comment_lines = Vec::new();
    let mut j = label_line - 1;
    loop {
        let trimmed = lines[j].trim();
        if trimmed.is_empty() && !comment_lines.is_empty() {
            // Blank line after we already found comments — stop
            break;
        }
        if trimmed.is_empty() && comment_lines.is_empty() {
            // Skip blank lines before any comments found
            if j == 0 {
                break;
            }
            j -= 1;
            continue;
        }

        let first = trimmed.chars().next()?;
        if !COMMENT_CHARS.contains(&first) {
            break;
        }

        // Strip leading comment char and optional space
        let rest = trimmed[first.len_utf8()..].trim_start();
        comment_lines.push(rest.to_string());

        if j == 0 {
            break;
        }
        j -= 1;
    }

    if comment_lines.is_empty() {
        return None;
    }

    comment_lines.reverse();
    Some(comment_lines.join("\n"))
}

/// Extract the file header: everything before the first label.
///
/// Returns `(non_comment_text, doc_comment)`.
fn extract_header(lines: &[&str], labels: &[AsmLabel]) -> (String, Option<String>) {
    let first_label_line = labels.first().map_or(lines.len(), |l| l.line_start);
    let header_lines = &lines[..first_label_line];

    let mut doc_lines = Vec::new();
    let mut other_lines = Vec::new();

    for line in header_lines {
        let trimmed = line.trim();
        if let Some(first) = trimmed.chars().next()
            && COMMENT_CHARS.contains(&first)
        {
            let rest = trimmed[first.len_utf8()..].trim_start();
            doc_lines.push(rest.to_string());
            continue;
        }
        if !trimmed.is_empty() {
            other_lines.push(trimmed.to_string());
        }
    }

    let doc = if doc_lines.is_empty() {
        None
    } else {
        Some(doc_lines.join("\n"))
    };

    (other_lines.join("\n"), doc)
}

/// Build a section for a single assembly label.
fn build_label_section(source_path: &str, label: &AsmLabel) -> Section {
    let section_id = generate_code_section_id(source_path, &[], &label.name);

    let kind_label = if label.exported { "export" } else { "label" };
    let heading = format!("{kind_label} {}", label.name);

    let mut text_parts = Vec::new();
    if let Some(doc) = &label.doc_comment {
        text_parts.push(doc.clone());
    }
    text_parts.push(label.text.clone());

    Section {
        id: SectionId(section_id),
        heading_path: vec![source_path.to_string(), heading],
        depth: 2,
        text: text_parts.join("\n\n"),
        structural_nodes: Vec::new(),
        children: Vec::new(),
        claims: Vec::new(),
        summary: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_labels() {
        let asm = "\
; File header comment
INCLUDE \"constants.asm\"

; Start the main game loop
Main::
\tld a, 1
\tcall DoSomething
.loop
\tjr .loop

; Handle input
HandleInput:
\tld b, a
\tret
";
        let tree = parse_assembly(Path::new("game.asm"), asm);
        assert_eq!(tree.sections.len(), 1);
        let root = &tree.sections[0];

        // Two child sections: Main and HandleInput
        assert_eq!(root.children.len(), 2);
        assert!(root.children[0].heading_path[1].contains("Main"));
        assert!(root.children[1].heading_path[1].contains("HandleInput"));
    }

    #[test]
    fn exported_vs_local_labels() {
        let line_exported = "Main::";
        let line_local = "helper:";
        let line_local_dot = ".loop";
        let line_instruction = "\tld a, 1";

        assert_eq!(
            parse_label_line(line_exported),
            Some(("Main".to_string(), true))
        );
        assert_eq!(
            parse_label_line(line_local),
            Some(("helper".to_string(), false))
        );
        assert_eq!(parse_label_line(line_local_dot), None); // dot-prefixed = local
        assert_eq!(parse_label_line(line_instruction), None); // indented
    }

    #[test]
    fn section_directive_not_treated_as_label() {
        assert_eq!(parse_label_line("SECTION \"ROM0\", ROM0"), None);
        assert_eq!(parse_label_line("section .text"), None);
    }

    #[test]
    fn preceding_comment_extraction() {
        let lines = vec![
            "; This is a doc comment",
            "; for the label below",
            "MyLabel:",
            "\tret",
        ];
        let doc = extract_preceding_comment(&lines, 2);
        assert_eq!(
            doc,
            Some("This is a doc comment\nfor the label below".to_string())
        );
    }

    #[test]
    fn no_comment_before_label() {
        let lines = vec!["MyLabel:", "\tret"];
        let doc = extract_preceding_comment(&lines, 0);
        assert_eq!(doc, None);
    }

    #[test]
    fn header_extraction() {
        let lines = vec![
            "; Copyright 2024",
            "; Game source",
            "",
            "INCLUDE \"defs.asm\"",
            "",
            "Main:",
        ];
        let labels = extract_labels(&lines);
        let (header_text, header_doc) = extract_header(&lines, &labels);
        assert_eq!(header_doc, Some("Copyright 2024\nGame source".to_string()));
        assert!(header_text.contains("INCLUDE"));
    }

    #[test]
    fn empty_file() {
        let tree = parse_assembly(Path::new("empty.asm"), "");
        assert_eq!(tree.sections.len(), 1);
        assert!(tree.sections[0].children.is_empty());
    }

    #[test]
    fn rgbds_double_colon_exported() {
        let asm = "\
DisplayStartMenu::
\tld a, BANK(StartMenu_Pokedex)
\tldh [hLoadedROMBank], a

RedisplayStartMenu::
\tfarcall DrawStartMenu
\tcall UpdateSprites
";
        let tree = parse_assembly(Path::new("start_menu.asm"), asm);
        let root = &tree.sections[0];
        assert_eq!(root.children.len(), 2);

        // Overview should list both labels
        assert!(root.text.contains("DisplayStartMenu"));
        assert!(root.text.contains("RedisplayStartMenu"));
    }

    #[test]
    fn x86_nasm_style() {
        let asm = "\
; x86 NASM example
section .text
global _start

_start:
    mov eax, 1
    mov ebx, 0
    int 0x80

print_message:
    ; print a message
    mov eax, 4
    mov ebx, 1
    ret
";
        let tree = parse_assembly(Path::new("hello.asm"), asm);
        let root = &tree.sections[0];
        // _start and print_message should be detected
        assert_eq!(root.children.len(), 2);
        assert!(root.children[0].heading_path[1].contains("_start"));
        assert!(root.children[1].heading_path[1].contains("print_message"));
    }
}
