//! FE1 harness proof — C++ end-to-end.
//!
//! Proves the shared `langtest` harness on a real multi-file C++ project:
//! ingest → symbol extraction → reference-graph query. C++ is deliberately the
//! first language proven because its cross-file model (`#include`, not symbol
//! imports) and its `.h` header ambiguity are the most likely to expose gaps.
//!
//! Two real gaps this file pins as baselines (for FE2/FE3 to close):
//! - `.h` headers are parsed as **C**, so C++ declarations inside a `.h` are
//!   mis-extracted (see `dot_h_header_with_cpp_content_is_parsed_as_c`).
//! - C/C++ emit only `#include` refs (header *paths*), so no cross-file
//!   *symbol* edge resolves today (see `cpp_cross_file_ref_baseline`).

mod langtest;

use langtest::{IngestedProject, assert_range_invariant, assert_symbol};
use ministr_core::types::RefKind;

/// A two-file C++ project using an **unambiguous** `.hpp` header (so the C++
/// grammar is selected): a header defining the types/functions, and a
/// translation unit that `#include`s the header and uses them.
fn cpp_project() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "geometry.hpp",
            r"#ifndef GEOMETRY_HPP
#define GEOMETRY_HPP

// A point in 2D space.
class Point {
public:
    int x;
    int y;

    int magnitude_squared() const {
        return x * x + y * y;
    }
};

// Supported shape kinds.
enum Shape {
    Circle,
    Square,
    Triangle,
};

// Adds two integers.
int add(int a, int b) {
    return a + b;
}

#endif // GEOMETRY_HPP
",
        ),
        (
            "main.cpp",
            r#"#include "geometry.hpp"
#include <cstdio>

// Computes a value using Point and add() from geometry.hpp.
int compute() {
    Point p;
    p.x = 3;
    p.y = 4;
    int m = p.magnitude_squared();
    return add(m, 1);
}

int main() {
    printf("%d\n", compute());
    return 0;
}
"#,
        ),
    ]
}

#[tokio::test]
async fn cpp_extraction_end_to_end() {
    let proj = IngestedProject::from_files(&cpp_project()).await;

    // Header symbols (class → struct, enum → enum, free function).
    let point = assert_symbol(&proj, "Point", "struct", "geometry.hpp").await;
    assert_range_invariant(&point);
    assert!(
        point.line_end > point.line_start,
        "Point is a multi-line class; its range must span >1 line, got {}..{}",
        point.line_start,
        point.line_end,
    );

    let shape = assert_symbol(&proj, "Shape", "enum", "geometry.hpp").await;
    assert_range_invariant(&shape);

    let add = assert_symbol(&proj, "add", "function", "geometry.hpp").await;
    assert_range_invariant(&add);
    assert!(
        add.line_end > add.line_start,
        "add() has a body; its range must span >1 line, got {}..{}",
        add.line_start,
        add.line_end,
    );

    // Translation-unit symbols.
    let compute = assert_symbol(&proj, "compute", "function", "main.cpp").await;
    assert_range_invariant(&compute);
    let _main = assert_symbol(&proj, "main", "function", "main.cpp").await;
}

/// Characterizes C++ cross-file reference resolution as it stands today.
///
/// `main.cpp` calls `add()` and uses `Point`, both defined in `geometry.hpp`.
/// In a fully-resolved graph there would be a cross-file edge
/// `main.cpp → geometry.hpp` for each. Today C/C++ ref extraction emits only
/// `#include` references (whose target is the header *path*, not a symbol), so
/// **no cross-file symbol edge resolves**. This test pins that baseline so FE3
/// (C++ cross-file call/use resolution) has a precise before/after, and proves
/// the harness ref surface itself works on C++ (returns a coherent edge set).
#[tokio::test]
async fn cpp_cross_file_ref_baseline() {
    let proj = IngestedProject::from_files(&cpp_project()).await;

    let add_edges = proj.refs_into("add", None).await;
    let point_edges = proj.refs_into("Point", None).await;

    // BASELINE (current behavior): C/C++ produce no cross-file symbol edges.
    // When FE3 adds call/use resolution this assertion flips — update it then.
    assert!(
        add_edges.is_empty() && point_edges.is_empty(),
        "C++ cross-file symbol edges unexpectedly resolved (add={}, Point={}). \
         If FE3 landed C++ call/use resolution, convert this baseline into a \
         positive assert_cross_file_ref check.",
        add_edges.len(),
        point_edges.len(),
    );

    // Whatever (if anything) resolves must be internally consistent.
    for e in &add_edges {
        assert_eq!(e.to_name, "add");
        assert!(matches!(
            e.kind,
            RefKind::Calls | RefKind::Uses | RefKind::Imports
        ));
    }
}

/// Pins the `.h`-header ambiguity as a baseline: a `.h` file containing C++
/// (`class` with an inline method) is parsed with the **C** grammar (the
/// registry maps `.h` → C, `.hpp/.hxx/.hh` → C++). Under the C grammar the
/// C++ `class` is mis-extracted — notably **not** as a `struct`. Most
/// real-world C++ ships declarations in `.h`, so this is a meaningful coverage
/// gap (tracked for FE2). When `.h` disambiguation lands, this test flips.
#[tokio::test]
async fn dot_h_header_with_cpp_content_is_parsed_as_c() {
    let proj = IngestedProject::from_files(&[(
        "widget.h",
        r"// A C++ class living in a .h header (the common real-world case).
class Widget {
public:
    int id;
    int doubled() const { return id * 2; }
};
",
    )])
    .await;

    let widgets = proj.symbols_named("Widget").await;
    // Baseline: under the C grammar, `class Widget` is NOT recognized as a
    // struct. (It currently surfaces as a function-like node.) Asserting the
    // negative keeps the test honest without over-fitting the exact wrong kind.
    let as_struct = widgets.iter().any(|s| s.kind == "struct");
    assert!(
        !as_struct,
        "`.h` header now extracts the C++ class `Widget` as a struct — \
         the `.h`→C++ disambiguation gap appears fixed. Promote this baseline \
         to a positive C++ extraction assertion (and add `.h` to the C++ matrix)."
    );
}
