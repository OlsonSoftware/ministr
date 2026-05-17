//! Type conversions from ministr-core domain types to ministr-api wire types.
//!
//! Free functions instead of From impls to avoid orphan rule violations
//! (both types are from external crates relative to ministr-daemon).

use ministr_api::query;

#[must_use]
pub fn survey_result(r: ministr_core::service::SurveyResult) -> query::SurveyResult {
    query::SurveyResult {
        content_id: r.content_id,
        resolution: r.resolution,
        score: r.score,
        text: r.text,
        heading_path: r.heading_path,
    }
}

#[must_use]
pub fn section_detail(d: ministr_core::service::SectionDetail) -> query::SectionDetail {
    query::SectionDetail {
        section_id: d.section_id,
        heading_path: d.heading_path,
        text: d.text,
        summary: d.summary,
        claims_available: d.claims_available,
        status: None,
        usage_status: None,
    }
}

#[must_use]
pub fn claim_result(c: ministr_core::service::ClaimResult) -> query::ClaimResult {
    query::ClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relevance: c.relevance,
    }
}

#[must_use]
pub fn symbol_definition(s: ministr_core::service::SymbolDefinition) -> query::SymbolDefinition {
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

#[must_use]
pub fn symbol_from_record(s: ministr_core::storage::SymbolRecord) -> query::SymbolDefinition {
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

#[must_use]
pub fn symbol_reference(r: ministr_core::service::SymbolRefResult) -> query::SymbolReference {
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

#[must_use]
pub fn related_claim(c: ministr_core::service::RelatedClaimResult) -> query::RelatedClaimResult {
    query::RelatedClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relation_type: c.relation_type,
        source_section: c.source_section,
        confidence: c.confidence,
    }
}

#[must_use]
pub fn toc_entry(e: ministr_core::types::TocEntry) -> query::TocEntry {
    query::TocEntry {
        id: e.section_id.0,
        title: e.heading_path.last().cloned().unwrap_or_default(),
        kind: "section".to_string(),
        depth: e.depth as usize,
        children: 0,
        source_path: Some(e.document_id.0),
    }
}

#[must_use]
pub fn bridge_link(l: ministr_core::storage::BridgeLinkDetail) -> query::BridgeLink {
    query::BridgeLink {
        kind: l.kind,
        source: l.export_binding_key,
        source_language: l.export_language,
        target: l.import_binding_key,
        target_language: l.import_language,
        confidence: l.confidence,
    }
}

#[must_use]
pub fn compressed_item(
    c: ministr_core::service::CompressedItem,
) -> ministr_api::session::CompressedItemApi {
    ministr_api::session::CompressedItemApi {
        original_id: c.original_id,
        summary: c.summary,
        original_tokens: c.original_tokens,
        compressed_tokens: c.compressed_tokens,
        method: c.method,
    }
}

#[must_use]
pub fn bundle_manifest(
    m: &ministr_core::bundle::BundleManifest,
) -> ministr_api::corpus::BundleManifestApi {
    ministr_api::corpus::BundleManifestApi {
        format_version: m.format_version,
        model_name: m.model_name.clone(),
        dimension: m.dimension,
        vector_count: m.vector_count,
        document_count: m.document_count,
        symbol_count: m.symbol_count,
        bundle_version: m.bundle_version.clone(),
    }
}

#[must_use]
pub fn usage_status(
    b: &ministr_core::session::UsageStatus,
) -> ministr_api::session::SessionUsageResponse {
    ministr_api::session::SessionUsageResponse {
        level: format!("{:?}", b.level).to_lowercase(),
        tokens_used: b.tokens_used,
        tokens_remaining: b.tokens_remaining,
        utilization: b.utilization,
    }
}

#[must_use]
pub fn prefetch_metrics(
    m: &ministr_core::session::prefetch::PrefetchMetrics,
    cache_size: usize,
    cache_capacity: usize,
) -> ministr_api::session::PrefetchMetricsResponse {
    ministr_api::session::PrefetchMetricsResponse {
        hits: m.hits,
        misses: m.misses,
        sequential_hits: m.sequential_hits,
        topical_hits: m.topical_hits,
        structural_hits: m.structural_hits,
        cross_session_hits: m.cross_session_hits,
        survey_expand_hits: m.survey_expand_hits,
        agent_plan_hits: m.agent_plan_hits,
        cache_size,
        cache_capacity,
    }
}
