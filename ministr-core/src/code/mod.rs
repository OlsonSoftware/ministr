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
//!
//! The [`GrammarRegistry`] maps file extensions to tree-sitter language
//! grammars via cargo feature flags. Rust is always on; the `lang-all`
//! default additionally enables Python, JavaScript, TypeScript/TSX, Go,
//! Java, C, C++, Ruby, C#, Swift, Kotlin, Bash, PHP, Scala, Lua, Elixir,
//! Haskell, OCaml (impl + interface), Dart, R, HCL/Terraform, JSON, YAML,
//! TOML, SQL, Zig, and Protobuf. The [`generic_extract_symbols`] function
//! provides language-agnostic symbol extraction using node kind heuristics
//! common across grammars.

pub(crate) mod ast_parser;
pub mod bridge;
mod complexity;
pub mod cpp_fallback;
pub mod generic_extractor;
pub mod grammar;
pub mod hlsl;
pub mod lang;
pub mod package_graph;
pub mod refs;
mod symbol;
mod symbol_table;

pub use ast_parser::{AstItem, AstParser, ItemKind, walk_top_level_items};
pub use complexity::cyclomatic_complexity;
pub use generic_extractor::generic_extract_symbols;
pub use grammar::{ALL_CODE_EXTENSIONS, GrammarRegistry, LanguageGrammar};
pub use symbol::{Symbol, Visibility, extract_symbols};
pub use symbol_table::SymbolTable;
