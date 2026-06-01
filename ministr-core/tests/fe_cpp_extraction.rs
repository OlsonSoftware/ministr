//! FE — C++ comprehensive extraction edge cases (part A of fe-cpp-comprehensive).
//!
//! Exercises the C++ symbol extractor across the constructs that real C++
//! codebases use and that thin "one class, one enum, one fn" fixtures miss:
//! namespaces (incl. nested), inheritance + virtual/pure-virtual methods,
//! nested classes, templates + specialization, function + operator overloads,
//! `enum class` vs plain `enum`, and `typedef`/`using` aliases.
//!
//! Firm positive assertions cover everything the extractor gets right. Two
//! real gaps the dump surfaced are pinned as baselines (they flip when fixed):
//! - function/method **overloads collapse** to a single symbol;
//! - **nested classes** inside a class body are not extracted.

mod langtest;

use langtest::{IngestedProject, assert_range_invariant, assert_symbol};

fn shapes_hpp() -> &'static str {
    r"#ifndef SHAPES_HPP
#define SHAPES_HPP

namespace geo {

// Abstract base with a pure-virtual method.
class Shape {
public:
    virtual ~Shape() {}
    virtual double area() const = 0;
    int id = 0;
};

// A mix-in used for multiple inheritance below.
struct Printable {
    virtual void print() const {}
};

// Derived class: multiple inheritance + overrides + a nested class.
class Circle : public Shape, public Printable {
public:
    double radius;
    explicit Circle(double r) : radius(r) {}
    double area() const override { return 3.14159 * radius * radius; }
    void print() const override {}

    class Builder {
    public:
        Circle build() const { return Circle(1.0); }
    };
};

// Scoped vs unscoped enums.
enum class Color { Red, Green, Blue };
enum Direction { North, South };

// Template class + an explicit specialization.
template <typename T>
class Box {
public:
    T value;
    T get() const { return value; }
};

template <>
class Box<bool> {
public:
    bool flag;
};

// Function overloads (same name, different signatures).
int maxv(int a, int b) { return a > b ? a : b; }
double maxv(double a, double b) { return a > b ? a : b; }

// Free operator overload + a template function.
Circle add_circles(const Circle& a, const Circle& b) {
    return Circle(a.radius + b.radius);
}

template <typename T>
T identity(T x) { return x; }

// Aliases.
typedef Box<int> IntBox;
using ColorAlias = Color;

// Nested namespace.
namespace detail {
    int helper() { return 42; }
}

} // namespace geo

#endif // SHAPES_HPP
"
}

/// Diagnostic dump (always passes) so regressions print the full picture.
#[tokio::test]
async fn cpp_edge_case_symbol_dump() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let mut symbols = proj.all_symbols().await;
    symbols.sort_by_key(|s| s.line_start);
    eprintln!("=== C++ extracted symbols ({}) ===", symbols.len());
    for s in &symbols {
        eprintln!(
            "  {:>3}..{:<3} {:<10} {:<18} vis={:?}",
            s.line_start, s.line_end, s.kind, s.name, s.visibility
        );
    }
    assert!(!symbols.is_empty(), "C++ extraction produced no symbols");
}

#[tokio::test]
async fn cpp_namespaces_and_nested_namespace() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let geo = assert_symbol(&proj, "geo", "module", "shapes.hpp").await;
    assert_range_invariant(&geo);
    assert!(
        geo.line_end > geo.line_start,
        "namespace geo spans the whole file body"
    );
    let detail = assert_symbol(&proj, "detail", "module", "shapes.hpp").await;
    assert_range_invariant(&detail);
}

#[tokio::test]
async fn cpp_classes_structs_and_inheritance() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;

    // `class` and `struct` both map to the struct kind.
    let shape = assert_symbol(&proj, "Shape", "struct", "shapes.hpp").await;
    assert_range_invariant(&shape);
    assert!(shape.line_end > shape.line_start, "Shape is multi-line");

    let printable = assert_symbol(&proj, "Printable", "struct", "shapes.hpp").await;
    assert_range_invariant(&printable);

    // Derived class with multiple inheritance.
    let circle = assert_symbol(&proj, "Circle", "struct", "shapes.hpp").await;
    assert_range_invariant(&circle);
    assert!(circle.line_end > circle.line_start, "Circle is multi-line");
}

#[tokio::test]
async fn cpp_methods_virtual_ctor_dtor() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;

    // Pure-virtual method, destructor, constructor, and overrides all extract
    // as functions.
    let _area = assert_symbol(&proj, "area", "function", "shapes.hpp").await;
    let _dtor = assert_symbol(&proj, "~Shape", "function", "shapes.hpp").await;
    let _ctor = assert_symbol(&proj, "Circle", "function", "shapes.hpp").await;
    let _print = assert_symbol(&proj, "print", "function", "shapes.hpp").await;
    let _get = assert_symbol(&proj, "get", "function", "shapes.hpp").await;
}

#[tokio::test]
async fn cpp_scoped_and_unscoped_enums() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let color = assert_symbol(&proj, "Color", "enum", "shapes.hpp").await; // enum class
    assert_range_invariant(&color);
    let _dir = assert_symbol(&proj, "Direction", "enum", "shapes.hpp").await; // plain enum
}

#[tokio::test]
async fn cpp_templates_and_specialization() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;

    // Primary template class.
    let boxt = assert_symbol(&proj, "Box", "struct", "shapes.hpp").await;
    assert_range_invariant(&boxt);

    // Explicit specialization keeps the template-argument suffix in its name.
    let spec = assert_symbol(&proj, "Box<bool>", "struct", "shapes.hpp").await;
    assert_range_invariant(&spec);

    // Template free function.
    let _identity = assert_symbol(&proj, "identity", "function", "shapes.hpp").await;
}

#[tokio::test]
async fn cpp_typedef_and_using_aliases() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let _intbox = assert_symbol(&proj, "IntBox", "type", "shapes.hpp").await; // typedef
    let _color_alias = assert_symbol(&proj, "ColorAlias", "type", "shapes.hpp").await; // using
}

// ── Pinned gap baselines (flip when fixed) ────────────────────────────────

/// BASELINE: function/method overloads collapse to a single symbol.
///
/// `int maxv(int,int)` and `double maxv(double,double)` are distinct overloads,
/// but only one `maxv` symbol survives (same name + scope → one stored symbol).
/// This also affects C++ method overloads, Java, and TS overload signatures.
/// Tracked as f-overload-symbols-collapse; flips to `== 2` when fixed.
#[tokio::test]
async fn cpp_overloads_collapse_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let maxv = proj.symbols_named("maxv").await;
    assert_eq!(
        maxv.len(),
        1,
        "C++ overloads now produce {} `maxv` symbols (expected the 1-symbol \
         collapse baseline). If overload distinction landed, assert == 2 and \
         check each signature.",
        maxv.len(),
    );
}

/// BASELINE: nested classes inside a class body are not extracted.
///
/// `Circle::Builder` (a class declared inside `Circle`) produces no symbol —
/// the extractor does not recurse into class bodies for nested type decls.
/// Tracked as f-nested-class-extraction; flips when `Builder` appears.
#[tokio::test]
async fn cpp_nested_class_not_extracted_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.hpp", shapes_hpp())]).await;
    let builder = proj.symbols_named("Builder").await;
    assert!(
        builder.is_empty(),
        "nested class `Circle::Builder` is now extracted ({} symbol(s)) — \
         promote this baseline to a positive nested-class assertion.",
        builder.len(),
    );
}
