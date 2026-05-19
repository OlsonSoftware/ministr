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
pub fn impact_risk(r: ministr_core::service::ImpactRisk) -> query::ImpactRisk {
    match r {
        ministr_core::service::ImpactRisk::Low => query::ImpactRisk::Low,
        ministr_core::service::ImpactRisk::Medium => query::ImpactRisk::Medium,
        ministr_core::service::ImpactRisk::High => query::ImpactRisk::High,
    }
}

#[must_use]
pub fn impact_caller(c: ministr_core::service::ImpactCaller) -> query::ImpactCaller {
    query::ImpactCaller {
        symbol_id: c.symbol_id,
        name: c.name,
        kind: c.kind,
        file: c.file,
        line: c.line,
        depth: c.depth,
    }
}

#[must_use]
pub fn impact_response(r: ministr_core::service::ImpactResult) -> query::ImpactResponse {
    query::ImpactResponse {
        target_symbol_id: r.target_symbol_id,
        depth: r.depth,
        symbols: r.symbols,
        files: r.files,
        tests: r.tests,
        risk: impact_risk(r.risk),
        callers: r.callers.into_iter().map(impact_caller).collect(),
    }
}

#[must_use]
pub fn dead_symbol(s: ministr_core::service::DeadSymbol) -> query::DeadSymbol {
    query::DeadSymbol {
        symbol_id: s.symbol_id,
        name: s.name,
        kind: s.kind,
        visibility: s.visibility,
        file: s.file,
        line: s.line,
        lines: s.lines,
    }
}

#[must_use]
pub fn solid_principle(p: ministr_core::service::SolidPrinciple) -> query::SolidPrinciple {
    match p {
        ministr_core::service::SolidPrinciple::DryOcp => query::SolidPrinciple::DryOcp,
        ministr_core::service::SolidPrinciple::Srp => query::SolidPrinciple::Srp,
        ministr_core::service::SolidPrinciple::Isp => query::SolidPrinciple::Isp,
        ministr_core::service::SolidPrinciple::Dip => query::SolidPrinciple::Dip,
        ministr_core::service::SolidPrinciple::ShotgunSurgery => {
            query::SolidPrinciple::ShotgunSurgery
        }
        ministr_core::service::SolidPrinciple::CyclicDependency => {
            query::SolidPrinciple::CyclicDependency
        }
    }
}

#[must_use]
pub fn api_solid_principle(p: query::SolidPrinciple) -> ministr_core::service::SolidPrinciple {
    match p {
        query::SolidPrinciple::DryOcp => ministr_core::service::SolidPrinciple::DryOcp,
        query::SolidPrinciple::Srp => ministr_core::service::SolidPrinciple::Srp,
        query::SolidPrinciple::Isp => ministr_core::service::SolidPrinciple::Isp,
        query::SolidPrinciple::Dip => ministr_core::service::SolidPrinciple::Dip,
        query::SolidPrinciple::ShotgunSurgery => {
            ministr_core::service::SolidPrinciple::ShotgunSurgery
        }
        query::SolidPrinciple::CyclicDependency => {
            ministr_core::service::SolidPrinciple::CyclicDependency
        }
    }
}

#[must_use]
pub fn solid_symbol_ref(s: ministr_core::service::SolidSymbolRef) -> query::SolidSymbolRef {
    query::SolidSymbolRef {
        symbol_id: s.symbol_id,
        name: s.name,
        kind: s.kind,
        file: s.file,
        line: s.line,
    }
}

#[must_use]
pub fn solid_component(c: ministr_core::service::SolidComponent) -> query::SolidComponent {
    query::SolidComponent {
        size: c.size,
        members: c.members.into_iter().map(solid_symbol_ref).collect(),
        members_omitted: c.members_omitted,
    }
}

#[must_use]
pub fn solid_edge(e: ministr_core::service::SolidEdge) -> query::SolidEdge {
    query::SolidEdge {
        from: e.from,
        to: e.to,
        example_from: solid_symbol_ref(e.example_from),
        example_to: solid_symbol_ref(e.example_to),
    }
}

#[must_use]
pub fn solid_finding(f: ministr_core::service::SolidFinding) -> query::SolidFinding {
    match f {
        ministr_core::service::SolidFinding::Redundancy {
            principle,
            members,
            members_omitted,
            members_total,
            canonical,
            avg_cosine,
            avg_jaccard,
            cross_module,
        } => query::SolidFinding::Redundancy {
            principle: solid_principle(principle),
            members: members.into_iter().map(solid_symbol_ref).collect(),
            members_omitted,
            members_total,
            canonical: solid_symbol_ref(canonical),
            avg_cosine,
            avg_jaccard,
            cross_module,
        },
        ministr_core::service::SolidFinding::LowCohesion {
            principle,
            container,
            components,
            method_count,
        } => query::SolidFinding::LowCohesion {
            principle: solid_principle(principle),
            container: solid_symbol_ref(container),
            components: components.into_iter().map(solid_component).collect(),
            method_count,
        },
        ministr_core::service::SolidFinding::FatInterface {
            principle,
            interface,
            method_count,
            unused_methods,
            unused_methods_omitted,
            under_using_implementors,
            under_using_implementors_omitted,
        } => query::SolidFinding::FatInterface {
            principle: solid_principle(principle),
            interface: solid_symbol_ref(interface),
            method_count,
            unused_methods,
            unused_methods_omitted,
            under_using_implementors: under_using_implementors
                .into_iter()
                .map(solid_symbol_ref)
                .collect(),
            under_using_implementors_omitted,
        },
        ministr_core::service::SolidFinding::ConcreteDependency {
            principle,
            consumer,
            concrete_target,
            suggested_abstraction,
        } => query::SolidFinding::ConcreteDependency {
            principle: solid_principle(principle),
            consumer: solid_symbol_ref(consumer),
            concrete_target: solid_symbol_ref(concrete_target),
            suggested_abstraction: suggested_abstraction.map(solid_symbol_ref),
        },
        ministr_core::service::SolidFinding::ShotgunSurgery {
            principle,
            name,
            kind,
            sites,
            sites_omitted,
            sites_total,
            avg_jaccard,
        } => query::SolidFinding::ShotgunSurgery {
            principle: solid_principle(principle),
            name,
            kind,
            sites: sites.into_iter().map(solid_symbol_ref).collect(),
            sites_omitted,
            sites_total,
            avg_jaccard,
        },
        ministr_core::service::SolidFinding::CyclicDependency {
            principle,
            packages,
            edge_count,
            example_edges,
            example_edges_omitted,
        } => query::SolidFinding::CyclicDependency {
            principle: solid_principle(principle),
            packages,
            edge_count,
            example_edges: example_edges.into_iter().map(solid_edge).collect(),
            example_edges_omitted,
        },
    }
}

/// Translate the on-the-wire request into the core service params, applying
/// the documented defaults when fields are omitted.
#[must_use]
pub fn api_solid_request_to_service(r: query::SolidRequest) -> ministr_core::service::SolidParams {
    let defaults = ministr_core::service::SolidParams::default();
    ministr_core::service::SolidParams {
        kind: r.kind,
        module: r.module,
        principles: r.principles.into_iter().map(api_solid_principle).collect(),
        container_kinds: if r.container_kinds.is_empty() {
            defaults.container_kinds
        } else {
            r.container_kinds
        },
        interface_kinds: if r.interface_kinds.is_empty() {
            defaults.interface_kinds
        } else {
            r.interface_kinds
        },
        similarity_threshold: r
            .similarity_threshold
            .unwrap_or(defaults.similarity_threshold),
        jaccard_threshold: r.jaccard_threshold.unwrap_or(defaults.jaccard_threshold),
        srp_cohesion_threshold: r
            .srp_cohesion_threshold
            .unwrap_or(defaults.srp_cohesion_threshold),
        isp_min_methods: r.isp_min_methods.unwrap_or(defaults.isp_min_methods),
        isp_max_overlap_fraction: r
            .isp_max_overlap_fraction
            .unwrap_or(defaults.isp_max_overlap_fraction),
        min_lines: r.min_lines.unwrap_or(defaults.min_lines),
        limit: r.limit.unwrap_or(defaults.limit).clamp(1, 500),
        max_pairs: r.max_pairs.unwrap_or(defaults.max_pairs),
        representative_count: r
            .representative_count
            .unwrap_or(defaults.representative_count),
        shotgun_min_sites: r.shotgun_min_sites.unwrap_or(defaults.shotgun_min_sites),
        shotgun_max_jaccard: r
            .shotgun_max_jaccard
            .unwrap_or(defaults.shotgun_max_jaccard),
        shotgun_min_packages: r
            .shotgun_min_packages
            .unwrap_or(defaults.shotgun_min_packages),
        shotgun_skip_conventional_names: r
            .shotgun_skip_conventional_names
            .unwrap_or(defaults.shotgun_skip_conventional_names),
        cyclic_min_edges_per_direction: r
            .cyclic_min_edges_per_direction
            .unwrap_or(defaults.cyclic_min_edges_per_direction),
        cyclic_skip_test_paths: r
            .cyclic_skip_test_paths
            .unwrap_or(defaults.cyclic_skip_test_paths),
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
