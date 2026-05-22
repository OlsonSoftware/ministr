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
        source_corpus: r.source_corpus,
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

/// F3.6-a — pure helper that converts a flat list of bridge-link
/// details into the `{nodes, edges}` wire shape the F3.6-b web
/// visualizer expects.
///
/// # Node identity
///
/// `id = "{file}::{symbol}::{line}"` — two same-named symbols in
/// different files or on different lines deliberately become
/// different nodes (a future renamed symbol leaves a stale node
/// behind in a long-running corpus; we want the graph to reflect
/// the current ground truth on each query). A symbol that
/// participates in multiple edges produces exactly one node (dedup
/// via `HashSet` on the id).
///
/// # Output ordering
///
/// Nodes are returned in **first-encounter order** across the input
/// links (export-side of link 0, then import-side of link 0, then
/// export-side of link 1, …). Edges follow the input order. Stable
/// ordering matters for the F3.6-b visualizer's layout (Cytoscape /
/// react-flow re-run physics on identity changes — same input must
/// produce the same node order).
#[must_use]
pub fn bridge_links_to_graph(
    links: &[ministr_core::storage::BridgeLinkDetail],
) -> query::BridgeGraph {
    fn make_id(file: &str, symbol: &str, line: u32) -> String {
        format!("{file}::{symbol}::{line}")
    }

    let mut nodes: Vec<query::BridgeNode> = Vec::new();
    let mut edges: Vec<query::BridgeEdge> = Vec::with_capacity(links.len());
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(links.len() * 2);

    for l in links {
        let from_id = make_id(&l.export_file, &l.export_symbol, l.export_line);
        let to_id = make_id(&l.import_file, &l.import_symbol, l.import_line);
        if seen.insert(from_id.clone()) {
            nodes.push(query::BridgeNode {
                id: from_id.clone(),
                label: l.export_symbol.clone(),
                file: l.export_file.clone(),
                lang: l.export_language.clone(),
                line: l.export_line,
            });
        }
        if seen.insert(to_id.clone()) {
            nodes.push(query::BridgeNode {
                id: to_id.clone(),
                label: l.import_symbol.clone(),
                file: l.import_file.clone(),
                lang: l.import_language.clone(),
                line: l.import_line,
            });
        }
        edges.push(query::BridgeEdge {
            from: from_id,
            to: to_id,
            kind: l.kind.clone(),
            confidence: l.confidence,
        });
    }

    query::BridgeGraph { nodes, edges }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_core::storage::BridgeLinkDetail;

    #[allow(clippy::too_many_arguments)] // test helper — flat positional args mirror the struct shape; a builder would just inflate the suite
    fn link(
        kind: &str,
        export_file: &str,
        export_symbol: &str,
        export_lang: &str,
        export_line: u32,
        import_file: &str,
        import_symbol: &str,
        import_lang: &str,
        import_line: u32,
        confidence: f32,
    ) -> BridgeLinkDetail {
        BridgeLinkDetail {
            kind: kind.into(),
            confidence,
            export_file: export_file.into(),
            export_binding_key: format!("{export_symbol}@{export_file}"),
            export_symbol: export_symbol.into(),
            export_language: export_lang.into(),
            export_line,
            import_file: import_file.into(),
            import_binding_key: format!("{import_symbol}@{import_file}"),
            import_symbol: import_symbol.into(),
            import_language: import_lang.into(),
            import_line,
        }
    }

    #[test]
    fn empty_links_yields_empty_graph() {
        let graph = bridge_links_to_graph(&[]);
        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn single_link_emits_two_nodes_and_one_edge() {
        let links = vec![link(
            "tauri_command",
            "src-tauri/src/main.rs",
            "cloud_status",
            "rust",
            42,
            "src/lib/cloudClient.ts",
            "cloudStatus",
            "typescript",
            10,
            0.95,
        )];
        let graph = bridge_links_to_graph(&links);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        // Export side first.
        assert_eq!(graph.nodes[0].label, "cloud_status");
        assert_eq!(graph.nodes[0].lang, "rust");
        assert_eq!(graph.nodes[0].line, 42);
        // Import side second.
        assert_eq!(graph.nodes[1].label, "cloudStatus");
        assert_eq!(graph.nodes[1].lang, "typescript");
        // Edge wires the two by id.
        assert_eq!(graph.edges[0].from, graph.nodes[0].id);
        assert_eq!(graph.edges[0].to, graph.nodes[1].id);
        assert_eq!(graph.edges[0].kind, "tauri_command");
        assert!((graph.edges[0].confidence - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn shared_endpoint_dedupes_to_one_node() {
        // Two edges both originate from the same Tauri command.
        let links = vec![
            link(
                "tauri_command",
                "src-tauri/src/main.rs",
                "shared_cmd",
                "rust",
                100,
                "src/a.ts",
                "callA",
                "typescript",
                1,
                0.9,
            ),
            link(
                "tauri_command",
                "src-tauri/src/main.rs",
                "shared_cmd",
                "rust",
                100,
                "src/b.ts",
                "callB",
                "typescript",
                2,
                0.9,
            ),
        ];
        let graph = bridge_links_to_graph(&links);
        // shared_cmd node appears ONCE; callA + callB each appear once → 3 nodes.
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        // Both edges' `from` reference the same id.
        assert_eq!(graph.edges[0].from, graph.edges[1].from);
    }

    #[test]
    fn same_symbol_in_different_files_is_two_distinct_nodes() {
        let links = vec![
            link(
                "pyo3",
                "src/exporter_a.rs",
                "handle_event",
                "rust",
                10,
                "pkg/__init__.py",
                "handle_event",
                "python",
                5,
                0.8,
            ),
            link(
                "pyo3",
                "src/exporter_b.rs",
                "handle_event",
                "rust",
                20,
                "pkg/other.py",
                "handle_event",
                "python",
                7,
                0.8,
            ),
        ];
        let graph = bridge_links_to_graph(&links);
        // 4 distinct nodes because the (file, symbol, line) tuples
        // all differ even though some symbol names match.
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 2);
        // Sanity: all node ids are unique.
        let mut ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn node_id_format_is_file_symbol_line() {
        let links = vec![link(
            "napi",
            "rust-side.rs",
            "exporter",
            "rust",
            7,
            "ts-side.ts",
            "importer",
            "typescript",
            3,
            0.5,
        )];
        let graph = bridge_links_to_graph(&links);
        assert_eq!(graph.nodes[0].id, "rust-side.rs::exporter::7");
        assert_eq!(graph.nodes[1].id, "ts-side.ts::importer::3");
    }

    #[test]
    fn nodes_appear_in_first_encounter_order() {
        // Determinism guard: re-rendering with the same data must
        // produce the same layout, so node order MUST be stable.
        let links = vec![
            link("a", "f1", "s1", "rust", 1, "g1", "t1", "typescript", 1, 0.5),
            link("b", "f2", "s2", "python", 1, "g2", "t2", "rust", 1, 0.5),
        ];
        let graph = bridge_links_to_graph(&links);
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.nodes[0].label, "s1");
        assert_eq!(graph.nodes[1].label, "t1");
        assert_eq!(graph.nodes[2].label, "s2");
        assert_eq!(graph.nodes[3].label, "t2");
    }
}
