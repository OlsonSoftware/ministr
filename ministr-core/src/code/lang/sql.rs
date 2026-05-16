//! SQL AST walker refinement (tree-sitter-sequel).
//!
//! Shape: `program` → `statement` → `create_table` / `create_view` /
//! `create_function` / … → `object_reference` → `identifier`. The
//! generic classifier matches none of these, so SQL files index with
//! no symbols. This surfaces DDL objects by name.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// SQL language refinement.
pub struct SqlRefinement;

impl LanguageRefinement for SqlRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        match kind {
            "create_table" => Some(Some(ItemKind::Struct)),
            "create_view" => Some(Some(ItemKind::Type)),
            "create_function" | "create_procedure" => Some(Some(ItemKind::Function)),
            "create_index" | "create_schema" | "create_type" | "create_trigger" => {
                Some(Some(ItemKind::Type))
            }
            // Wrapper / noise.
            "statement" | "program" | "comment" => Some(None),
            _ => None,
        }
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        if !node.kind().starts_with("create_") {
            return None;
        }
        // First `object_reference` child carries the object name.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "object_reference"
                && let Ok(t) = child.utf8_text(source)
            {
                let t = t.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
        None
    }

    fn language_name(&self) -> &'static str {
        "sql"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::{GrammarRegistry, generic_extract_symbols_for};

    #[test]
    fn ddl_objects_become_symbols() {
        let src = "CREATE TABLE users (id int);\nCREATE VIEW active AS SELECT 1;\nCREATE FUNCTION f() RETURNS int AS $$ SELECT 1 $$;\n";
        let l = GrammarRegistry::global().language_by_name("sql").unwrap();
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        let t = p.parse(src, None).unwrap();
        let syms = generic_extract_symbols_for(&t, src.as_bytes(), "schema.sql", &[], Some("sql"));
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"users"), "got {names:?}");
        assert!(names.contains(&"active"), "got {names:?}");
        assert!(names.contains(&"f"), "got {names:?}");
    }
}
