//! FE3a — Core cross-file reference-graph matrix (every `extract_refs` language).
//!
//! For each supported language, a 2-file fixture where file B references a
//! symbol `target` defined in file A, asserting the cross-file edge resolves in
//! **both ingest orders**:
//!
//! - **definition-before-importer** — the easy case, and
//! - **importer-before-definition** — the deferred second-pass path that was
//!   the real "no related files" bug (an importer ingested before the file it
//!   references must still get its edge filled in on the second pass).
//!
//! The harness ingests a directory in sorted path order, so order is controlled
//! purely by filename: the definition always lives in `lib.<ext>` (keeping the
//! importer's import path valid), and only the importer's filename changes —
//! `zuse.<ext>` sorts *after* `lib` (def first), `aimp.<ext>` sorts *before*
//! (importer first).
//!
//! Resolution of `Calls`/`Uses` edges is by symbol NAME (per the edge-graph
//! rollout), so the assertions accept any ref kind (`None`). Per-language import
//! *edge cases* (aliased/re-export/star/namespace/dynamic) live in FE3b and the
//! existing `fe_ts_refs.rs`; this file is the systematic both-orders core.
//!
//! A coverage guard ([`every_extract_refs_language_has_a_both_orders_fixture`])
//! fails when a language the resolver supports lacks a fixture here.

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};

/// Assert `target` — defined in `lib.<ext>` — is referenced cross-file from the
/// importer file, in BOTH ingest orders.
async fn resolves_both_orders(ext: &str, def: &str, importer: &str, target: &str) {
    let lib = format!("lib.{ext}");
    let zuse = format!("zuse.{ext}");
    let aimp = format!("aimp.{ext}");

    // Definition-before-importer (lib.<ext> < zuse.<ext>).
    let def_first =
        IngestedProject::from_files(&[(lib.as_str(), def), (zuse.as_str(), importer)]).await;
    assert_cross_file_ref(&def_first, target, &lib, &zuse, None).await;

    // Importer-before-definition (aimp.<ext> < lib.<ext>) — deferred second pass.
    let importer_first =
        IngestedProject::from_files(&[(aimp.as_str(), importer), (lib.as_str(), def)]).await;
    assert_cross_file_ref(&importer_first, target, &lib, &aimp, None).await;
}

// ── Rust ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn rust_cross_file_ref_both_orders() {
    resolves_both_orders(
        "rs",
        "pub fn target() -> i32 {\n    1\n}\n",
        "pub fn caller() -> i32 {\n    target()\n}\n",
        "target",
    )
    .await;
}

// ── Python ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn python_cross_file_ref_both_orders() {
    resolves_both_orders(
        "py",
        "def target():\n    return 1\n",
        "from lib import target\n\n\ndef caller():\n    return target()\n",
        "target",
    )
    .await;
}

// ── JavaScript ────────────────────────────────────────────────────────────

#[tokio::test]
async fn javascript_cross_file_ref_both_orders() {
    resolves_both_orders(
        "js",
        "export function target() {\n  return 1;\n}\n",
        "import { target } from './lib';\nexport function caller() {\n  return target();\n}\n",
        "target",
    )
    .await;
}

// ── TypeScript ──────────────────────────────────────────────────────────

#[tokio::test]
async fn typescript_cross_file_ref_both_orders() {
    resolves_both_orders(
        "ts",
        "export function target(): number {\n  return 1;\n}\n",
        "import { target } from './lib';\nexport function caller(): number {\n  return target();\n}\n",
        "target",
    )
    .await;
}

// ── TSX ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tsx_cross_file_ref_both_orders() {
    resolves_both_orders(
        "tsx",
        "export function target(): number {\n  return 1;\n}\n",
        "import { target } from './lib';\nexport function caller(): number {\n  return target();\n}\n",
        "target",
    )
    .await;
}

// ── Go ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn go_cross_file_ref_both_orders() {
    resolves_both_orders(
        "go",
        "package main\n\nfunc target() int {\n\treturn 1\n}\n",
        "package main\n\nfunc caller() int {\n\treturn target()\n}\n",
        "target",
    )
    .await;
}

// ── Java ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn java_cross_file_ref_both_orders() {
    resolves_both_orders(
        "java",
        "public class Lib {\n    public static int target() {\n        return 1;\n    }\n}\n",
        "public class Use {\n    int caller() {\n        return Lib.target();\n    }\n}\n",
        "target",
    )
    .await;
}

// ── C ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c_cross_file_ref_both_orders() {
    resolves_both_orders(
        "c",
        "int target(void) {\n    return 1;\n}\n",
        "int caller(void) {\n    return target();\n}\n",
        "target",
    )
    .await;
}

// ── C++ ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cpp_cross_file_ref_both_orders() {
    resolves_both_orders(
        "cpp",
        "int target() {\n    return 1;\n}\n",
        "int caller() {\n    return target();\n}\n",
        "target",
    )
    .await;
}

// ── PHP ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn php_cross_file_ref_both_orders() {
    resolves_both_orders(
        "php",
        "<?php\nfunction target() {\n    return 1;\n}\n",
        "<?php\nfunction caller() {\n    return target();\n}\n",
        "target",
    )
    .await;
}

// ── Kotlin ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn kotlin_cross_file_ref_both_orders() {
    resolves_both_orders(
        "kt",
        "fun target(): Int {\n    return 1\n}\n",
        "fun caller(): Int {\n    return target()\n}\n",
        "target",
    )
    .await;
}

// ── Scala ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn scala_cross_file_ref_both_orders() {
    resolves_both_orders(
        "scala",
        "object Lib {\n  def target(): Int = 1\n}\n",
        "object Use {\n  def caller(): Int = Lib.target()\n}\n",
        "target",
    )
    .await;
}

// ── C# ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn csharp_cross_file_ref_both_orders() {
    resolves_both_orders(
        "cs",
        "public class Lib {\n    public static int Target() {\n        return 1;\n    }\n}\n",
        "public class Use {\n    int Caller() {\n        return Lib.Target();\n    }\n}\n",
        "Target",
    )
    .await;
}

// ── Swift ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn swift_cross_file_ref_both_orders() {
    resolves_both_orders(
        "swift",
        "func target() -> Int {\n    return 1\n}\n",
        "func caller() -> Int {\n    return target()\n}\n",
        "target",
    )
    .await;
}

// ── Ruby ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ruby_cross_file_ref_both_orders() {
    // Use explicit call parens: a paren-less `target` parses as an identifier
    // (could be a local var), so the Ruby ref extractor only emits a `Calls`
    // edge for the unambiguous `target()` form.
    resolves_both_orders(
        "rb",
        "def target\n  1\nend\n",
        "def caller\n  target()\nend\n",
        "target",
    )
    .await;
}
