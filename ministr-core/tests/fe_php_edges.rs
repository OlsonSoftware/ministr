//! FE — PHP edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Before this slice the PHP extractor was import-only, so `ministr_references`
//! / `ministr_solid` starved on PHP codebases. PHP is explicit-OO with both
//! `extends` and `implements`. These tests assert SPECIFIC `RefKind::Implements`
//! and `RefKind::Calls` cross-file edges in BOTH file ingest orders, plus a
//! `RefKind::Uses` edge for a `new` expression.

#![cfg(feature = "lang-php")]

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
async fn php_implements_interface_both_orders() {
    // Definition-before-implementer (a_IService.php < b_Economy.php).
    assert_edge(
        &[
            (
                "a_IService.php",
                "<?php\ninterface IService { public function run(); }\n",
            ),
            (
                "b_Economy.php",
                "<?php\nclass EconomyService implements IService {\n    public function run() {}\n}\n",
            ),
        ],
        "IService",
        "a_IService.php",
        "b_Economy.php",
        RefKind::Implements,
    )
    .await;

    // Implementer-before-definition (a_Other.php < b_IOther.php).
    assert_edge(
        &[
            (
                "a_Other.php",
                "<?php\nclass OtherService implements IOther {\n    public function go() {}\n}\n",
            ),
            (
                "b_IOther.php",
                "<?php\ninterface IOther { public function go(); }\n",
            ),
        ],
        "IOther",
        "b_IOther.php",
        "a_Other.php",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn php_extends_base_class_cross_file() {
    // `class Derived extends BaseService` → an Implements edge onto the base.
    assert_edge(
        &[
            (
                "BaseService.php",
                "<?php\nclass BaseService {\n    public function ping() {}\n}\n",
            ),
            (
                "DerivedService.php",
                "<?php\nclass DerivedService extends BaseService {}\n",
            ),
        ],
        "BaseService",
        "BaseService.php",
        "DerivedService.php",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn php_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.php < b_caller.php).
    assert_edge(
        &[
            ("a_lib.php", "<?php\nfunction helper() { return 1; }\n"),
            (
                "b_caller.php",
                "<?php\nfunction run() { return helper(); }\n",
            ),
        ],
        "helper",
        "a_lib.php",
        "b_caller.php",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.php < b_lib.php).
    assert_edge(
        &[
            (
                "a_caller.php",
                "<?php\nfunction run() { return compute(); }\n",
            ),
            ("b_lib.php", "<?php\nfunction compute() { return 2; }\n"),
        ],
        "compute",
        "b_lib.php",
        "a_caller.php",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn php_new_expression_emits_uses_cross_file() {
    // `new Widget()` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            (
                "Widget.php",
                "<?php\nclass Widget {\n    public function draw() {}\n}\n",
            ),
            (
                "Factory.php",
                "<?php\nclass Factory {\n    public function make() { return new Widget(); }\n}\n",
            ),
        ],
        "Widget",
        "Widget.php",
        "Factory.php",
        RefKind::Uses,
    )
    .await;
}
