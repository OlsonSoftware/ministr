//! FE6 — the single cross-suite language-coverage GA gate.
//!
//! The FE2–FE5 suites each carry a *local* coverage guard:
//!
//! - `fe2_extraction::every_code_grammar_has_an_extraction_fixture`
//! - `fe3_refs::every_extract_refs_language_has_a_both_orders_fixture`
//! - `fe4`/`bridge_fixtures::every_bridge_kind_has_an_e2e_fixture`
//!
//! This file is the *consolidated* gate that ties them together — semgrep's
//! "GA only when the matrix passes". Its value beyond the per-suite guards is
//! **cross-dimension consistency**: a single canonical [`CODE_LANGUAGES`] list
//! drives BOTH the extraction and the reference dimensions, so adding a
//! language to one suite but forgetting the other is impossible without this
//! gate failing too. All FE2–FE5 suites (and this guard) are ordinary
//! `tests/*.rs` integration tests, so they run under `cargo test` / `just
//! validate` / CI automatically.
//!
//! Adding a language → see CONTRIBUTING.md "Adding a language to the
//! code-intelligence test matrix".

use ministr_core::code::GrammarRegistry;
use ministr_core::code::bridge::BridgeKind;

/// The canonical set of "code" languages the suite covers in depth: each has a
/// symbol-extraction fixture (FE2) AND a cross-file reference fixture (FE3). The
/// extraction and reference matrices MUST cover exactly this set — that shared
/// invariant is what makes the two suites stay in lockstep.
const CODE_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "tsx",
    "go",
    "java",
    "c",
    "cpp",
    "ruby",
    "csharp",
    "swift",
    "kotlin",
    "scala",
    "php",
];

/// Registered grammars deliberately outside the code-intelligence matrix:
/// config/data/markup formats and niche/functional languages without a rich
/// symbol model. Adding a grammar to `GrammarRegistry` forces a choice — move
/// it to `CODE_LANGUAGES` (and write fixtures) or document it here.
const NON_CODE_DEFERRED: &[&str] = &[
    "bash",
    "lua",
    "elixir",
    "haskell",
    "ocaml",
    "ocaml_interface",
    "dart",
    "r",
    "hcl",
    "json",
    "yaml",
    "toml",
    "sql",
    "zig",
    "proto",
    "svelte",
    "css",
    "graphql",
    "groovy",
    "nix",
    "erlang",
    "powershell",
    "solidity",
    "objc",
    "julia",
    "cmake",
    "make",
];

/// Cross-suite gate, code-language dimension: every registered grammar is
/// either a covered code language (extraction + reference fixtures) or an
/// explicitly-deferred non-code format. A newly-registered grammar that is in
/// neither fails here — the GA gate.
#[test]
fn every_registered_grammar_is_categorized() {
    let registry = GrammarRegistry::global();

    // Every code language must be registered (catches a typo / removed grammar).
    for lang in CODE_LANGUAGES {
        assert!(
            registry.language_by_name(lang).is_some(),
            "CODE_LANGUAGES lists `{lang}`, but it is not a registered grammar",
        );
    }

    // No language may be in both partitions.
    let deferred: std::collections::HashSet<&str> = NON_CODE_DEFERRED.iter().copied().collect();
    for lang in CODE_LANGUAGES {
        assert!(
            !deferred.contains(lang),
            "`{lang}` is in both CODE_LANGUAGES and NON_CODE_DEFERRED — pick one",
        );
    }

    // Every registered grammar is categorized.
    let covered: std::collections::HashSet<&str> = CODE_LANGUAGES.iter().copied().collect();
    let mut uncategorized: Vec<&str> = registry
        .language_names()
        .filter(|l| !covered.contains(l) && !deferred.contains(l))
        .collect();
    uncategorized.sort_unstable();

    assert!(
        uncategorized.is_empty(),
        "these registered grammars are in neither CODE_LANGUAGES nor \
         NON_CODE_DEFERRED: {uncategorized:?}\n\
         → add extraction (fe2_extraction.rs) + reference (fe3_refs.rs) fixtures \
         and list it in CODE_LANGUAGES, or add it to NON_CODE_DEFERRED with a \
         reason. See CONTRIBUTING.md.",
    );
}

/// Bridge kinds with an e2e link fixture in `bridge_fixtures.rs` (FE4).
const BRIDGE_COVERED: &[&str] = &[
    "tauri_command",
    "tauri_event",
    "napi",
    "wasm_bindgen",
    "pyo3",
    "http_route",
    "ffi",
    "cgo",
    "jni",
    "uniffi",
    "grpc",
    "flutter_channel",
    "electron_ipc",
];

/// Cross-suite gate, bridge dimension. The `match` is exhaustive: adding a
/// `BridgeKind` variant fails to compile HERE until it is handled (and listed
/// in `BRIDGE_COVERED` + given a fixture in `bridge_fixtures.rs`).
#[test]
fn every_bridge_kind_is_categorized() {
    let all = [
        BridgeKind::TauriCommand,
        BridgeKind::TauriEvent,
        BridgeKind::Napi,
        BridgeKind::WasmBindgen,
        BridgeKind::PyO3,
        BridgeKind::HttpRoute,
        BridgeKind::Ffi,
        BridgeKind::Cgo,
        BridgeKind::Jni,
        BridgeKind::UniFfi,
        BridgeKind::Grpc,
        BridgeKind::FlutterChannel,
        BridgeKind::ElectronIpc,
    ];
    for k in all {
        // Exhaustiveness tripwire — a new variant breaks this match.
        match k {
            BridgeKind::TauriCommand
            | BridgeKind::TauriEvent
            | BridgeKind::Napi
            | BridgeKind::WasmBindgen
            | BridgeKind::PyO3
            | BridgeKind::HttpRoute
            | BridgeKind::Ffi
            | BridgeKind::Cgo
            | BridgeKind::Jni
            | BridgeKind::UniFfi
            | BridgeKind::Grpc
            | BridgeKind::FlutterChannel
            | BridgeKind::ElectronIpc => {}
        }
        assert!(
            BRIDGE_COVERED.contains(&k.as_str()),
            "BridgeKind `{}` has no e2e fixture — add one to bridge_fixtures.rs \
             and list it in BRIDGE_COVERED",
            k.as_str(),
        );
    }
}
