//! Code intelligence via tree-sitter AST parsing.
//!
//! The [`AstParser`] initializes a tree-sitter parser with a language grammar
//! and parses source bytes into a syntax tree. The [`walk_top_level_items`]
//! function traverses the tree to identify top-level Rust items (functions,
//! structs, enums, traits, impl blocks, etc.).
//!
//! The [`extract_symbols`] function builds on the walker to produce rich
//! [`Symbol`] values with visibility, doc comments, and signatures. The
//! [`SymbolTable`] collects symbols and provides query methods.

pub(crate) mod ast_parser;
mod symbol;
mod symbol_table;

pub use ast_parser::{AstItem, AstParser, ItemKind, walk_top_level_items};
pub use symbol::{Symbol, Visibility, extract_symbols};
pub use symbol_table::SymbolTable;
