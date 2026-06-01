//! FE — C comprehensive extraction edge cases (fe-c-comprehensive).
//!
//! Sibling to `fe_cpp_extraction.rs`, for the C grammar specifically. Fixtures
//! use a `.c` extension on purpose: when `lang-cpp` is enabled, `.h` routes to
//! the C++ grammar, so a C-only fixture must live in `.c` to exercise the C
//! path.
//!
//! Exercises the constructs thin fixtures miss: named structs, `typedef struct`
//! (tagged + anonymous), unions + `typedef union`, plain + `typedef` enums,
//! nested structs, anonymous struct/union members, function declaration vs
//! definition (same name), `static` functions, `extern` declarations,
//! function-pointer typedefs, and `#define` macros.

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref, assert_range_invariant, assert_symbol};
use ministr_core::types::RefKind;

fn shapes_c() -> &'static str {
    r"#include <stddef.h>

// Named struct.
struct Point {
    int x;
    int y;
};

// typedef struct with a tag.
typedef struct Node {
    int val;
    struct Node *next;
} Node;

// Anonymous typedef struct.
typedef struct {
    double re;
    double im;
} Complex;

// Plain union.
union Value {
    int i;
    float f;
};

// typedef union.
typedef union {
    long l;
    double d;
} Number;

// Plain enum + typedef enum.
enum Status {
    OK,
    ERR,
};

typedef enum {
    LOW,
    HIGH,
} Level;

// Nested struct (struct declared inside another struct body).
struct Outer {
    int id;
    struct Inner {
        int z;
    } inner;
};

// Anonymous union member inside a struct.
struct Wrapper {
    union {
        int as_int;
        float as_float;
    };
    int tag;
};

// Function declaration (prototype) vs definition, same name.
int compute(int a, int b);

int compute(int a, int b) {
    return a + b;
}

// static function.
static int helper(void) {
    return 1;
}

// extern declaration.
extern int global_counter;

// Function-pointer typedef.
typedef int (*Callback)(int);

void register_cb(Callback cb) {
    (void)cb;
}

// Macros (not AST symbols — characterized below).
#define MAX_ITEMS 100
#define SQUARE(x) ((x) * (x))
"
}

/// Diagnostic dump (always passes) so regressions print the full picture.
#[tokio::test]
async fn c_edge_case_symbol_dump() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    let mut symbols = proj.all_symbols().await;
    symbols.sort_by_key(|s| s.line_start);
    eprintln!("=== C extracted symbols ({}) ===", symbols.len());
    for s in &symbols {
        eprintln!(
            "  {:>3}..{:<3} {:<10} {:<18} vis={:?}",
            s.line_start, s.line_end, s.kind, s.name, s.visibility
        );
    }
    assert!(!symbols.is_empty(), "C extraction produced no symbols");
}

// ── Structs + unions ──────────────────────────────────────────────────────

#[tokio::test]
async fn c_structs_and_unions() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;

    let point = assert_symbol(&proj, "Point", "struct", "shapes.c").await;
    assert_range_invariant(&point);
    assert!(point.line_end > point.line_start, "Point is multi-line");

    let outer = assert_symbol(&proj, "Outer", "struct", "shapes.c").await;
    assert_range_invariant(&outer);

    let wrapper = assert_symbol(&proj, "Wrapper", "struct", "shapes.c").await;
    assert_range_invariant(&wrapper);

    // A plain `union` extracts under the `struct` kind (the C grammar's
    // struct/union node family is mapped together).
    let value = assert_symbol(&proj, "Value", "struct", "shapes.c").await;
    assert_range_invariant(&value);
}

// ── typedefs (struct / union / enum / fn-pointer) → kind `type` ───────────

#[tokio::test]
async fn c_typedefs_map_to_type_kind() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;

    // `typedef struct Node {...} Node;` — the typedef name extracts as `type`.
    let node = assert_symbol(&proj, "Node", "type", "shapes.c").await;
    assert_range_invariant(&node);

    // Anonymous `typedef struct {...} Complex;`.
    let complex = assert_symbol(&proj, "Complex", "type", "shapes.c").await;
    assert_range_invariant(&complex);

    // `typedef union {...} Number;`.
    assert_symbol(&proj, "Number", "type", "shapes.c").await;

    // `typedef enum {...} Level;` extracts as `type` (NOT `enum`) — the typedef
    // wrapper wins over the inner enum.
    assert_symbol(&proj, "Level", "type", "shapes.c").await;
}

// ── Enums ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c_plain_enum() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    // A non-typedef'd `enum Status {...}` keeps the `enum` kind.
    let status = assert_symbol(&proj, "Status", "enum", "shapes.c").await;
    assert_range_invariant(&status);
}

// ── Functions: definitions, static ─────────────────────────────────────────

#[tokio::test]
async fn c_functions_and_static() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    let compute = assert_symbol(&proj, "compute", "function", "shapes.c").await;
    assert_range_invariant(&compute);
    // `static` functions still extract.
    assert_symbol(&proj, "helper", "function", "shapes.c").await;
    assert_symbol(&proj, "register_cb", "function", "shapes.c").await;
}

// ── Macros: #define extracts (object → const, function-like → function) ─────

#[tokio::test]
async fn c_macros_extract() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    // Object-like macro → const.
    assert_symbol(&proj, "MAX_ITEMS", "const", "shapes.c").await;
    // Function-like macro → function.
    assert_symbol(&proj, "SQUARE", "function", "shapes.c").await;
}

// ── Pinned baselines (characterize current behavior; flip when fixed) ──────

/// BASELINE: a function-pointer typedef keeps the `(*Name)` declarator syntax
/// in its symbol name rather than the bare `Callback`. Tracked as a naming
/// refinement; flips when the extractor strips the pointer-declarator wrapper.
#[tokio::test]
async fn c_function_pointer_typedef_name_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    assert!(
        proj.symbols_named("Callback").await.is_empty(),
        "fn-pointer typedef now extracts as bare `Callback` — promote this baseline",
    );
    let cb = assert_symbol(&proj, "(*Callback)", "type", "shapes.c").await;
    assert_range_invariant(&cb);
}

/// BASELINE: a nested `struct Inner` declared inside another struct body is not
/// extracted (same gap as C++ nested classes — f-nested-class-extraction).
#[tokio::test]
async fn c_nested_struct_not_extracted_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    assert!(
        proj.symbols_named("Inner").await.is_empty(),
        "nested `struct Inner` is now extracted — promote this baseline",
    );
}

/// BASELINE: anonymous-union members (`as_int`/`as_float`) are not extracted as
/// their own symbols — only the enclosing `Wrapper` struct is.
#[tokio::test]
async fn c_anonymous_union_member_not_extracted_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    assert!(
        proj.symbols_named("as_int").await.is_empty(),
        "anonymous-union member `as_int` is now extracted — promote this baseline",
    );
}

/// BASELINE: an `extern int global_counter;` declaration is not extracted (only
/// definitions are; a bare extern variable decl produces no symbol).
#[tokio::test]
async fn c_extern_decl_not_extracted_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    assert!(
        proj.symbols_named("global_counter").await.is_empty(),
        "extern decl `global_counter` is now extracted — promote this baseline",
    );
}

/// BASELINE: a function prototype + its definition (same name) collapse to a
/// single symbol — the prototype-only declaration does not add a second
/// `compute` (related to f-overload-symbols-collapse).
#[tokio::test]
async fn c_prototype_and_definition_single_symbol_baseline() {
    let proj = IngestedProject::from_files(&[("shapes.c", shapes_c())]).await;
    let computes = proj.symbols_named("compute").await;
    assert_eq!(
        computes.len(),
        1,
        "prototype + definition now produce {} `compute` symbols (expected the \
         1-symbol collapse baseline)",
        computes.len(),
    );
}

// ── Cross-file references (both orders) ────────────────────────────────────

/// A C `Calls` edge resolves across `.c` files in BOTH ingest orders. The
/// caller's `area()` calls `square()` defined in another file; C/C++ ref
/// extraction emits a name-based `Calls` ref that the resolver binds
/// cross-file, and the deferred second pass fills it in even when the caller is
/// ingested before the definition.
#[tokio::test]
async fn c_cross_file_call_both_orders() {
    let def = "int square(int n) {\n    return n * n;\n}\n";
    let caller = "int area(int side) {\n    return square(side);\n}\n";

    // Definition-before-caller (geom.c < zmain.c).
    let def_first = IngestedProject::from_files(&[("geom.c", def), ("zmain.c", caller)]).await;
    assert_cross_file_ref(
        &def_first,
        "square",
        "geom.c",
        "zmain.c",
        Some(RefKind::Calls),
    )
    .await;

    // Caller-before-definition (amain.c < geom.c) — deferred second pass.
    let caller_first = IngestedProject::from_files(&[("amain.c", caller), ("geom.c", def)]).await;
    assert_cross_file_ref(
        &caller_first,
        "square",
        "geom.c",
        "amain.c",
        Some(RefKind::Calls),
    )
    .await;
}

/// The `#include "x.h"` shape: a header declares the prototype, one TU defines
/// the function, another `#include`s the header and calls it. The call resolves
/// cross-file to a `square` definition (characterizing whatever the C ref model
/// binds — kind left open).
#[tokio::test]
async fn c_include_header_cross_file_call() {
    let proj = IngestedProject::from_files(&[
        (
            "mathlib.c",
            "#include \"mathlib.h\"\nint cube(int n) {\n    return n * n * n;\n}\n",
        ),
        ("mathlib.h", "int cube(int n);\n"),
        (
            "app.c",
            "#include \"mathlib.h\"\nint go(int n) {\n    return cube(n);\n}\n",
        ),
    ])
    .await;
    // app.c's go() calls cube() — resolves cross-file to the definition in
    // mathlib.c (any ref kind).
    assert_cross_file_ref(&proj, "cube", "mathlib.c", "app.c", None).await;
}
