//! FE — Swift edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Companion to the JS/TS, Java, C#, and Kotlin edge tests. Before this slice
//! the Swift extractor was import-only, so `ministr_references` /
//! `ministr_solid` starved on Swift codebases. These tests assert SPECIFIC
//! `RefKind::Implements` and `RefKind::Calls` cross-file edges in BOTH file
//! ingest orders, plus a `RefKind::Uses` edge for a type annotation. Swift
//! lists the base class and conformed protocols together (one inheritance
//! clause), and has no `new` (construction is a plain call), so the `Uses`
//! edge is asserted via a return type rather than a constructor.

#![cfg(feature = "lang-swift")]

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
async fn swift_conforms_protocol_both_orders() {
    // Definition-before-conformer (a_IService.swift < b_Economy.swift).
    assert_edge(
        &[
            ("a_IService.swift", "protocol IService { func run() }\n"),
            (
                "b_Economy.swift",
                "class EconomyService : IService {\n  func run() {}\n}\n",
            ),
        ],
        "IService",
        "a_IService.swift",
        "b_Economy.swift",
        RefKind::Implements,
    )
    .await;

    // Conformer-before-definition (a_Other.swift < b_IOther.swift).
    assert_edge(
        &[
            (
                "a_Other.swift",
                "class OtherService : IOther {\n  func go() {}\n}\n",
            ),
            ("b_IOther.swift", "protocol IOther { func go() }\n"),
        ],
        "IOther",
        "b_IOther.swift",
        "a_Other.swift",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn swift_base_class_cross_file() {
    // `class Derived : BaseService` → an Implements edge onto the base class
    // (Swift puts the base class in the same inheritance clause as protocols).
    assert_edge(
        &[
            (
                "BaseService.swift",
                "class BaseService {\n  func ping() {}\n}\n",
            ),
            (
                "DerivedService.swift",
                "class DerivedService : BaseService {}\n",
            ),
        ],
        "BaseService",
        "BaseService.swift",
        "DerivedService.swift",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn swift_calls_cross_file_both_orders() {
    // Definition-before-caller (a_Lib.swift < b_Caller.swift).
    assert_edge(
        &[
            ("a_Lib.swift", "func helper() -> Int { return 1 }\n"),
            (
                "b_Caller.swift",
                "func run() -> Int {\n  return helper()\n}\n",
            ),
        ],
        "helper",
        "a_Lib.swift",
        "b_Caller.swift",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_Caller.swift < b_Lib.swift).
    assert_edge(
        &[
            (
                "a_Caller.swift",
                "func run() -> Int {\n  return compute()\n}\n",
            ),
            ("b_Lib.swift", "func compute() -> Int { return 2 }\n"),
        ],
        "compute",
        "b_Lib.swift",
        "a_Caller.swift",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn swift_type_annotation_emits_uses_cross_file() {
    // A `-> Widget` return type → a Uses edge onto the named type (Swift has
    // no `new`, so Uses comes from a type position, not a constructor).
    assert_edge(
        &[
            ("Widget.swift", "class Widget {\n  func draw() {}\n}\n"),
            (
                "Factory.swift",
                "class Factory {\n  func make() -> Widget {\n    return Widget()\n  }\n}\n",
            ),
        ],
        "Widget",
        "Widget.swift",
        "Factory.swift",
        RefKind::Uses,
    )
    .await;
}
