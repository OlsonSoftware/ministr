//! HCL / Terraform AST walker refinement.
//!
//! tree-sitter-hcl shape: `config_file` → `body` → `block`. Each `block`
//! is `identifier` (`resource` / `module` / `variable` / `output` /
//! `data` / `provider` / …) followed by zero or more `string_lit`
//! labels. None of this is caught by the generic classifier, so without
//! this refinement Terraform/HCL files index with no symbols at all.
//!
//! The symbol name is the dotted block address — e.g.
//! `resource.aws_s3_bucket.web`, `variable.region`, `module.vpc` — which
//! is exactly how Terraform itself addresses resources.

use crate::code::ast_parser::ItemKind;
use crate::code::lang::LanguageRefinement;

/// HCL / Terraform language refinement.
pub struct HclRefinement;

impl LanguageRefinement for HclRefinement {
    fn classify_node_kind(&self, kind: &str) -> Option<Option<ItemKind>> {
        match kind {
            // Every HCL block is a declaration; treat it as a Struct
            // (record-like) so it surfaces in the symbol index and
            // nested-member recursion is not attempted.
            "block" => Some(Some(ItemKind::Struct)),
            // Wrapper / noise nodes — skip without delegating.
            "body" | "config_file" | "comment" | "attribute" => Some(None),
            _ => None,
        }
    }

    fn extract_name(&self, node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
        if node.kind() != "block" {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    if let Ok(t) = child.utf8_text(source) {
                        parts.push(t.trim().to_string());
                    }
                }
                "string_lit" => {
                    // Inner `template_literal` carries the unquoted label.
                    let mut c2 = child.walk();
                    for g in child.children(&mut c2) {
                        if g.kind() == "template_literal"
                            && let Ok(t) = g.utf8_text(source)
                        {
                            parts.push(t.trim().to_string());
                        }
                    }
                }
                // Stop once the block body starts.
                "block_start" | "{" => break,
                _ => {}
            }
        }
        let name = parts.join(".");
        (!name.is_empty()).then_some(name)
    }

    fn language_name(&self) -> &'static str {
        "hcl"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::{GrammarRegistry, generic_extract_symbols_for};

    #[test]
    fn classify() {
        let r = HclRefinement;
        assert_eq!(r.classify_node_kind("block"), Some(Some(ItemKind::Struct)));
        assert_eq!(r.classify_node_kind("body"), Some(None));
        assert_eq!(r.classify_node_kind("identifier"), None);
    }

    #[test]
    fn terraform_blocks_become_symbols() {
        let src = "resource \"aws_s3_bucket\" \"web\" {\n bucket = \"x\"\n}\nvariable \"region\" { default = \"us\" }\nmodule \"vpc\" { source = \"./vpc\" }\n";
        let l = GrammarRegistry::global().language_by_name("hcl").unwrap();
        let mut p = tree_sitter::Parser::new();
        p.set_language(l).unwrap();
        let t = p.parse(src, None).unwrap();
        let syms = generic_extract_symbols_for(&t, src.as_bytes(), "main.tf", &[], Some("hcl"));
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"resource.aws_s3_bucket.web"),
            "got {names:?}"
        );
        assert!(names.contains(&"variable.region"), "got {names:?}");
        assert!(names.contains(&"module.vpc"), "got {names:?}");
    }
}
