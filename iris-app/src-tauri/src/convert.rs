//! Type conversions from iris-core domain types to iris-api wire types.
//!
//! Free functions instead of From impls to avoid orphan rule violations
//! (both types are from external crates relative to iris-app).

use iris_api::query;

pub fn survey_result(r: iris_core::service::SurveyResult) -> query::SurveyResult {
    query::SurveyResult {
        content_id: r.content_id,
        resolution: r.resolution,
        score: r.score,
        text: r.text,
        heading_path: r.heading_path,
    }
}

pub fn section_detail(d: iris_core::service::SectionDetail) -> query::SectionDetail {
    query::SectionDetail {
        section_id: d.section_id,
        heading_path: d.heading_path,
        text: d.text,
        summary: d.summary,
        claims_available: d.claims_available,
    }
}

pub fn claim_result(c: iris_core::service::ClaimResult) -> query::ClaimResult {
    query::ClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relevance: c.relevance,
    }
}

pub fn symbol_definition(s: iris_core::service::SymbolDefinition) -> query::SymbolDefinition {
    query::SymbolDefinition {
        id: s.id,
        name: s.name,
        kind: s.kind,
        visibility: s.visibility,
        signature: s.signature,
        doc_comment: s.doc_comment,
        file_path: s.file_path,
        line_start: s.line_start,
        line_end: s.line_end,
        heading_path: s.heading_path,
        source_context: s.source_context,
    }
}

pub fn symbol_from_record(s: iris_core::storage::SymbolRecord) -> query::SymbolDefinition {
    query::SymbolDefinition {
        id: s.id.0,
        name: s.name,
        kind: s.kind,
        visibility: s.visibility,
        signature: s.signature,
        doc_comment: s.doc_comment,
        file_path: s.file_path,
        line_start: s.line_start,
        line_end: s.line_end,
        heading_path: s
            .module_path
            .split("::")
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect(),
        source_context: String::new(),
    }
}

pub fn symbol_reference(r: iris_core::service::SymbolRefResult) -> query::SymbolReference {
    query::SymbolReference {
        from_symbol_id: r.from_symbol_id,
        from_name: r.from_name,
        from_file: r.from_file,
        from_line: r.from_line,
        to_symbol_id: r.to_symbol_id,
        to_name: r.to_name,
        to_file: r.to_file,
        to_line: r.to_line,
        ref_kind: r.ref_kind,
    }
}

pub fn related_claim(c: iris_core::service::RelatedClaimResult) -> query::RelatedClaimResult {
    query::RelatedClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relation_type: c.relation_type,
        source_section: c.source_section,
        confidence: c.confidence,
    }
}

pub fn toc_entry(e: iris_core::types::TocEntry) -> query::TocEntry {
    query::TocEntry {
        id: e.section_id.0,
        title: e.heading_path.last().cloned().unwrap_or_default(),
        kind: "section".to_string(),
        depth: e.depth as usize,
        children: 0,
        source_path: Some(e.document_id.0),
    }
}

pub fn bridge_link(l: iris_core::storage::BridgeLinkDetail) -> query::BridgeLink {
    query::BridgeLink {
        kind: l.kind,
        source: l.export_binding_key,
        source_language: l.export_language,
        target: l.import_binding_key,
        target_language: l.import_language,
        confidence: l.confidence,
    }
}
