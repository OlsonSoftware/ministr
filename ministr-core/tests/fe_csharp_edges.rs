//! FE — C# edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Companion to the JS/TS and Java edge tests. Before this slice the C#
//! extractor was import-only, so `ministr_references` / `ministr_solid`
//! starved on C# DI codebases. These tests assert SPECIFIC
//! `RefKind::Implements` and `RefKind::Calls` cross-file edges in BOTH file
//! ingest orders, plus a `RefKind::Uses` edge for a `new` expression. C#
//! merges the base class and interfaces into one `base_list`, so both
//! `extends`-style and `implements`-style heritage are `Implements`.

#![cfg(feature = "lang-csharp")]

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
async fn csharp_implements_interface_both_orders() {
    // Definition-before-implementer (a_IService.cs < b_Economy.cs).
    assert_edge(
        &[
            ("a_IService.cs", "interface IService { void Run(); }\n"),
            (
                "b_Economy.cs",
                "class EconomyService : IService {\n  public void Run() {}\n}\n",
            ),
        ],
        "IService",
        "a_IService.cs",
        "b_Economy.cs",
        RefKind::Implements,
    )
    .await;

    // Implementer-before-definition (a_Other.cs < b_IOther.cs).
    assert_edge(
        &[
            (
                "a_Other.cs",
                "class OtherService : IOther {\n  public void Go() {}\n}\n",
            ),
            ("b_IOther.cs", "interface IOther { void Go(); }\n"),
        ],
        "IOther",
        "b_IOther.cs",
        "a_Other.cs",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn csharp_base_class_cross_file() {
    // `class Derived : BaseService` → an Implements edge onto the base class
    // (C# uses one base_list for both the base class and interfaces).
    assert_edge(
        &[
            (
                "BaseService.cs",
                "class BaseService {\n  public void Ping() {}\n}\n",
            ),
            (
                "DerivedService.cs",
                "class DerivedService : BaseService {}\n",
            ),
        ],
        "BaseService",
        "BaseService.cs",
        "DerivedService.cs",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn csharp_calls_cross_file_both_orders() {
    // Definition-before-caller (a_Lib.cs < b_Caller.cs).
    assert_edge(
        &[
            (
                "a_Lib.cs",
                "class Lib {\n  public static int Helper() { return 1; }\n}\n",
            ),
            (
                "b_Caller.cs",
                "class Caller {\n  int Run() { return Lib.Helper(); }\n}\n",
            ),
        ],
        "Helper",
        "a_Lib.cs",
        "b_Caller.cs",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_Caller.cs < b_Lib.cs).
    assert_edge(
        &[
            (
                "a_Caller.cs",
                "class Caller {\n  int Run() { return Lib.Compute(); }\n}\n",
            ),
            (
                "b_Lib.cs",
                "class Lib {\n  public static int Compute() { return 2; }\n}\n",
            ),
        ],
        "Compute",
        "b_Lib.cs",
        "a_Caller.cs",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn csharp_new_expression_emits_uses_cross_file() {
    // `new Widget()` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            ("Widget.cs", "class Widget {\n  public void Draw() {}\n}\n"),
            (
                "Factory.cs",
                "class Factory {\n  Widget Make() { return new Widget(); }\n}\n",
            ),
        ],
        "Widget",
        "Widget.cs",
        "Factory.cs",
        RefKind::Uses,
    )
    .await;
}
