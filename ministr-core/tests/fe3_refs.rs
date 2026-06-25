//! Core cross-file reference-graph matrix (every `extract_refs` language).
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
//! *edge cases* (aliased/re-export/star/namespace/dynamic) live in `fe3b_import_edges.rs` and the
//! existing `fe_ts_refs.rs`; this file is the systematic both-orders core.
//!
//! A coverage guard ([`every_extract_refs_language_has_a_both_orders_fixture`])
//! fails when a language the resolver supports lacks a fixture here.

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};
use ministr_core::code::GrammarRegistry;

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

// ── Coverage guard ──────────────────────────────────────────────────────
//
// Every language whose `code::refs::extract_refs` emits a cross-reference graph
// has a both-orders fixture above; they are listed in `FE3_COVERED`. The
// remaining registered grammars have no ref-extraction dispatch (config/data/
// markup or deferred) and are parked in `REF_DEFERRED` with a reason. The guard
// fails when a registered grammar is in neither set — so adding a grammar (and
// wiring it into `extract_refs`) forces a decision: write a both-orders fixture
// (move it to `FE3_COVERED`) or document why it has no ref graph.

/// Languages with a `<lang>_cross_file_ref_both_orders` test in this file.
const FE3_COVERED: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "tsx",
    "go",
    "java",
    "c",
    "cpp",
    "php",
    "kotlin",
    "scala",
    "csharp",
    "swift",
    "ruby",
];

/// Registered grammars with no `extract_refs` dispatch (no cross-reference
/// graph), hence no both-orders fixture, with the reason.
const REF_DEFERRED: &[(&str, &str)] = &[
    ("bash", "shell — no symbol-ref graph"),
    ("lua", "scripting — no ref dispatch"),
    ("elixir", "functional — no ref dispatch"),
    ("haskell", "functional — no ref dispatch"),
    ("ocaml", "functional — no ref dispatch"),
    ("ocaml_interface", "OCaml .mli — no ref dispatch"),
    ("dart", "no ref dispatch yet"),
    ("r", "statistical scripting — no ref dispatch"),
    ("hcl", "config (Terraform) — no code refs"),
    ("json", "data format — no code refs"),
    ("yaml", "data format — no code refs"),
    ("toml", "data format — no code refs"),
    ("sql", "query language — no ref dispatch"),
    ("zig", "no ref dispatch yet"),
    ("proto", "IDL — no ref dispatch"),
    ("svelte", "SFC markup host — no ref dispatch"),
    ("css", "stylesheet — no code refs"),
    ("graphql", "schema IDL — no ref dispatch"),
    ("groovy", "JVM scripting — no ref dispatch"),
    ("nix", "config/expr — no ref dispatch"),
    ("erlang", "functional — no ref dispatch"),
    ("powershell", "shell scripting — no ref dispatch"),
    ("solidity", "no ref dispatch yet"),
    ("objc", "no ref dispatch yet"),
    ("julia", "no ref dispatch yet"),
    ("cmake", "build config — no code refs"),
    ("make", "build config — no code refs"),
];

#[test]
fn every_extract_refs_language_has_a_both_orders_fixture() {
    let registry = GrammarRegistry::global();

    // 1. Every covered language must be a registered grammar.
    for lang in FE3_COVERED {
        assert!(
            registry.language_by_name(lang).is_some(),
            "FE3_COVERED lists `{lang}`, but it is not a registered grammar",
        );
    }

    // 2. Every registered grammar is categorized: covered (has a both-orders
    //    fixture) or explicitly ref-deferred. A new grammar in neither fails.
    let deferred: std::collections::HashSet<&str> = REF_DEFERRED.iter().map(|(l, _)| *l).collect();
    let covered: std::collections::HashSet<&str> = FE3_COVERED.iter().copied().collect();

    let mut uncategorized: Vec<&str> = registry
        .language_names()
        .filter(|l| !covered.contains(l) && !deferred.contains(l))
        .collect();
    uncategorized.sort_unstable();

    assert!(
        uncategorized.is_empty(),
        "these registered grammars have no both-orders ref fixture and are not in \
         the deferral allowlist: {uncategorized:?}\n\
         → add a `<lang>_cross_file_ref_both_orders` test (and list it in \
         FE3_COVERED), or add the language to REF_DEFERRED with a reason.",
    );

    // 3. No language in both lists.
    for lang in FE3_COVERED {
        assert!(
            !deferred.contains(lang),
            "`{lang}` is in both FE3_COVERED and REF_DEFERRED — pick one",
        );
    }
}
