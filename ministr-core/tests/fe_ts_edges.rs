//! FE — TypeScript edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Companion to `fe_ts_refs.rs` (which covers `import` resolution). Before
//! this slice the JS/TS extractor was import-only, so `ministr_references` /
//! `ministr_solid` starved on real DI codebases (`class X implements IX`
//! produced zero `Implements` edges). These tests assert SPECIFIC
//! `RefKind::Implements` and `RefKind::Calls` cross-file edges in BOTH file
//! ingest orders (file names encode order — the harness ingests in sorted
//! path order), plus a `RefKind::Uses` edge for a `new` expression.

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};
use ministr_core::types::RefKind;

/// Ingest `files` and assert a cross-file edge of `kind` into `target`
/// (defined in `def_suffix`) originating from a symbol in `importer_suffix`.
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
async fn ts_implements_interface_both_orders() {
    // Definition-before-implementer (a_iface.ts < b_economy.ts).
    assert_edge(
        &[
            ("a_iface.ts", "export interface IService { run(): void; }\n"),
            (
                "b_economy.ts",
                "import { IService } from './a_iface';\n\
                 export class EconomyService implements IService {\n  run(): void {}\n}\n",
            ),
        ],
        "IService",
        "a_iface.ts",
        "b_economy.ts",
        RefKind::Implements,
    )
    .await;

    // Implementer-before-definition (a_economy.ts < b_iface.ts) — the deferred
    // second-pass resolution path.
    assert_edge(
        &[
            (
                "a_economy.ts",
                "import { IOther } from './b_iface';\n\
                 export class OtherService implements IOther {\n  go(): void {}\n}\n",
            ),
            ("b_iface.ts", "export interface IOther { go(): void; }\n"),
        ],
        "IOther",
        "b_iface.ts",
        "a_economy.ts",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn ts_class_extends_cross_file() {
    // `class Derived extends Base` → an Implements edge onto the base class.
    assert_edge(
        &[
            (
                "base.ts",
                "export class BaseService {\n  ping(): void {}\n}\n",
            ),
            (
                "derived.ts",
                "import { BaseService } from './base';\n\
                 export class DerivedService extends BaseService {}\n",
            ),
        ],
        "BaseService",
        "base.ts",
        "derived.ts",
        RefKind::Implements,
    )
    .await;
}

#[tokio::test]
async fn ts_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.ts < b_caller.ts).
    assert_edge(
        &[
            (
                "a_lib.ts",
                "export function helper(): number { return 1; }\n",
            ),
            (
                "b_caller.ts",
                "import { helper } from './a_lib';\n\
                 export function run(): number { return helper(); }\n",
            ),
        ],
        "helper",
        "a_lib.ts",
        "b_caller.ts",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.ts < b_lib.ts).
    assert_edge(
        &[
            (
                "a_caller.ts",
                "import { compute } from './b_lib';\n\
                 export function run(): number { return compute(); }\n",
            ),
            (
                "b_lib.ts",
                "export function compute(): number { return 2; }\n",
            ),
        ],
        "compute",
        "b_lib.ts",
        "a_caller.ts",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn ts_new_expression_emits_uses_cross_file() {
    // `new Widget()` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            ("widget.ts", "export class Widget {\n  draw(): void {}\n}\n"),
            (
                "factory.ts",
                "import { Widget } from './widget';\n\
                 export function make(): Widget { return new Widget(); }\n",
            ),
        ],
        "Widget",
        "widget.ts",
        "factory.ts",
        RefKind::Uses,
    )
    .await;
}
