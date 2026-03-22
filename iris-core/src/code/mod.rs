//! Code intelligence via tree-sitter AST parsing.
//!
//! The [`AstParser`] initializes a tree-sitter parser with a language grammar
//! and parses source bytes into a syntax tree. The [`walk_top_level_items`]
//! function traverses the tree to identify top-level Rust items (functions,
//! structs, enums, traits, impl blocks, etc.).

mod ast_parser;

pub use ast_parser::{AstItem, AstParser, ItemKind, walk_top_level_items};
