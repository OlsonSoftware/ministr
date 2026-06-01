//! FE — Go edge-graph (Calls / Uses) cross-file tests.
//!
//! Before this slice the Go extractor was import-only, so `ministr_references`
//! / `ministr_solid` starved on Go codebases. Go is the ODD ONE of the
//! edge-graph rollout: interface conformance is STRUCTURAL (implicit — a type
//! never names the interface it satisfies), there is no inheritance, and no
//! `new`. So Go emits `Calls` + `Uses` only — NO `Implements` (there is no
//! such signal in the AST). These tests assert SPECIFIC `RefKind::Calls`
//! (function call) and `RefKind::Uses` (composite literal + declared type
//! position) cross-file edges in BOTH file ingest orders.

#![cfg(feature = "lang-go")]

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};
use ministr_core::types::RefKind;

async fn assert_edge(
    files: &[(&str, &str)],
    target: &str,
    def_suffix: &str,
    importer_suffix: &str,
    kind: RefKind,
) {
    let proj = IngestedProject::from_files(files).await;
    assert_cross_file_ref(&proj, target, def_suffix, importer_suffix, Some(kind)).await;
}

#[tokio::test]
async fn go_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.go < b_caller.go).
    assert_edge(
        &[
            ("a_lib.go", "package demo\n\nfunc Helper() int {\n\treturn 1\n}\n"),
            (
                "b_caller.go",
                "package demo\n\nfunc Run() int {\n\treturn Helper()\n}\n",
            ),
        ],
        "Helper",
        "a_lib.go",
        "b_caller.go",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.go < b_lib.go).
    assert_edge(
        &[
            (
                "a_caller.go",
                "package demo\n\nfunc Run() int {\n\treturn Compute()\n}\n",
            ),
            ("b_lib.go", "package demo\n\nfunc Compute() int {\n\treturn 2\n}\n"),
        ],
        "Compute",
        "b_lib.go",
        "a_caller.go",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn go_composite_literal_emits_uses_cross_file() {
    // `Widget{...}` composite literal → a Uses edge onto the constructed type
    // (Go has no `new`; struct values are composite literals).
    assert_edge(
        &[
            (
                "widget.go",
                "package demo\n\ntype Widget struct {\n\tName string\n}\n",
            ),
            (
                "factory.go",
                "package demo\n\nfunc Make() Widget {\n\treturn Widget{Name: \"a\"}\n}\n",
            ),
        ],
        "Widget",
        "widget.go",
        "factory.go",
        RefKind::Uses,
    )
    .await;
}

#[tokio::test]
async fn go_parameter_type_emits_uses_cross_file() {
    // A `Shape` parameter type → a Uses edge onto the named type (a declared
    // type position distinct from a composite literal).
    assert_edge(
        &[
            ("shape.go", "package demo\n\ntype Shape struct {\n\tSides int\n}\n"),
            (
                "draw.go",
                "package demo\n\nfunc Draw(s Shape) int {\n\treturn s.Sides\n}\n",
            ),
        ],
        "Shape",
        "shape.go",
        "draw.go",
        RefKind::Uses,
    )
    .await;
}
