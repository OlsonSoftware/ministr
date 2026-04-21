//! A queryable collection of extracted symbols.
//!
//! [`SymbolTable`] wraps a `Vec<Symbol>` and provides filtering and search
//! methods for finding symbols by name, kind, visibility, or module path.

use crate::code::ast_parser::ItemKind;
use crate::code::symbol::{Symbol, Visibility};

/// A queryable collection of code symbols.
///
/// # Examples
///
/// ```
/// use ministr_core::code::{AstParser, SymbolTable, ItemKind, Visibility, extract_symbols};
///
/// let mut parser = AstParser::new();
/// let source = b"pub fn foo() {}\nstruct Bar;";
/// let tree = parser.parse(source).unwrap();
/// let symbols = extract_symbols(&tree, source, "lib.rs", &[]);
/// let table = SymbolTable::new(symbols);
///
/// assert_eq!(table.len(), 2);
/// assert_eq!(table.filter_by_kind(ItemKind::Function).len(), 1);
/// assert_eq!(table.filter_by_visibility(&Visibility::Public).len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct SymbolTable {
    symbols: Vec<Symbol>,
}

impl SymbolTable {
    /// Create a new `SymbolTable` from a list of symbols.
    #[must_use]
    pub fn new(symbols: Vec<Symbol>) -> Self {
        Self { symbols }
    }

    /// Number of symbols in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Get all symbols.
    #[must_use]
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// Find symbols whose name contains the given pattern (case-insensitive).
    ///
    /// # Examples
    ///
    /// ```
    /// use ministr_core::code::{AstParser, SymbolTable, extract_symbols};
    ///
    /// let mut parser = AstParser::new();
    /// let source = b"pub fn hello_world() {}\nfn goodbye() {}";
    /// let tree = parser.parse(source).unwrap();
    /// let table = SymbolTable::new(extract_symbols(&tree, source, "lib.rs", &[]));
    ///
    /// let results = table.find_by_name("hello");
    /// assert_eq!(results.len(), 1);
    /// assert_eq!(results[0].name, "hello_world");
    /// ```
    #[must_use]
    pub fn find_by_name(&self, pattern: &str) -> Vec<&Symbol> {
        let pattern_lower = pattern.to_lowercase();
        self.symbols
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&pattern_lower))
            .collect()
    }

    /// Filter symbols by kind.
    #[must_use]
    pub fn filter_by_kind(&self, kind: ItemKind) -> Vec<&Symbol> {
        self.symbols.iter().filter(|s| s.kind == kind).collect()
    }

    /// Filter symbols by visibility.
    #[must_use]
    pub fn filter_by_visibility(&self, visibility: &Visibility) -> Vec<&Symbol> {
        self.symbols
            .iter()
            .filter(|s| &s.visibility == visibility)
            .collect()
    }

    /// Filter symbols whose module path starts with the given prefix.
    ///
    /// An empty prefix matches all symbols.
    ///
    /// # Examples
    ///
    /// ```
    /// use ministr_core::code::{AstParser, SymbolTable, extract_symbols};
    ///
    /// let mut parser = AstParser::new();
    /// let source = b"pub fn foo() {}";
    /// let tree = parser.parse(source).unwrap();
    /// let table = SymbolTable::new(extract_symbols(&tree, source, "lib.rs", &["core", "config"]));
    ///
    /// assert_eq!(table.filter_by_module("core").len(), 1);
    /// assert_eq!(table.filter_by_module("core::config").len(), 1);
    /// assert_eq!(table.filter_by_module("other").len(), 0);
    /// ```
    #[must_use]
    pub fn filter_by_module(&self, path: &str) -> Vec<&Symbol> {
        if path.is_empty() {
            return self.symbols.iter().collect();
        }

        let segments: Vec<&str> = path.split("::").collect();
        self.symbols
            .iter()
            .filter(|s| {
                if s.module_path.len() < segments.len() {
                    return false;
                }
                s.module_path
                    .iter()
                    .zip(segments.iter())
                    .all(|(a, b)| a == b)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::{AstParser, extract_symbols};

    fn make_table(source: &[u8]) -> SymbolTable {
        let mut parser = AstParser::new();
        let tree = parser.parse(source).unwrap();
        let symbols = extract_symbols(&tree, source, "test.rs", &["test"]);
        SymbolTable::new(symbols)
    }

    #[test]
    fn find_by_name_case_insensitive() {
        let table = make_table(b"pub fn HelloWorld() {}\nfn goodbye() {}");
        assert_eq!(table.find_by_name("hello").len(), 1);
        assert_eq!(table.find_by_name("HELLO").len(), 1);
        assert_eq!(table.find_by_name("world").len(), 1);
        assert_eq!(table.find_by_name("nonexistent").len(), 0);
    }

    #[test]
    fn filter_by_kind() {
        let table = make_table(b"pub struct Foo;\npub fn bar() {}\nconst X: i32 = 0;\nimpl Foo {}");
        assert_eq!(table.filter_by_kind(ItemKind::Struct).len(), 1);
        assert_eq!(table.filter_by_kind(ItemKind::Function).len(), 1);
        assert_eq!(table.filter_by_kind(ItemKind::Const).len(), 1);
        assert_eq!(table.filter_by_kind(ItemKind::Impl).len(), 1);
        assert_eq!(table.filter_by_kind(ItemKind::Enum).len(), 0);
    }

    #[test]
    fn filter_by_visibility() {
        let table =
            make_table(b"pub fn public_fn() {}\nfn private_fn() {}\npub(crate) struct Internal;");
        assert_eq!(table.filter_by_visibility(&Visibility::Public).len(), 1);
        assert_eq!(table.filter_by_visibility(&Visibility::Private).len(), 1);
        assert_eq!(table.filter_by_visibility(&Visibility::PubCrate).len(), 1);
    }

    #[test]
    fn filter_by_module() {
        let mut parser = AstParser::new();
        let source = b"pub fn a() {}\npub fn b() {}";
        let tree = parser.parse(source).unwrap();

        let mut symbols = extract_symbols(&tree, source, "a.rs", &["core", "config"]);
        let tree2 = parser.parse(b"pub fn c() {}").unwrap();
        symbols.extend(extract_symbols(
            &tree2,
            b"pub fn c() {}",
            "b.rs",
            &["core", "types"],
        ));

        let table = SymbolTable::new(symbols);
        assert_eq!(table.filter_by_module("core").len(), 3);
        assert_eq!(table.filter_by_module("core::config").len(), 2);
        assert_eq!(table.filter_by_module("core::types").len(), 1);
        assert_eq!(table.filter_by_module("other").len(), 0);
        assert_eq!(table.filter_by_module("").len(), 3);
    }

    #[test]
    fn len_and_is_empty() {
        let table = SymbolTable::new(vec![]);
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        let table = make_table(b"fn foo() {}");
        assert!(!table.is_empty());
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn symbols_accessor() {
        let table = make_table(b"pub fn foo() {}\nstruct Bar;");
        assert_eq!(table.symbols().len(), 2);
        assert_eq!(table.symbols()[0].name, "foo");
        assert_eq!(table.symbols()[1].name, "Bar");
    }
}
