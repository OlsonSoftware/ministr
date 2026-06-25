//! C++ end-to-end harness proof.
//!
//! Proves the shared `langtest` harness on a real multi-file C++ project:
//! ingest → symbol extraction → reference-graph query. C++ is deliberately the
//! first language proven because its cross-file model (`#include`, not symbol
//! imports) and its `.h` header ambiguity were the most likely to expose gaps.
//!
//! Two real gaps the harness surfaced here are now FIXED and guarded by
//! positive assertions:
//! - `.h` headers are treated as C **and** C++ (routed to the C++ grammar),
//!   so a C++ class in a `.h` extracts correctly
//!   (`dot_h_header_with_cpp_content_extracts_as_cpp`).
//! - C/C++ now emit call + type-use refs, so cross-file *symbol* edges resolve
//!   in both file orders (`cpp_cross_file_refs_resolve`,
//!   `cpp_cross_file_ref_importer_before_definition`).

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref, assert_range_invariant, assert_symbol};
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

/// C++ cross-file references resolve via call + type-use edges.
///
/// `main.cpp` calls `add()` and uses `Point`, both defined in `geometry.hpp`.
/// C/C++ ref extraction now emits `Calls`/`Uses` refs (not just `#include`
/// paths), and the unified c/cpp ref family lets them resolve across files.
/// This is the definition-before-importer order (`geometry.hpp` < `main.cpp`).
#[tokio::test]
async fn cpp_cross_file_refs_resolve() {
    let proj = IngestedProject::from_files(&cpp_project()).await;

    // Call edge: main.cpp's compute() calls add() in geometry.hpp.
    assert_cross_file_ref(
        &proj,
        "add",
        "geometry.hpp",
        "main.cpp",
        Some(RefKind::Calls),
    )
    .await;

    // Type-use edge: main.cpp's compute() declares a `Point` from geometry.hpp.
    assert_cross_file_ref(
        &proj,
        "Point",
        "geometry.hpp",
        "main.cpp",
        Some(RefKind::Uses),
    )
    .await;
}

/// The same C++ cross-file edge must resolve in the **importer-before-definition**
/// order (the deferred second-pass case that was the real "no related files"
/// bug for TS). File names are chosen so the caller sorts first
/// (`a_caller.cpp` < `z_geometry.hpp`) and is therefore ingested before the
/// definition file.
#[tokio::test]
async fn cpp_cross_file_ref_importer_before_definition() {
    let proj = IngestedProject::from_files(&[
        (
            "a_caller.cpp",
            r#"#include "z_geometry.hpp"

int run() {
    return ping(7);
}
"#,
        ),
        (
            "z_geometry.hpp",
            r"#ifndef Z_GEOMETRY_HPP
#define Z_GEOMETRY_HPP

int ping(int n) {
    return n + 1;
}

#endif
",
        ),
    ])
    .await;

    assert_cross_file_ref(
        &proj,
        "ping",
        "z_geometry.hpp",
        "a_caller.cpp",
        Some(RefKind::Calls),
    )
    .await;
}

/// `.h` headers are treated as C **and** C++: a `.h` file containing a C++
/// class extracts the class as a `struct` (routed to the C++ grammar, a
/// near-superset of C). Promoted from the former `.h`-parsed-as-C baseline.
#[tokio::test]
async fn dot_h_header_with_cpp_content_extracts_as_cpp() {
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

    let widget = assert_symbol(&proj, "Widget", "struct", "widget.h").await;
    assert_range_invariant(&widget);
    // The member method is extracted too (C++ class body, not a C function).
    let _doubled = assert_symbol(&proj, "doubled", "function", "widget.h").await;
}
