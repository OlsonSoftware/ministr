//! Section-level processing: enrichment, coalescing, splitting, and text collection.

use crate::extraction::claims::ClaimExtractor;
use crate::extraction::summary::SummaryGenerator;
use crate::token::count_tokens;
use crate::types::{Claim, Section, SectionId};

pub(super) const SUMMARY_MAX_SENTENCES: usize = 3;
const PARAGRAPH_SPLIT_THRESHOLD: usize = 500;

/// Recursively enrich sections with claims and summaries.
///
/// Returns `(total_sections, total_claims)` counts.
pub(super) fn enrich_sections(
    sections: &mut [Section],
    claim_extractor: &dyn ClaimExtractor,
    summary_generator: &dyn SummaryGenerator,
) -> (usize, usize) {
    let mut section_count = 0;
    let mut claim_count = 0;

    for section in sections.iter_mut() {
        section_count += 1;

        if !section.text.trim().is_empty() {
            let claims = claim_extractor.extract(&section.text, &section.id);
            claim_count += claims.len();
            section.claims = claims;

            let summary = summary_generator.summarize(&section.text, SUMMARY_MAX_SENTENCES);
            if !summary.is_empty() {
                section.summary = Some(summary);
            }
        }

        let (child_sections, child_claims) =
            enrich_sections(&mut section.children, claim_extractor, summary_generator);
        section_count += child_sections;
        claim_count += child_claims;
    }

    (section_count, claim_count)
}

pub(super) fn collect_all_text(sections: &[Section]) -> String {
    let mut parts = Vec::new();
    collect_text_recursive(sections, &mut parts);
    parts.join(" ")
}

fn collect_text_recursive(sections: &[Section], parts: &mut Vec<String>) {
    for section in sections {
        if !section.text.trim().is_empty() {
            parts.push(section.text.clone());
        }
        collect_text_recursive(&section.children, parts);
    }
}

pub(super) fn collect_all_claims(sections: &[Section]) -> Vec<Claim> {
    let mut claims = Vec::new();
    for section in sections {
        claims.extend(section.claims.iter().cloned());
        claims.extend(collect_all_claims(&section.children));
    }
    claims
}

pub(super) fn split_large_headingless_section(section: Section, source_path: &str) -> Vec<Section> {
    if section.depth != 0 || section.text.split_whitespace().count() <= PARAGRAPH_SPLIT_THRESHOLD {
        return vec![section];
    }

    let paragraphs: Vec<&str> = section
        .text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    if paragraphs.len() <= 1 {
        return vec![section];
    }

    paragraphs
        .into_iter()
        .enumerate()
        .map(|(i, para)| {
            let id_str = format!("{source_path}#paragraph-{i}");
            Section {
                id: SectionId(id_str),
                heading_path: Vec::new(),
                depth: 0,
                text: para.to_string(),
                structural_nodes: Vec::new(),
                children: Vec::new(),
                claims: Vec::new(),
                summary: None,
            }
        })
        .collect()
}

/// Coalesce adjacent sibling sections below a minimum token threshold.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::coalesce_small_sections;
/// use iris_core::types::{Section, SectionId};
///
/// let sections = vec![
///     Section {
///         id: SectionId("s1".into()),
///         heading_path: vec!["Small A".into()],
///         depth: 2,
///         text: "Tiny.".into(),
///         structural_nodes: vec![],
///         children: vec![],
///         claims: vec![],
///         summary: None,
///     },
///     Section {
///         id: SectionId("s2".into()),
///         heading_path: vec!["Small B".into()],
///         depth: 2,
///         text: "Also tiny.".into(),
///         structural_nodes: vec![],
///         children: vec![],
///         claims: vec![],
///         summary: None,
///     },
/// ];
///
/// let merged = coalesce_small_sections(sections, 50);
/// assert_eq!(merged.len(), 1);
/// assert!(merged[0].text.contains("Small B"));
/// ```
#[must_use]
pub fn coalesce_small_sections(sections: Vec<Section>, min_tokens: usize) -> Vec<Section> {
    if min_tokens == 0 {
        return sections;
    }

    let mut result: Vec<Section> = Vec::new();

    for section in sections {
        let token_count = count_tokens(&section.text);

        if token_count >= min_tokens {
            let mut section = section;
            section.children =
                coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
            result.push(section);
        } else if let Some(prev) = result.last_mut() {
            if prev.depth == section.depth && count_tokens(&prev.text) < min_tokens {
                merge_into(prev, section);
            } else {
                let mut section = section;
                section.children =
                    coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
                result.push(section);
            }
        } else {
            let mut section = section;
            section.children =
                coalesce_small_sections(std::mem::take(&mut section.children), min_tokens);
            result.push(section);
        }
    }

    result
}

fn merge_into(target: &mut Section, source: Section) {
    use std::fmt::Write;

    let heading = source.heading_path.last().cloned().unwrap_or_default();

    if heading.is_empty() {
        target.text.push_str("\n\n");
    } else {
        let _ = write!(target.text, "\n\n### {heading}\n\n");
    }
    target.text.push_str(&source.text);

    target.structural_nodes.extend(source.structural_nodes);
    target.children.extend(source.children);
}
