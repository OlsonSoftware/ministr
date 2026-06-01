//! FE — Kotlin edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Companion to the JS/TS, Java, and C# edge tests. Before this slice the
//! Kotlin extractor was import-only, so `ministr_references` / `ministr_solid`
//! starved on Kotlin DI codebases. These tests assert SPECIFIC
//! `RefKind::Implements` and `RefKind::Calls` cross-file edges in BOTH file
//! ingest orders, plus a `RefKind::Uses` edge for a type annotation. Kotlin
//! merges the base class and interfaces into one delegation-specifier list,
//! and has no `new` (construction is a plain call), so the `Uses` edge is
//! asserted via a property/return type rather than a constructor.

#![cfg(feature = "lang-kotlin")]

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
async fn kotlin_implements_interface_both_orders() {
    // Definition-before-implementer (a_IService.kt < b_Economy.kt).
    assert_edge(
        &[
            ("a_IService.kt", "interface IService { fun run() }\n"),
            (
                "b_Economy.kt",
                "class EconomyService : IService {\n  override fun run() {}\n}\n",
            ),
        ],
        "IService",
        "a_IService.kt",
        "b_Economy.kt",
        RefKind::Implements,
    )
    .await;

    // Implementer-before-definition (a_Other.kt < b_IOther.kt).
    assert_edge(
        &[
            (
                "a_Other.kt",
                "class OtherService : IOther {\n  override fun go() {}\n}\n",
            ),
            ("b_IOther.kt", "interface IOther { fun go() }\n"),
        ],
        "IOther",
        "b_IOther.kt",
        "a_Other.kt",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn kotlin_base_class_cross_file() {
    // `class Derived : BaseService()` → an Implements edge onto the base class
    // (the base-class constructor invocation lives in the delegation list).
    assert_edge(
        &[
            (
                "BaseService.kt",
                "open class BaseService {\n  fun ping() {}\n}\n",
            ),
            (
                "DerivedService.kt",
                "class DerivedService : BaseService()\n",
            ),
        ],
        "BaseService",
        "BaseService.kt",
        "DerivedService.kt",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn kotlin_calls_cross_file_both_orders() {
    // Definition-before-caller (a_Lib.kt < b_Caller.kt).
    assert_edge(
        &[
            ("a_Lib.kt", "fun helper(): Int { return 1 }\n"),
            ("b_Caller.kt", "fun run(): Int {\n  return helper()\n}\n"),
        ],
        "helper",
        "a_Lib.kt",
        "b_Caller.kt",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_Caller.kt < b_Lib.kt).
    assert_edge(
        &[
            ("a_Caller.kt", "fun run(): Int {\n  return compute()\n}\n"),
            ("b_Lib.kt", "fun compute(): Int { return 2 }\n"),
        ],
        "compute",
        "b_Lib.kt",
        "a_Caller.kt",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn kotlin_type_annotation_emits_uses_cross_file() {
    // A `: Widget` return type → a Uses edge onto the named type (Kotlin has
    // no `new`, so Uses comes from a type position, not a constructor).
    assert_edge(
        &[
            ("Widget.kt", "class Widget {\n  fun draw() {}\n}\n"),
            (
                "Factory.kt",
                "class Factory {\n  fun make(): Widget {\n    return Widget()\n  }\n}\n",
            ),
        ],
        "Widget",
        "Widget.kt",
        "Factory.kt",
        RefKind::Uses,
    )
    .await;
}
