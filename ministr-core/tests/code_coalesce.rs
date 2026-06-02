//! Regression test for rq3b: tiny adjacent CODE symbols coalesce.
//!
//! `coalesce_small_sections` is applied in the shared `store_enriched_document`
//! ingestion path (with `min_section_tokens = 50` by default) and recurses into
//! a section's children — so the per-symbol sections that `CodeParser` emits as
//! depth-2 children of the file-overview root are coalesced too. A file of many
//! tiny one-line symbols (getters, trivial fns) therefore does NOT emit a swarm
//! of sub-budget sections: adjacent tiny symbols merge, while a substantial
//! symbol stays its own embedded unit. This locks that behaviour so a future
//! change to the parser or the ingestion path can't silently regress it.

use ministr_core::ingestion::coalesce_small_sections;
use ministr_core::parser::{CodeParser, DocumentParser};
use std::path::Path;

const SAMPLE: &str = r"/// Sum every value in the slice and return the running total. This function is
/// deliberately documented and multi-statement so it stays above the
/// small-section coalescing threshold and remains its own embedded unit.
pub fn compute_running_total(values: &[u32]) -> u64 {
    let mut total: u64 = 0;
    for value in values {
        total += u64::from(*value);
    }
    total
}

pub fn one() -> u8 { 1 }

pub fn two() -> u8 { 2 }
";

#[test]
fn tiny_adjacent_code_symbols_coalesce_but_a_big_symbol_stays_separate() {
    let tree = CodeParser::new()
        .parse(Path::new("src/widgets.rs"), SAMPLE)
        .unwrap();

    // Sanity: the parser emits one section per symbol as children of the
    // file-overview root — compute_running_total, one, two.
    let before = tree.sections[0].children.len();
    assert_eq!(
        before, 3,
        "expected 3 per-symbol sections before coalescing, got {before}"
    );

    // The default ingestion threshold (IngestionPipeline::new -> 50).
    let coalesced = coalesce_small_sections(tree.sections.clone(), 50);
    let root = &coalesced[0];

    // The two tiny adjacent fns merge into one section; the big one stays.
    assert_eq!(
        root.children.len(),
        2,
        "tiny adjacent symbols should coalesce (3 -> 2), got {}: {:?}",
        root.children.len(),
        root.children
            .iter()
            .map(|c| &c.heading_path)
            .collect::<Vec<_>>()
    );

    // The big, well-documented symbol is preserved as its own unit.
    assert!(
        root.children
            .iter()
            .any(|c| c.text.contains("compute_running_total") && c.text.contains("for value")),
        "the substantial symbol must remain its own section"
    );

    // Exactly one section now carries BOTH tiny symbols (they merged together).
    let merged = root
        .children
        .iter()
        .filter(|c| c.text.contains("fn one") && c.text.contains("fn two"))
        .count();
    assert_eq!(
        merged, 1,
        "the two tiny fns should share one merged section"
    );
}
