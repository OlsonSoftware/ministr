//! Reverse wire-type conversions: ministr-api → ministr-core service types.
//!
//! `ministr-daemon/src/convert.rs` covers the forward direction
//! (service → api), which the daemon needs to serialize responses. Daemon
//! backend ([`super::daemon::DaemonBackend`]) needs the reverse so it can
//! return service-layer types and let the MCP handler treat both backends
//! interchangeably.
//!
//! These helpers live in their own module so that adding a new backend
//! method only requires adding one converter here per new return type.

use ministr_core::service::{
    ClaimResult, CompressedItem, ImpactCaller, ImpactResult, ImpactRisk, RelatedClaimResult,
    SectionDetail, SolidComponent, SolidEdge, SolidFinding, SolidParams, SolidPrinciple,
    SolidSymbolRef, SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::{BridgeLinkDetail, SymbolRecord};
use ministr_core::types::{ContentId, SectionId, SymbolId, TocEntry};

pub(super) fn api_survey_to_service(r: ministr_api::query::SurveyResult) -> SurveyResult {
    SurveyResult {
        content_id: r.content_id,
        resolution: r.resolution,
        score: r.score,
        text: r.text,
        heading_path: r.heading_path,
        source_corpus: r.source_corpus,
    }
}

pub(super) fn api_section_to_service(s: ministr_api::query::SectionDetail) -> SectionDetail {
    // api::SectionDetail carries extra wire-only fields (`status`,
    // `usage_status`); drop them — they're envelope hints, not section
    // content.
    SectionDetail {
        section_id: s.section_id,
        heading_path: s.heading_path,
        text: s.text,
        summary: s.summary,
        claims_available: s.claims_available,
    }
}

pub(super) fn api_claim_to_service(c: ministr_api::query::ClaimResult) -> ClaimResult {
    ClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relevance: c.relevance,
    }
}

pub(super) fn api_symbol_def_to_service(
    s: ministr_api::query::SymbolDefinition,
) -> SymbolDefinition {
    SymbolDefinition {
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

/// `/symbols` endpoint returns `api::SymbolDefinition` (carries
/// `source_context` when present). `LocalBackend`'s `search_symbols`
/// returns the leaner `storage::SymbolRecord`. Convert down —
/// `source_context` is dropped (matches what the daemon's symbols list
/// endpoint actually returns in practice).
pub(super) fn api_symbol_def_to_record(s: ministr_api::query::SymbolDefinition) -> SymbolRecord {
    SymbolRecord {
        id: SymbolId(s.id),
        file_path: s.file_path,
        name: s.name,
        kind: s.kind,
        visibility: s.visibility,
        signature: s.signature,
        doc_comment: s.doc_comment,
        module_path: s.heading_path.join("::"),
        line_start: s.line_start,
        line_end: s.line_end,
        cyclomatic_complexity: None,
    }
}

pub(super) fn api_symbol_reference_to_service(
    r: ministr_api::query::SymbolReference,
) -> SymbolRefResult {
    SymbolRefResult {
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

pub(super) fn api_impact_to_service(r: ministr_api::query::ImpactResponse) -> ImpactResult {
    ImpactResult {
        target_symbol_id: r.target_symbol_id,
        direction: ministr_core::service::CallDirection::parse(&r.direction).unwrap_or_default(),
        depth: r.depth,
        symbols: r.symbols,
        files: r.files,
        tests: r.tests,
        risk: api_impact_risk_to_service(r.risk),
        callers: r
            .callers
            .into_iter()
            .map(api_impact_caller_to_service)
            .collect(),
    }
}

fn api_impact_caller_to_service(c: ministr_api::query::ImpactCaller) -> ImpactCaller {
    ImpactCaller {
        symbol_id: c.symbol_id,
        name: c.name,
        kind: c.kind,
        file: c.file,
        line: c.line,
        depth: c.depth,
    }
}

fn api_impact_risk_to_service(r: ministr_api::query::ImpactRisk) -> ImpactRisk {
    match r {
        ministr_api::query::ImpactRisk::Low => ImpactRisk::Low,
        ministr_api::query::ImpactRisk::Medium => ImpactRisk::Medium,
        ministr_api::query::ImpactRisk::High => ImpactRisk::High,
    }
}

pub(super) fn api_dead_symbol_to_service(
    s: ministr_api::query::DeadSymbol,
) -> ministr_core::service::DeadSymbol {
    ministr_core::service::DeadSymbol {
        symbol_id: s.symbol_id,
        name: s.name,
        kind: s.kind,
        visibility: s.visibility,
        file: s.file,
        line: s.line,
        lines: s.lines,
    }
}

fn api_solid_principle_to_service(p: ministr_api::query::SolidPrinciple) -> SolidPrinciple {
    match p {
        ministr_api::query::SolidPrinciple::DryOcp => SolidPrinciple::DryOcp,
        ministr_api::query::SolidPrinciple::Srp => SolidPrinciple::Srp,
        ministr_api::query::SolidPrinciple::Isp => SolidPrinciple::Isp,
        ministr_api::query::SolidPrinciple::Dip => SolidPrinciple::Dip,
        ministr_api::query::SolidPrinciple::ShotgunSurgery => SolidPrinciple::ShotgunSurgery,
        ministr_api::query::SolidPrinciple::CyclicDependency => SolidPrinciple::CyclicDependency,
    }
}

fn service_solid_principle_to_api(p: SolidPrinciple) -> ministr_api::query::SolidPrinciple {
    match p {
        SolidPrinciple::DryOcp => ministr_api::query::SolidPrinciple::DryOcp,
        SolidPrinciple::Srp => ministr_api::query::SolidPrinciple::Srp,
        SolidPrinciple::Isp => ministr_api::query::SolidPrinciple::Isp,
        SolidPrinciple::Dip => ministr_api::query::SolidPrinciple::Dip,
        SolidPrinciple::ShotgunSurgery => ministr_api::query::SolidPrinciple::ShotgunSurgery,
        SolidPrinciple::CyclicDependency => ministr_api::query::SolidPrinciple::CyclicDependency,
    }
}

fn api_solid_component_to_service(c: ministr_api::query::SolidComponent) -> SolidComponent {
    SolidComponent {
        size: c.size,
        members: c
            .members
            .into_iter()
            .map(api_solid_symbol_ref_to_service)
            .collect(),
        members_omitted: c.members_omitted,
    }
}

fn api_solid_edge_to_service(e: ministr_api::query::SolidEdge) -> SolidEdge {
    SolidEdge {
        from: e.from,
        to: e.to,
        example_from: api_solid_symbol_ref_to_service(e.example_from),
        example_to: api_solid_symbol_ref_to_service(e.example_to),
    }
}

fn api_solid_symbol_ref_to_service(s: ministr_api::query::SolidSymbolRef) -> SolidSymbolRef {
    SolidSymbolRef {
        symbol_id: s.symbol_id,
        name: s.name,
        kind: s.kind,
        file: s.file,
        line: s.line,
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn api_solid_finding_to_service(f: ministr_api::query::SolidFinding) -> SolidFinding {
    match f {
        ministr_api::query::SolidFinding::Redundancy {
            principle,
            members,
            members_omitted,
            members_total,
            canonical,
            avg_cosine,
            avg_jaccard,
            cross_module,
        } => SolidFinding::Redundancy {
            principle: api_solid_principle_to_service(principle),
            members: members
                .into_iter()
                .map(api_solid_symbol_ref_to_service)
                .collect(),
            members_omitted,
            members_total,
            canonical: api_solid_symbol_ref_to_service(canonical),
            avg_cosine,
            avg_jaccard,
            cross_module,
        },
        ministr_api::query::SolidFinding::LowCohesion {
            principle,
            container,
            components,
            method_count,
        } => SolidFinding::LowCohesion {
            principle: api_solid_principle_to_service(principle),
            container: api_solid_symbol_ref_to_service(container),
            components: components
                .into_iter()
                .map(api_solid_component_to_service)
                .collect(),
            method_count,
        },
        ministr_api::query::SolidFinding::FatInterface {
            principle,
            interface,
            method_count,
            unused_methods,
            unused_methods_omitted,
            under_using_implementors,
            under_using_implementors_omitted,
        } => SolidFinding::FatInterface {
            principle: api_solid_principle_to_service(principle),
            interface: api_solid_symbol_ref_to_service(interface),
            method_count,
            unused_methods,
            unused_methods_omitted,
            under_using_implementors: under_using_implementors
                .into_iter()
                .map(api_solid_symbol_ref_to_service)
                .collect(),
            under_using_implementors_omitted,
        },
        ministr_api::query::SolidFinding::ConcreteDependency {
            principle,
            consumer,
            concrete_target,
            suggested_abstraction,
        } => SolidFinding::ConcreteDependency {
            principle: api_solid_principle_to_service(principle),
            consumer: api_solid_symbol_ref_to_service(consumer),
            concrete_target: api_solid_symbol_ref_to_service(concrete_target),
            suggested_abstraction: suggested_abstraction.map(api_solid_symbol_ref_to_service),
        },
        ministr_api::query::SolidFinding::ShotgunSurgery {
            principle,
            name,
            kind,
            sites,
            sites_omitted,
            sites_total,
            avg_jaccard,
        } => SolidFinding::ShotgunSurgery {
            principle: api_solid_principle_to_service(principle),
            name,
            kind,
            sites: sites
                .into_iter()
                .map(api_solid_symbol_ref_to_service)
                .collect(),
            sites_omitted,
            sites_total,
            avg_jaccard,
        },
        ministr_api::query::SolidFinding::CyclicDependency {
            principle,
            packages,
            edge_count,
            example_edges,
            example_edges_omitted,
        } => SolidFinding::CyclicDependency {
            principle: api_solid_principle_to_service(principle),
            packages,
            edge_count,
            example_edges: example_edges
                .into_iter()
                .map(api_solid_edge_to_service)
                .collect(),
            example_edges_omitted,
        },
    }
}

/// Forward-direction translator used by [`super::daemon::DaemonBackend`] —
/// the daemon backend takes service-shaped params and forwards them as the
/// JSON request body.
pub(super) fn service_solid_params_to_api(p: SolidParams) -> ministr_api::query::SolidRequest {
    ministr_api::query::SolidRequest {
        kind: p.kind,
        module: p.module,
        principles: p
            .principles
            .into_iter()
            .map(service_solid_principle_to_api)
            .collect(),
        container_kinds: p.container_kinds,
        interface_kinds: p.interface_kinds,
        similarity_threshold: Some(p.similarity_threshold),
        jaccard_threshold: Some(p.jaccard_threshold),
        srp_cohesion_threshold: Some(p.srp_cohesion_threshold),
        isp_min_methods: Some(p.isp_min_methods),
        isp_max_overlap_fraction: Some(p.isp_max_overlap_fraction),
        min_lines: Some(p.min_lines),
        limit: Some(p.limit),
        max_pairs: Some(p.max_pairs),
        representative_count: Some(p.representative_count),
        shotgun_min_sites: Some(p.shotgun_min_sites),
        shotgun_max_jaccard: Some(p.shotgun_max_jaccard),
        shotgun_min_packages: Some(p.shotgun_min_packages),
        shotgun_skip_conventional_names: Some(p.shotgun_skip_conventional_names),
        cyclic_min_edges_per_direction: Some(p.cyclic_min_edges_per_direction),
        cyclic_skip_test_paths: Some(p.cyclic_skip_test_paths),
    }
}

pub(super) fn api_related_to_service(
    c: ministr_api::query::RelatedClaimResult,
) -> RelatedClaimResult {
    RelatedClaimResult {
        claim_id: c.claim_id,
        text: c.text,
        relation_type: c.relation_type,
        source_section: c.source_section,
        confidence: c.confidence,
    }
}

pub(super) fn api_compressed_to_service(
    c: ministr_api::session::CompressedItemApi,
) -> CompressedItem {
    CompressedItem {
        original_id: c.original_id,
        summary: c.summary,
        original_tokens: c.original_tokens,
        compressed_tokens: c.compressed_tokens,
        method: c.method,
    }
}

/// Convert an `api::TocEntry` (daemon wire shape) back to the rich service
/// [`TocEntry`]. As of the toc-schema-convergence work, `api::TocEntry`
/// carries `heading_path`, `claims_available`, and `token_count`, so daemon
/// mode is at parity with local mode; `document_id` rides on `source_path`.
/// The `heading_path` fallback to `[title]` keeps this lossless-enough against
/// an older daemon that only sent the leaf `title`.
pub(super) fn api_toc_to_service(e: ministr_api::query::TocEntry) -> TocEntry {
    let heading_path = if e.heading_path.is_empty() {
        if e.title.is_empty() {
            Vec::new()
        } else {
            vec![e.title]
        }
    } else {
        e.heading_path
    };
    TocEntry {
        document_id: e
            .source_path
            .map_or_else(|| ContentId(String::new()), ContentId),
        section_id: SectionId(e.id),
        heading_path,
        depth: u32::try_from(e.depth).unwrap_or(0),
        claims_available: e.claims_available,
        token_count: e.token_count,
    }
}

/// Convert an `api::BridgeLink` (daemon wire shape) back to the rich
/// [`BridgeLinkDetail`]. As of the schema-convergence work the per-endpoint
/// binding key (`source`/`target`), symbol, file, and line all ride the wire,
/// so daemon mode is at parity with local mode. Only the internal
/// `*_symbol_id` heuristic ids are not carried — they are not surfaced by the
/// MCP bridge result.
pub(super) fn api_bridge_to_storage(l: ministr_api::query::BridgeLink) -> BridgeLinkDetail {
    BridgeLinkDetail {
        kind: l.kind,
        confidence: l.confidence,
        export_file: l.export_file,
        export_binding_key: l.source,
        export_symbol: l.export_symbol,
        export_language: l.source_language,
        export_line: l.export_line,
        export_symbol_id: None,
        import_file: l.import_file,
        import_binding_key: l.target,
        import_symbol: l.import_symbol,
        import_language: l.target_language,
        import_line: l.import_line,
        import_symbol_id: None,
    }
}

// ---------------------------------------------------------------------------
// Tests — verify wire-type conversions preserve every field. TOC is now
// lossless (heading_path / claims_available / token_count round-trip); the
// bridge path is still lossy (per-endpoint file/line/binding_key drop) and is
// tested on the fields that DO survive.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survey_round_trips_every_field() {
        let api = ministr_api::query::SurveyResult {
            content_id: "doc.md#sec".into(),
            resolution: "claim".into(),
            score: 0.91,
            text: "the body".into(),
            heading_path: Some(vec!["A".into(), "B".into()]),
            source_corpus: Some("atlas/react".into()),
        };
        let svc = api_survey_to_service(api.clone());
        assert_eq!(svc.content_id, api.content_id);
        assert_eq!(svc.resolution, api.resolution);
        assert!((svc.score - api.score).abs() < f32::EPSILON);
        assert_eq!(svc.text, api.text);
        assert_eq!(svc.heading_path, api.heading_path);
        assert_eq!(svc.source_corpus, api.source_corpus);
    }

    #[test]
    fn symbol_reference_round_trips_every_field() {
        let api = ministr_api::query::SymbolReference {
            from_symbol_id: "sym-a".into(),
            from_name: "a".into(),
            from_file: "a.rs".into(),
            from_line: 10,
            to_symbol_id: "sym-b".into(),
            to_name: "b".into(),
            to_file: "b.rs".into(),
            to_line: 20,
            ref_kind: "calls".into(),
        };
        let svc = api_symbol_reference_to_service(api.clone());
        assert_eq!(svc.from_symbol_id, api.from_symbol_id);
        assert_eq!(svc.from_line, api.from_line);
        assert_eq!(svc.to_symbol_id, api.to_symbol_id);
        assert_eq!(svc.to_line, api.to_line);
        assert_eq!(svc.ref_kind, api.ref_kind);
    }

    #[test]
    fn impact_round_trips_every_field() {
        let api = ministr_api::query::ImpactResponse {
            target_symbol_id: "sym-x".into(),
            direction: "outgoing".into(),
            depth: 3,
            symbols: 7,
            files: 4,
            tests: 2,
            risk: ministr_api::query::ImpactRisk::High,
            callers: vec![ministr_api::query::ImpactCaller {
                symbol_id: "sym-c".into(),
                name: "c".into(),
                kind: "function".into(),
                file: "c.rs".into(),
                line: 100,
                depth: 1,
            }],
        };
        let svc = api_impact_to_service(api.clone());
        assert_eq!(svc.target_symbol_id, api.target_symbol_id);
        assert_eq!(
            svc.direction,
            ministr_core::service::CallDirection::Outgoing,
            "direction must round-trip api→service"
        );
        assert_eq!(svc.symbols, api.symbols);
        assert_eq!(svc.files, api.files);
        assert_eq!(svc.tests, api.tests);
        assert!(matches!(svc.risk, ImpactRisk::High));
        assert_eq!(svc.callers.len(), 1);
        assert_eq!(svc.callers[0].symbol_id, "sym-c");
        assert_eq!(svc.callers[0].depth, 1);
    }

    #[test]
    fn dead_symbol_round_trips_every_field() {
        let api = ministr_api::query::DeadSymbol {
            symbol_id: "sym-d".into(),
            name: "d".into(),
            kind: "function".into(),
            visibility: "pub(crate)".into(),
            file: "d.rs".into(),
            line: 42,
            lines: 18,
        };
        let svc = api_dead_symbol_to_service(api.clone());
        assert_eq!(svc.symbol_id, api.symbol_id);
        assert_eq!(svc.lines, api.lines);
        assert_eq!(svc.visibility, api.visibility);
    }

    #[test]
    fn related_round_trips_every_field() {
        let api = ministr_api::query::RelatedClaimResult {
            claim_id: "claim-1".into(),
            text: "the claim".into(),
            relation_type: "references".into(),
            source_section: "doc.md#intro".into(),
            confidence: 0.84,
        };
        let svc = api_related_to_service(api.clone());
        assert_eq!(svc.claim_id, api.claim_id);
        assert_eq!(svc.relation_type, api.relation_type);
        assert!((svc.confidence - api.confidence).abs() < f32::EPSILON);
    }

    #[test]
    fn compressed_round_trips_every_field() {
        let api = ministr_api::session::CompressedItemApi {
            original_id: "doc.md#a".into(),
            summary: "short".into(),
            original_tokens: 100,
            compressed_tokens: 25,
            method: "extractive".into(),
        };
        let svc = api_compressed_to_service(api.clone());
        assert_eq!(svc.original_id, api.original_id);
        assert_eq!(svc.original_tokens, api.original_tokens);
        assert_eq!(svc.compressed_tokens, api.compressed_tokens);
        assert_eq!(svc.method, api.method);
    }

    #[test]
    fn toc_round_trips_rich_fields() {
        // toc-schema-convergence: heading_path, claims_available, and
        // token_count now ride the wire, so daemon mode is lossless.
        let api = ministr_api::query::TocEntry {
            id: "doc.md#sec1".into(),
            title: "Section 1".into(),
            kind: "section".into(),
            depth: 2,
            children: 3,
            source_path: Some("docs/doc.md".into()),
            heading_path: vec!["Doc".into(), "Section 1".into()],
            claims_available: 4,
            token_count: 321,
        };
        let svc = api_toc_to_service(api.clone());
        assert_eq!(svc.section_id.0, api.id);
        assert_eq!(svc.document_id.0, "docs/doc.md");
        assert_eq!(svc.depth, u32::try_from(api.depth).unwrap());
        assert_eq!(
            svc.heading_path,
            vec!["Doc".to_string(), "Section 1".to_string()]
        );
        assert_eq!(svc.claims_available, 4);
        assert_eq!(svc.token_count, 321);
    }

    #[test]
    fn toc_falls_back_to_title_when_heading_path_empty() {
        // Back-compat: an older daemon only sends the leaf `title`.
        let api = ministr_api::query::TocEntry {
            id: "doc.md#sec1".into(),
            title: "Section 1".into(),
            kind: "section".into(),
            depth: 2,
            children: 0,
            source_path: Some("docs/doc.md".into()),
            heading_path: vec![],
            claims_available: 0,
            token_count: 0,
        };
        let svc = api_toc_to_service(api);
        assert_eq!(svc.heading_path, vec!["Section 1".to_string()]);
    }

    #[test]
    fn bridge_round_trips_rich_fields() {
        // schema-convergence: binding key (source/target), symbol, file, and
        // line all survive; only the internal symbol_id is intentionally
        // dropped (not surfaced by the MCP bridge result).
        let api = ministr_api::query::BridgeLink {
            kind: "tauri_command".into(),
            source: "auth.validateToken".into(),
            source_language: "rust".into(),
            target: "auth.invoke".into(),
            target_language: "typescript".into(),
            confidence: 0.95,
            export_symbol: "validate_token".into(),
            export_file: "src/auth.rs".into(),
            export_line: 12,
            import_symbol: "invoke".into(),
            import_file: "src/api.ts".into(),
            import_line: 88,
        };
        let svc = api_bridge_to_storage(api.clone());
        assert_eq!(svc.kind, api.kind);
        assert!((svc.confidence - api.confidence).abs() < f32::EPSILON);
        // Binding key rides source/target.
        assert_eq!(svc.export_binding_key, "auth.validateToken");
        assert_eq!(svc.import_binding_key, "auth.invoke");
        // Per-endpoint symbol/file/line preserved.
        assert_eq!(svc.export_symbol, "validate_token");
        assert_eq!(svc.export_file, "src/auth.rs");
        assert_eq!(svc.export_line, 12);
        assert_eq!(svc.import_symbol, "invoke");
        assert_eq!(svc.import_file, "src/api.ts");
        assert_eq!(svc.import_line, 88);
        assert_eq!(svc.export_language, api.source_language);
        assert_eq!(svc.import_language, api.target_language);
    }
}
