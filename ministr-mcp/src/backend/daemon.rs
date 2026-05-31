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
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{RefKind, RelationType, TocEntry};

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
        _ref_kind: Option<RefKind>,
    ) -> impl Future<Output = Result<Vec<SymbolRefResult>, BackendError>> + Send {
        // The daemon HTTP route doesn't accept a ref_kind filter today —
        // forward unfiltered and rely on the caller. Parity follow-up.
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .references(&corpus_id, &symbol_id, session_id.as_deref())
                .await?;
            Ok(resp
                .references
                .into_iter()
                .map(api_symbol_reference_to_service)
                .collect())
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
}
