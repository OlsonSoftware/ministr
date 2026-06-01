//! FE — TypeScript cross-file reference-graph edge cases.
//!
//! The core of the language suite: for every import shape, a 2+-file fixture
//! where one file imports + uses a symbol defined in another, asserting the
//! cross-file edge resolves. Built on the FE1 harness and the `js_ts`
//! ref-family fix (`.tsx`/`.ts`/`.js` are one resolution family).
//!
//! Resolution is tested in BOTH file ingest orders for the headline cases —
//! importer-before-definition is the deferred second-pass path that was the
//! real "no related files" bug. File names encode the order (the harness
//! ingests a directory in sorted path order).
//!
//! Two member-access-through-an-object shapes remain gaps and are pinned as
//! flip-when-fixed baselines: namespace-member access (`import * as ns;
//! ns.x()`) and dynamic `import()`. Both need module-member resolution the
//! name-based resolver doesn't do (tracked: f-ts-namespace-dynamic-import-refs).

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};

/// Ingest `files` and assert a symbol named `target` (defined in
/// `def_suffix`) is referenced from `importer_suffix`.
async fn assert_resolves(
    files: &[(&str, &str)],
    target: &str,
    def_suffix: &str,
    importer_suffix: &str,
) {
    let proj = IngestedProject::from_files(files).await;
    assert_cross_file_ref(&proj, target, def_suffix, importer_suffix, None).await;
}

#[tokio::test]
async fn ts_named_import_both_orders() {
    // Definition-before-importer (lib.ts < zapp.ts).
    assert_resolves(
        &[
            ("lib.ts", "export function foo() { return 1; }\n"),
            (
                "zapp.ts",
                "import { foo } from './lib';\nexport const x = foo();\n",
            ),
        ],
        "foo",
        "lib.ts",
        "zapp.ts",
    )
    .await;

    // Importer-before-definition (aaa.ts < lib.ts) — deferred second pass.
    assert_resolves(
        &[
            (
                "aaa.ts",
                "import { bar } from './lib';\nexport const x = bar();\n",
            ),
            ("lib.ts", "export function bar() { return 1; }\n"),
        ],
        "bar",
        "lib.ts",
        "aaa.ts",
    )
    .await;
}

#[tokio::test]
async fn ts_aliased_import_resolves() {
    // `{ baz as qux }` resolves to the original exported name `baz`.
    assert_resolves(
        &[
            ("lib.ts", "export function baz() { return 1; }\n"),
            (
                "zapp.ts",
                "import { baz as qux } from './lib';\nexport const x = qux();\n",
            ),
        ],
        "baz",
        "lib.ts",
        "zapp.ts",
    )
    .await;
}

#[tokio::test]
async fn ts_default_import_resolves() {
    assert_resolves(
        &[
            ("lib.ts", "export default function real() { return 1; }\n"),
            (
                "zapp.ts",
                "import real from './lib';\nexport const x = real();\n",
            ),
        ],
        "real",
        "lib.ts",
        "zapp.ts",
    )
    .await;
}

#[tokio::test]
async fn ts_type_only_import_resolves() {
    assert_resolves(
        &[
            ("lib.ts", "export interface Thing { id: number; }\n"),
            (
                "zapp.ts",
                "import type { Thing } from './lib';\nexport const x: Thing = { id: 1 };\n",
            ),
        ],
        "Thing",
        "lib.ts",
        "zapp.ts",
    )
    .await;
}

#[tokio::test]
async fn ts_reexport_hop_resolves() {
    // app imports from a re-exporter that pulls from the real definition.
    assert_resolves(
        &[
            ("lib.ts", "export function hop() { return 1; }\n"),
            ("mid.ts", "export { hop } from './lib';\n"),
            (
                "zapp.ts",
                "import { hop } from './mid';\nexport const x = hop();\n",
            ),
        ],
        "hop",
        "lib.ts",
        "zapp.ts",
    )
    .await;
}

#[tokio::test]
async fn ts_tsx_to_ts_both_orders() {
    // Importer-before-definition (button.tsx < utils.ts) — the cn regression.
    assert_resolves(
        &[
            (
                "button.tsx",
                "import { cn } from './utils';\nexport function B() { return cn('a'); }\n",
            ),
            (
                "utils.ts",
                "export function cn(...a: string[]) { return a.join(' '); }\n",
            ),
        ],
        "cn",
        "utils.ts",
        "button.tsx",
    )
    .await;

    // Definition-before-importer (a_utils.ts < z_button.tsx).
    assert_resolves(
        &[
            (
                "a_utils.ts",
                "export function cx(...a: string[]) { return a.join(' '); }\n",
            ),
            (
                "z_button.tsx",
                "import { cx } from './a_utils';\nexport function B() { return cx('a'); }\n",
            ),
        ],
        "cx",
        "a_utils.ts",
        "z_button.tsx",
    )
    .await;
}

// ── Pinned gap baselines (flip when fixed) ────────────────────────────────

/// BASELINE: namespace-member access does not resolve.
///
/// `import * as ns from './lib'; ns.member()` records only the namespace alias
/// `ns` as an import (no real symbol), and the member access `ns.member` is not
/// tracked — so `member` gets no cross-file edge. Needs module-member
/// resolution (track namespace aliases → resolve `.member` against the imported
/// module's exports). Tracked: f-ts-namespace-dynamic-import-refs.
#[tokio::test]
async fn ts_namespace_member_access_baseline() {
    let proj = IngestedProject::from_files(&[
        ("lib.ts", "export function member() { return 1; }\n"),
        (
            "zapp.ts",
            "import * as ns from './lib';\nexport const x = ns.member();\n",
        ),
    ])
    .await;
    let edges = proj.refs_into("member", None).await;
    let from_app = edges.iter().any(|e| e.from_file.ends_with("zapp.ts"));
    assert!(
        !from_app,
        "namespace-member access now resolves — promote this baseline to a \
         positive `assert_cross_file_ref`."
    );
}

/// BASELINE: dynamic `import()` member access does not resolve.
///
/// `const m = await import('./lib'); m.dyn()` binds the module to a local via
/// an await expression; tracking that binding + resolving `m.dyn` is flow
/// analysis the extractor does not do. Tracked: f-ts-namespace-dynamic-import-refs.
#[tokio::test]
async fn ts_dynamic_import_baseline() {
    let proj = IngestedProject::from_files(&[
        ("lib.ts", "export function dyn() { return 1; }\n"),
        (
            "zapp.ts",
            "export async function go() { const m = await import('./lib'); return m.dyn(); }\n",
        ),
    ])
    .await;
    let edges = proj.refs_into("dyn", None).await;
    let from_app = edges.iter().any(|e| e.from_file.ends_with("zapp.ts"));
    assert!(
        !from_app,
        "dynamic import() member access now resolves — promote this baseline to \
         a positive `assert_cross_file_ref`."
    );
}
