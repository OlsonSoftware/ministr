//! FE — Java edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Companion to the JS/TS edge tests. Before this slice the Java extractor was
//! import-only, so `ministr_references` / `ministr_solid` starved on Java DI
//! codebases. These tests assert SPECIFIC `RefKind::Implements` and
//! `RefKind::Calls` cross-file edges in BOTH file ingest orders (file names
//! encode order — the harness ingests in sorted path order), plus a
//! `RefKind::Uses` edge for a `new` expression.

#![cfg(feature = "lang-java")]

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
async fn java_implements_interface_both_orders() {
    // Definition-before-implementer (a_IService.java < b_Economy.java).
    assert_edge(
        &[
            ("a_IService.java", "interface IService { void run(); }\n"),
            (
                "b_Economy.java",
                "class EconomyService implements IService {\n  public void run() {}\n}\n",
            ),
        ],
        "IService",
        "a_IService.java",
        "b_Economy.java",
        RefKind::Implements,
    )
    .await;

    // Implementer-before-definition (a_Other.java < b_IOther.java).
    assert_edge(
        &[
            (
                "a_Other.java",
                "class OtherService implements IOther {\n  public void go() {}\n}\n",
            ),
            ("b_IOther.java", "interface IOther { void go(); }\n"),
        ],
        "IOther",
        "b_IOther.java",
        "a_Other.java",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn java_class_extends_cross_file() {
    // `class Derived extends Base` → an Implements edge onto the base class.
    assert_edge(
        &[
            (
                "BaseService.java",
                "class BaseService {\n  public void ping() {}\n}\n",
            ),
            (
                "DerivedService.java",
                "class DerivedService extends BaseService {}\n",
            ),
        ],
        "BaseService",
        "BaseService.java",
        "DerivedService.java",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn java_calls_cross_file_both_orders() {
    // Definition-before-caller (a_Lib.java < b_Caller.java).
    assert_edge(
        &[
            (
                "a_Lib.java",
                "class Lib {\n  static int helper() { return 1; }\n}\n",
            ),
            (
                "b_Caller.java",
                "class Caller {\n  int run() { return Lib.helper(); }\n}\n",
            ),
        ],
        "helper",
        "a_Lib.java",
        "b_Caller.java",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_Caller.java < b_Lib.java).
    assert_edge(
        &[
            (
                "a_Caller.java",
                "class Caller {\n  int run() { return Lib.compute(); }\n}\n",
            ),
            (
                "b_Lib.java",
                "class Lib {\n  static int compute() { return 2; }\n}\n",
            ),
        ],
        "compute",
        "b_Lib.java",
        "a_Caller.java",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn java_new_expression_emits_uses_cross_file() {
    // `new Widget()` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            ("Widget.java", "class Widget {\n  void draw() {}\n}\n"),
            (
                "Factory.java",
                "class Factory {\n  Widget make() { return new Widget(); }\n}\n",
            ),
        ],
        "Widget",
        "Widget.java",
        "Factory.java",
        RefKind::Uses,
    )
    .await;
}
