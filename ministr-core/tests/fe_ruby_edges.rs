//! FE — Ruby edge-graph (Calls / Implements / Uses) cross-file tests.
//!
//! Before this slice the Ruby extractor was import-only, so `ministr_references`
//! / `ministr_solid` starved on Ruby codebases. Ruby is dynamic with no type
//! annotations, so the heritage signals are the class superclass (`class C <
//! Base`) and module mixins (`include Mixin` — Ruby's de-facto interface).
//! These tests assert SPECIFIC `RefKind::Implements` and `RefKind::Calls`
//! cross-file edges in BOTH file ingest orders, plus a `RefKind::Uses` edge
//! for a `Constant.new` construction.

#![cfg(feature = "lang-ruby")]

mod langtest;

use langtest::{IngestedProject, assert_cross_file_ref};
use ministr_core::code::refs::extract_refs;
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
async fn ruby_superclass_both_orders() {
    // Definition-before-subclass (a_base.rb < b_economy.rb).
    assert_edge(
        &[
            ("a_base.rb", "class BaseService\n  def ping; end\nend\n"),
            (
                "b_economy.rb",
                "require 'a_base'\nclass EconomyService < BaseService\n  def run; end\nend\n",
            ),
        ],
        "BaseService",
        "a_base.rb",
        "b_economy.rb",
        RefKind::Implements,
    )
    .await;

    // Subclass-before-definition (a_economy.rb < b_base.rb).
    assert_edge(
        &[
            (
                "a_economy.rb",
                "require 'b_base'\nclass OtherService < OtherBase\n  def go; end\nend\n",
            ),
            ("b_base.rb", "class OtherBase\n  def go; end\nend\n"),
        ],
        "OtherBase",
        "b_base.rb",
        "a_economy.rb",
        RefKind::Implements,
    )
    .await;
}

/// `include` / `prepend` emit `RefKind::Implements` edges onto the mixed-in
/// modules (Ruby's interface mechanism). Asserted directly on the extractor
/// output: cross-file *resolution* of a mixin requires the target module to be
/// an in-corpus symbol, and module-kind symbols aren't yet resolved as
/// Implements targets (tracked: f-ruby-module-implements-resolution) — but the
/// extractor correctly produces the edge.
#[test]
fn ruby_include_prepend_emit_implements() {
    let src = b"class EconomyService\n  include IService\n  prepend IOther\nend\n";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(&src[..], None).unwrap();
    let refs = extract_refs(&tree, &src[..], "ruby");

    let implements: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == RefKind::Implements)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        implements.contains(&"IService"),
        "expected Implements(IService) from `include`; got {implements:?}",
    );
    assert!(
        implements.contains(&"IOther"),
        "expected Implements(IOther) from `prepend`; got {implements:?}",
    );
}

#[tokio::test]
async fn ruby_calls_cross_file_both_orders() {
    // Definition-before-caller (a_lib.rb < b_caller.rb).
    assert_edge(
        &[
            ("a_lib.rb", "def helper\n  1\nend\n"),
            ("b_caller.rb", "require 'a_lib'\ndef run\n  helper()\nend\n"),
        ],
        "helper",
        "a_lib.rb",
        "b_caller.rb",
        RefKind::Calls,
    )
    .await;

    // Caller-before-definition (a_caller.rb < b_lib.rb).
    assert_edge(
        &[
            (
                "a_caller.rb",
                "require 'b_lib'\ndef run\n  compute()\nend\n",
            ),
            ("b_lib.rb", "def compute\n  2\nend\n"),
        ],
        "compute",
        "b_lib.rb",
        "a_caller.rb",
        RefKind::Calls,
    )
    .await;
}

#[tokio::test]
async fn ruby_new_emits_uses_cross_file() {
    // `Widget.new` → a Uses edge onto the constructed class.
    assert_edge(
        &[
            ("widget.rb", "class Widget\n  def draw; end\nend\n"),
            (
                "factory.rb",
                "require 'widget'\nclass Factory\n  def make\n    Widget.new\n  end\nend\n",
            ),
        ],
        "Widget",
        "widget.rb",
        "factory.rb",
        RefKind::Uses,
    )
    .await;
}
