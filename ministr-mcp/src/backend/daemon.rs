//! [`DaemonBackend`] — HTTP-forwarding implementation of [`QueryBackend`].
//!
//! Every call becomes a JSON-RPC request to a running `ministr-daemon` via
//! the shared [`DaemonClient`]. Use when ministr is running as a stdio MCP
//! proxy that delegates the index to a separate daemon process.

use std::future::Future;
use std::sync::Arc;

use ministr_api::client::DaemonClient;
use ministr_core::service::{
    ClaimResult, CompressedItem, DeadSymbol, ImpactResult, RelatedClaimResult, SectionDetail,
    SolidFinding, SolidParams, SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::traits::{OccurrenceRecord, occurrence_at};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{RefKind, RelationType, SymbolId, TocEntry};

use super::convert::{
    api_bridge_to_storage, api_claim_to_service, api_compressed_to_service,
    api_dead_symbol_to_service, api_impact_to_service, api_related_to_service,
    api_section_to_service, api_solid_finding_to_service, api_survey_to_service,
    api_symbol_def_to_record, api_symbol_def_to_service, api_symbol_reference_to_service,
    api_toc_to_service, service_solid_params_to_api,
};
use super::{BackendError, QueryBackend};

/// Backend that forwards every operation to a running `ministr-daemon`.
pub struct DaemonBackend {
    client: Arc<DaemonClient>,
    corpus_id: String,
    session_id: Option<String>,
}

impl DaemonBackend {
    #[must_use]
    pub fn new(client: Arc<DaemonClient>, corpus_id: String, session_id: Option<String>) -> Self {
        Self {
            client,
            corpus_id,
            session_id,
        }
    }

    /// Borrow the underlying daemon client (for tools like `ministr_clone`
    /// that call daemon endpoints not covered by [`QueryBackend`]).
    #[must_use]
    pub fn client(&self) -> &Arc<DaemonClient> {
        &self.client
    }

    /// The parent corpus id this backend is bound to.
    #[must_use]
    pub fn corpus_id(&self) -> &str {
        &self.corpus_id
    }
}

impl QueryBackend for DaemonBackend {
    fn survey(
        &self,
        query: &str,
        top_k: usize,
    ) -> impl Future<Output = Result<Vec<SurveyResult>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::SurveyRequest {
            query: query.to_string(),
            top_k: Some(top_k),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.survey_req(&corpus_id, &req).await?;
            Ok(resp
                .results
                .into_iter()
                .map(api_survey_to_service)
                .collect())
        }
    }

    fn survey_with_exclude(
        &self,
        query: &str,
        top_k: usize,
        _exclude_ids: &std::collections::HashSet<String>,
    ) -> impl Future<Output = Result<(Vec<SurveyResult>, usize), BackendError>> + Send {
        // The daemon dedupes server-side using `session_id` captured at
        // construction — `exclude_ids` is intentionally ignored. The daemon
        // doesn't expose a dedup-count field today, so we return 0; if/when
        // the daemon's survey response carries `deduplicated_count`, plumb
        // it through here.
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::SurveyRequest {
            query: query.to_string(),
            top_k: Some(top_k),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.survey_req(&corpus_id, &req).await?;
            let results: Vec<SurveyResult> = resp
                .results
                .into_iter()
                .map(api_survey_to_service)
                .collect();
            let deduplicated = resp.deduplicated_count.unwrap_or(0);
            Ok((results, deduplicated))
        }
    }

    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<SectionDetail, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let section_id = section_id.to_string();
        async move {
            let resp = client.read_section(&corpus_id, &section_id).await?;
            Ok(api_section_to_service(resp))
        }
    }

    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ClaimResult>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::ExtractRequest {
            section_id: section_id.to_string(),
            query: query.map(String::from),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.extract(&corpus_id, &req).await?;
            Ok(resp.claims.into_iter().map(api_claim_to_service).collect())
        }
    }

    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<Vec<SymbolRecord>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        async move {
            let req = ministr_api::query::SymbolsRequest {
                query: filter.name.clone().unwrap_or_default(),
                kind: filter.kind.clone(),
                module: filter.module.clone(),
                visibility: filter.visibility.clone(),
                // gd2c-2: the daemon now honors a file_path filter, so the
                // daemon-backend forwards it instead of silently dropping it.
                file_path: filter.file_path.clone(),
                limit: None,
                session_id,
            };
            let resp = client.symbols(&corpus_id, &req).await?;
            Ok(resp
                .symbols
                .into_iter()
                .map(api_symbol_def_to_record)
                .collect())
        }
    }

    fn definition(
        &self,
        symbol_id: &str,
    ) -> impl Future<Output = Result<SymbolDefinition, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .definition(&corpus_id, &symbol_id, session_id.as_deref())
                .await?;
            Ok(api_symbol_def_to_service(resp))
        }
    }

    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> impl Future<Output = Result<Vec<SymbolRefResult>, BackendError>> + Send {
        // The daemon HTTP route doesn't accept a ref_kind filter, so apply it
        // client-side here for parity with `LocalBackend` (which filters in
        // the query service). Without this the `ref_kind` argument was
        // silently dropped on the daemon path — the mode the stdio MCP proxy
        // actually runs in.
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .references(&corpus_id, &symbol_id, session_id.as_deref())
                .await?;
            let mut refs: Vec<SymbolRefResult> = resp
                .references
                .into_iter()
                .map(api_symbol_reference_to_service)
                .collect();
            retain_ref_kind(&mut refs, ref_kind);
            Ok(refs)
        }
    }

    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
    ) -> impl Future<Output = Result<ImpactResult, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .impact(
                    &corpus_id,
                    &symbol_id,
                    Some(max_depth),
                    session_id.as_deref(),
                )
                .await?;
            Ok(api_impact_to_service(resp))
        }
    }

    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<DeadSymbol>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        let req = ministr_api::query::DeadCodeRequest {
            kind: kind.map(String::from),
            module: module.map(String::from),
            min_lines: Some(min_lines),
            limit: Some(limit),
        };
        async move {
            let resp = client
                .dead_code(&corpus_id, &req, session_id.as_deref())
                .await?;
            Ok(resp
                .symbols
                .into_iter()
                .map(api_dead_symbol_to_service)
                .collect())
        }
    }

    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<Vec<SolidFinding>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        let req = service_solid_params_to_api(params.clone());
        async move {
            let resp = client
                .solid(&corpus_id, &req, session_id.as_deref())
                .await?;
            Ok(resp
                .findings
                .into_iter()
                .map(api_solid_finding_to_service)
                .collect())
        }
    }

    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<Vec<RelatedClaimResult>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::RelatedRequest {
            claim_id: claim_id.to_string(),
            relation_types: relation_types
                .map(|rs| rs.iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.related(&corpus_id, &req).await?;
            Ok(resp
                .claims
                .into_iter()
                .map(api_related_to_service)
                .collect())
        }
    }

    fn compress(
        &self,
        content_ids: &[String],
    ) -> impl Future<Output = Result<Vec<CompressedItem>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::session::CompressRequest {
            content_ids: content_ids.to_vec(),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.compress(&corpus_id, &req).await?;
            Ok(resp
                .summaries
                .into_iter()
                .map(api_compressed_to_service)
                .collect())
        }
    }

    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<Vec<TocEntry>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::TocRequest {
            document_id: document_id.map(String::from),
            offset: None,
            limit: None,
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.toc(&corpus_id, &req).await?;
            Ok(resp.entries.into_iter().map(api_toc_to_service).collect())
        }
    }

    fn bridges(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> impl Future<Output = Result<Vec<BridgeLinkDetail>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::BridgeRequest {
            query: query.map(String::from),
            kind: kind.map(String::from),
            source_language: language.map(String::from),
            file_path: file_path.map(String::from),
            limit: None,
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.bridge(&corpus_id, &req).await?;
            Ok(resp.links.into_iter().map(api_bridge_to_storage).collect())
        }
    }

    fn symbol_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> impl Future<Output = Result<Option<String>, BackendError>> + Send {
        // No dedicated daemon route: reuse the existing `/occurrences`
        // endpoint (file_occurrences) and run the SAME pure covering-match
        // (`occurrence_at`) the local backend uses via the query service, so
        // resolution logic stays single-sourced across both deployment shapes.
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let file_path = file_path.to_string();
        async move {
            let occurrences = client
                .file_occurrences(&corpus_id, file_path.clone())
                .await?;
            Ok(resolve_occurrence(&file_path, occurrences, line, col))
        }
    }
}

/// Map a file's wire occurrences to the symbol id covering `(line, col)`.
///
/// The daemon backend has no dedicated `symbol_at` route — it fetches the
/// file's occurrences over the existing `/occurrences` endpoint and runs the
/// SAME pure covering-match ([`occurrence_at`]) the query service uses, so the
/// position→symbol logic is single-sourced across local and daemon modes.
fn resolve_occurrence(
    file_path: &str,
    occurrences: Vec<ministr_api::query::Occurrence>,
    line: u32,
    col: u32,
) -> Option<String> {
    let records: Vec<OccurrenceRecord> = occurrences
        .into_iter()
        .map(|o| OccurrenceRecord {
            file_path: file_path.to_string(),
            name: o.name,
            symbol_id: SymbolId(o.symbol_id),
            byte_start: o.byte_start,
            byte_end: o.byte_end,
            line: o.line,
            col: o.col,
        })
        .collect();
    occurrence_at(&records, line, col).map(|o| o.symbol_id.0.clone())
}

/// Keep only references whose kind matches `ref_kind` (no-op when `None`).
///
/// The daemon returns every reference to a symbol regardless of kind, so the
/// `ref_kind` narrowing requested through `ministr_references` is applied here.
/// [`SymbolRefResult::ref_kind`] is the snake-case string form produced by
/// [`RefKind::as_str`], so the comparison is against that.
fn retain_ref_kind(refs: &mut Vec<SymbolRefResult>, ref_kind: Option<RefKind>) {
    if let Some(kind) = ref_kind {
        refs.retain(|r| r.ref_kind == kind.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_of(kind: &str) -> SymbolRefResult {
        SymbolRefResult {
            from_symbol_id: "sym-from".into(),
            from_name: "from".into(),
            from_file: "a.rs".into(),
            from_line: 1,
            to_symbol_id: "sym-to".into(),
            to_name: "to".into(),
            to_file: "b.rs".into(),
            to_line: 2,
            ref_kind: kind.into(),
        }
    }

    #[test]
    fn retain_ref_kind_none_keeps_all() {
        let mut refs = vec![ref_of("calls"), ref_of("imports"), ref_of("implements")];
        retain_ref_kind(&mut refs, None);
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn retain_ref_kind_narrows_to_requested_kind() {
        let mut refs = vec![
            ref_of("imports"),
            ref_of("calls"),
            ref_of("implements"),
            ref_of("calls"),
            ref_of("uses"),
        ];
        retain_ref_kind(&mut refs, Some(RefKind::Calls));
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| r.ref_kind == "calls"));

        let mut implements = vec![ref_of("imports"), ref_of("implements"), ref_of("calls")];
        retain_ref_kind(&mut implements, Some(RefKind::Implements));
        assert_eq!(implements.len(), 1);
        assert_eq!(implements[0].ref_kind, "implements");
    }

    fn occ(name: &str, sym: &str, line: u32, col: u32, len: u32) -> ministr_api::query::Occurrence {
        ministr_api::query::Occurrence {
            symbol_id: sym.into(),
            name: name.into(),
            byte_start: 0,
            byte_end: len,
            line,
            col,
        }
    }

    #[test]
    fn resolve_occurrence_picks_covering_symbol() {
        // line 7: `foo` at col 4 (len 3) → covers cols 4..7.
        let occs = vec![occ("a", "sym-a", 7, 0, 1), occ("foo", "sym-foo", 7, 4, 3)];
        assert_eq!(
            resolve_occurrence("src/x.rs", occs.clone(), 7, 4).as_deref(),
            Some("sym-foo")
        );
        // mid-token still hits.
        assert_eq!(
            resolve_occurrence("src/x.rs", occs.clone(), 7, 6).as_deref(),
            Some("sym-foo")
        );
        // col 0 hits the 1-char `a`.
        assert_eq!(
            resolve_occurrence("src/x.rs", occs.clone(), 7, 0).as_deref(),
            Some("sym-a")
        );
        // whitespace gap (col 3) covers nothing.
        assert_eq!(resolve_occurrence("src/x.rs", occs.clone(), 7, 3), None);
        // wrong line.
        assert_eq!(resolve_occurrence("src/x.rs", occs, 8, 4), None);
    }

    #[test]
    fn resolve_occurrence_empty_is_none() {
        assert_eq!(resolve_occurrence("src/x.rs", Vec::new(), 1, 0), None);
    }
}
