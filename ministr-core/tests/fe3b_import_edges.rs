//! Per-language import edge cases for the cross-file reference graph.
//!
//! Where `fe3_refs.rs` covers the *core* both-orders matrix (a call by
//! the symbol's ORIGINAL name), this file covers the import *shapes* that
//! rename, re-export, or splat names: aliased imports, star/glob imports,
//! re-export hops, Go dot/blank imports, and embedded-struct promoted methods.
//!
//! TypeScript's import-shape matrix (aliased / default / type-only / re-export
//! hop / namespace-member / dynamic import) already lives in `fe_ts_refs.rs`
//! and is NOT duplicated here.
//!
//! Grounded finding (think:116): the resolver is name-based, and every
//! import *statement* emits an `Imports` ref keyed by the symbol's ORIGINAL
//! name. So an aliased / star / re-exported import still produces a resolvable
//! cross-file edge to the definition — the rename only affects the *call*
//! site, not the import edge. These tests assert that resolved edge. The one
//! deliberate non-edge is a Go blank import (`import _ "pkg"`, side-effect
//! only), which `extract_go_import_spec` skips by design — characterized here
//! so the omission is explicit.

mod langtest;

use langtest::IngestedProject;

/// Does a cross-file ref to `target` (defined in `def_suffix`) resolve from
/// `importer_suffix`? Non-panicking, so both positive and characterized-negative
/// cases can assert the real behavior.
async fn cross_file_resolves(
    files: &[(&str, &str)],
    target: &str,
    def_suffix: &str,
    importer_suffix: &str,
) -> bool {
    let proj = IngestedProject::from_files(files).await;
    proj.refs_into(target, None)
        .await
        .iter()
        .any(|e| e.to_file.ends_with(def_suffix) && e.from_file.ends_with(importer_suffix))
}

// ── Python ────────────────────────────────────────────────────────────────

/// `from lib import target as alias` — the import edge to the original `target`
/// resolves cross-file even though the call uses the alias.
#[tokio::test]
async fn python_aliased_import_resolves() {
    assert!(
        cross_file_resolves(
            &[
                ("lib.py", "def target():\n    return 1\n"),
                (
                    "app.py",
                    "from lib import target as alias\n\n\ndef caller():\n    return alias()\n",
                ),
            ],
            "target",
            "lib.py",
            "app.py",
        )
        .await,
        "Python aliased import should resolve the edge to the original `target`",
    );
}

/// `from lib import *` then a bare call — the star import + name-based call
/// resolve cross-file.
#[tokio::test]
async fn python_star_import_resolves() {
    assert!(
        cross_file_resolves(
            &[
                ("lib.py", "def target():\n    return 1\n"),
                (
                    "app.py",
                    "from lib import *\n\n\ndef caller():\n    return target()\n",
                ),
            ],
            "target",
            "lib.py",
            "app.py",
        )
        .await,
        "Python star import + bare call should resolve cross-file",
    );
}

/// `__init__.py` re-export hop: `app` imports from `pkg`, which re-exports from
/// `pkg.core`. The edge to the real definition in `core.py` resolves.
#[tokio::test]
async fn python_init_reexport_resolves() {
    assert!(
        cross_file_resolves(
            &[
                ("pkg/core.py", "def target():\n    return 1\n"),
                ("pkg/__init__.py", "from core import target\n"),
                (
                    "app.py",
                    "from pkg import target\n\n\ndef caller():\n    return target()\n",
                ),
            ],
            "target",
            "pkg/core.py",
            "app.py",
        )
        .await,
        "Python __init__ re-export hop should resolve to the core definition",
    );
}

// ── Go ──────────────────────────────────────────────────────────────────

/// Go dot import (`import . "lib"`) lets `Target()` be called unqualified; the
/// edge to the defining package resolves.
#[tokio::test]
async fn go_dot_import_resolves() {
    assert!(
        cross_file_resolves(
            &[
                ("lib/lib.go", "package lib\n\nfunc Target() int {\n\treturn 1\n}\n"),
                (
                    "main.go",
                    "package main\n\nimport . \"lib\"\n\nfunc caller() int {\n\treturn Target()\n}\n",
                ),
            ],
            "Target",
            "lib/lib.go",
            "main.go",
        )
        .await,
        "Go dot import + unqualified call should resolve cross-file",
    );
}

/// A method promoted through an embedded struct resolves: `User` embeds `Base`,
/// and `u.Describe()` binds to `Base.Describe` in the other file.
#[tokio::test]
async fn go_promoted_method_resolves() {
    assert!(
        cross_file_resolves(
            &[
                (
                    "base.go",
                    "package main\n\ntype Base struct{}\n\nfunc (b Base) Describe() string {\n\treturn \"b\"\n}\n",
                ),
                (
                    "use.go",
                    "package main\n\ntype User struct {\n\tBase\n}\n\nfunc run(u User) string {\n\treturn u.Describe()\n}\n",
                ),
            ],
            "Describe",
            "base.go",
            "use.go",
        )
        .await,
        "Go promoted method (embedded struct) should resolve cross-file",
    );
}

/// CHARACTERIZATION: a Go blank import (`import _ "lib"`) is side-effect-only.
/// `extract_go_import_spec` deliberately skips the `_` (and `.`) alias forms,
/// and the importer names no symbol, so there is NO cross-file edge to the
/// imported package's definitions. This is by design; flipping it would mean
/// blank imports start creating phantom edges.
#[tokio::test]
async fn go_blank_import_creates_no_symbol_edge() {
    assert!(
        !cross_file_resolves(
            &[
                (
                    "lib/lib.go",
                    "package lib\n\nfunc Target() int {\n\treturn 1\n}\n"
                ),
                (
                    "main.go",
                    "package main\n\nimport _ \"lib\"\n\nfunc caller() int {\n\treturn 0\n}\n",
                ),
            ],
            "Target",
            "lib/lib.go",
            "main.go",
        )
        .await,
        "a blank import names no symbol, so it must NOT create a cross-file edge",
    );
}

// ── PHP ────────────────────────────────────────────────────────────────────

/// `use function lib\target as alias` — the import edge to the original
/// `target` resolves even though the call uses the alias.
#[tokio::test]
async fn php_use_alias_resolves() {
    assert!(
        cross_file_resolves(
            &[
                ("lib.php", "<?php\nfunction target() {\n    return 1;\n}\n"),
                (
                    "app.php",
                    "<?php\nuse function lib\\target as alias;\n\nfunction caller() {\n    return alias();\n}\n",
                ),
            ],
            "target",
            "lib.php",
            "app.php",
        )
        .await,
        "PHP `use ... as` aliased import should resolve the edge to the original",
    );
}
