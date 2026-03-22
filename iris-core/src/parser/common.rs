//! Shared utilities for document parsers.
//!
//! Provides the [`RawSection`] intermediate representation and the
//! [`nest_sections`] function used by all parser implementations to
//! convert a flat list of depth-annotated sections into a nested tree.

use super::section_id::generate_section_id;
use crate::types::{Section, SectionId, StructuralNode};

/// Intermediate representation of a section being built during parsing.
pub struct RawSection {
    /// Heading hierarchy path (e.g. `["Chapter", "Subsection"]`).
    pub heading_path: Vec<String>,
    /// Heading depth (1 = top-level, 2 = subsection, etc.). 0 = implicit root.
    pub depth: u32,
    /// Accumulated text fragments for this section.
    pub text_parts: Vec<String>,
    /// Typed structural elements (code blocks, tables, lists).
    pub structural_nodes: Vec<StructuralNode>,
}

/// Convert a flat list of [`RawSection`]s into a vec of [`Section`]s with
/// stable IDs, then nest them into a tree based on heading depth.
pub fn build_section_tree(source_path: &str, raw_sections: Vec<RawSection>) -> Vec<Section> {
    let flat: Vec<Section> = raw_sections
        .into_iter()
        .map(|raw| {
            let heading_path_refs: Vec<&str> =
                raw.heading_path.iter().map(String::as_str).collect();
            let id = SectionId(generate_section_id(source_path, &heading_path_refs));

            Section {
                id,
                heading_path: raw.heading_path,
                depth: raw.depth,
                text: raw.text_parts.join("\n\n"),
                structural_nodes: raw.structural_nodes,
                children: Vec::new(),
                claims: Vec::new(),
                summary: None,
            }
        })
        .collect();

    nest_sections(flat)
}

/// Build a nested section tree from a flat depth-ordered list.
///
/// Sections with greater depth are nested as children of the preceding
/// section with lesser depth.
fn nest_sections(flat: Vec<Section>) -> Vec<Section> {
    let mut result: Vec<Section> = Vec::new();
    let mut stack: Vec<Section> = Vec::new();

    for section in flat {
        // Pop sections from the stack that are at the same or deeper level
        while stack.last().is_some_and(|top| top.depth >= section.depth) {
            let popped = stack.pop().expect("stack non-empty");
            if let Some(parent) = stack.last_mut() {
                parent.children.push(popped);
            } else {
                result.push(popped);
            }
        }
        stack.push(section);
    }

    // Drain remaining stack
    while let Some(popped) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(popped);
        } else {
            result.push(popped);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_section_tree_flat() {
        let sections = vec![
            RawSection {
                heading_path: vec!["A".into()],
                depth: 1,
                text_parts: vec!["Content A.".into()],
                structural_nodes: Vec::new(),
            },
            RawSection {
                heading_path: vec!["B".into()],
                depth: 1,
                text_parts: vec!["Content B.".into()],
                structural_nodes: Vec::new(),
            },
        ];

        let tree = build_section_tree("test.html", sections);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].heading_path, vec!["A"]);
        assert_eq!(tree[1].heading_path, vec!["B"]);
    }

    #[test]
    fn build_section_tree_nested() {
        let sections = vec![
            RawSection {
                heading_path: vec!["Top".into()],
                depth: 1,
                text_parts: vec!["Intro.".into()],
                structural_nodes: Vec::new(),
            },
            RawSection {
                heading_path: vec!["Top".into(), "Sub".into()],
                depth: 2,
                text_parts: vec!["Detail.".into()],
                structural_nodes: Vec::new(),
            },
        ];

        let tree = build_section_tree("test.html", sections);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].heading_path, vec!["Top", "Sub"]);
    }
}
