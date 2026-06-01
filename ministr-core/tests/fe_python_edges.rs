//! FE — Python edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! First of the dynamic tail. Before this slice the Python extractor was
//! import-only, so `ministr_references` / `ministr_solid` starved on Python
//! codebases. Python has no interfaces, so a base class is the conformance
//! signal (`class C(Base)` → `Implements(Base)`; ABCs are Python's de-facto
//! interfaces). These tests assert SPECIFIC `RefKind::Implements` and
//! `RefKind::Calls` cross-file edges in BOTH file ingest orders, plus a
//! `RefKind::Uses` edge for a type annotation (Python has no `new`, so Uses
//! comes from an annotation rather than a constructor).

#![cfg(feature = "lang-python")]

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
async fn python_base_class_both_orders() {
    // Definition-before-subclass (a_base.py < b_economy.py).
    assert_edge(
        &[
            ("a_base.py", "class IService:\n    def run(self): ...\n"),
            (
                "b_economy.py",
                "from a_base import IService\nclass EconomyService(IService):\n    def run(self): pass\n",
            ),
        ],
        "IService",
        "a_base.py",
        "b_economy.py",
        RefKind::Implements,
    )
    .await;

    // Subclass-before-definition (a_economy.py < b_base.py).
    assert_edge(
        &[
            (
                "a_economy.py",
                "from b_base import IOther\nclass OtherService(IOther):\n    def go(self): pass\n",
            ),
            ("b_base.py", "class IOther:\n    def go(self): ...\n"),
        ],
        "IOther",
        "b_base.py",
        "a_economy.py",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn python_multiple_bases_cross_file() {
    // `class D(A, B)` → an Implements edge onto each positional base
    // (the `metaclass=` keyword arg is ignored).
    assert_edge(
        &[
            ("base_a.py", "class BaseService:\n    def ping(self): ...\n"),
            (
                "derived.py",
                "from base_a import BaseService\nclass DerivedService(BaseService, object):\n    pass\n",
            ),
        ],
        "BaseService",
        "base_a.py",
        "derived.py",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn python_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.py < b_caller.py).
    assert_edge(
        &[
            ("a_lib.py", "def helper():\n    return 1\n"),
            (
                "b_caller.py",
                "from a_lib import helper\ndef run():\n    return helper()\n",
            ),
        ],
        "helper",
        "a_lib.py",
        "b_caller.py",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.py < b_lib.py).
    assert_edge(
        &[
            (
                "a_caller.py",
                "from b_lib import compute\ndef run():\n    return compute()\n",
            ),
            ("b_lib.py", "def compute():\n    return 2\n"),
        ],
        "compute",
        "b_lib.py",
        "a_caller.py",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn python_type_annotation_emits_uses_cross_file() {
    // A `-> Widget` return annotation → a Uses edge onto the named type
    // (Python has no `new`, so Uses comes from a type position).
    assert_edge(
        &[
            ("widget.py", "class Widget:\n    def draw(self): ...\n"),
            (
                "factory.py",
                "from widget import Widget\ndef make() -> Widget:\n    return Widget()\n",
            ),
        ],
        "Widget",
        "widget.py",
        "factory.py",
        RefKind::Uses,
    )
    .await;
}
