//! FE — Scala edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Before this slice the Scala extractor was import-only, so `ministr_references`
//! / `ministr_solid` starved on Scala codebases. Scala is explicit-OO: a class /
//! trait / object lists its base AND every `with` mixin trait under one
//! `extends_clause`, so both are `Implements` signals. These tests assert
//! SPECIFIC `RefKind::Implements` and `RefKind::Calls` cross-file edges in BOTH
//! file ingest orders, a `with`-mixin Implements onto a trait, plus a
//! `RefKind::Uses` edge for a `new` instance expression.

#![cfg(feature = "lang-scala")]

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
async fn scala_extends_class_both_orders() {
    // Definition-before-subclass (a_base.scala < b_economy.scala).
    assert_edge(
        &[
            ("a_base.scala", "class BaseService {\n  def ping(): Unit = {}\n}\n"),
            (
                "b_economy.scala",
                "class EconomyService extends BaseService {\n  def run(): Unit = {}\n}\n",
            ),
        ],
        "BaseService",
        "a_base.scala",
        "b_economy.scala",
        RefKind::Implements,
    )
    .await;

    // Subclass-before-definition (a_economy.scala < b_base.scala).
    assert_edge(
        &[
            (
                "a_economy.scala",
                "class OtherService extends OtherBase {\n  def go(): Unit = {}\n}\n",
            ),
            ("b_base.scala", "class OtherBase {\n  def go(): Unit = {}\n}\n"),
        ],
        "OtherBase",
        "b_base.scala",
        "a_economy.scala",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn scala_with_trait_mixin_implements_cross_file() {
    // `class C extends Service2 with IService` → an Implements edge onto the
    // `with` mixin trait (the second slot of the extends_clause). Proves the
    // trait resolves as an Implements target (unlike Ruby's module gap).
    assert_edge(
        &[
            (
                "a_iservice.scala",
                "trait IService {\n  def run(): Unit\n}\n",
            ),
            (
                "b_economy.scala",
                "class EconomyService extends Service2 with IService {\n  def run(): Unit = {}\n}\n",
            ),
        ],
        "IService",
        "a_iservice.scala",
        "b_economy.scala",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn scala_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.scala < b_caller.scala).
    assert_edge(
        &[
            ("a_lib.scala", "object Lib {\n  def helper(): Int = 1\n}\n"),
            (
                "b_caller.scala",
                "object Caller {\n  def run(): Int = helper()\n}\n",
            ),
        ],
        "helper",
        "a_lib.scala",
        "b_caller.scala",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.scala < b_lib.scala).
    assert_edge(
        &[
            (
                "a_caller.scala",
                "object Caller {\n  def run(): Int = compute()\n}\n",
            ),
            ("b_lib.scala", "object Lib2 {\n  def compute(): Int = 2\n}\n"),
        ],
        "compute",
        "b_lib.scala",
        "a_caller.scala",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn scala_new_emits_uses_cross_file() {
    // `new Widget()` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            ("widget.scala", "class Widget {\n  def draw(): Unit = {}\n}\n"),
            (
                "factory.scala",
                "class Factory {\n  def make(): Widget = new Widget()\n}\n",
            ),
        ],
        "Widget",
        "widget.scala",
        "factory.scala",
        RefKind::Uses,
    )
    .await;
}
