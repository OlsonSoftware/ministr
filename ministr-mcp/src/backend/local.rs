//! [`LocalBackend`] — in-process implementation of [`QueryBackend`].
//!
//! Calls directly into a shared [`QueryService`]. Use when ministr owns
//! the index for this process (no daemon to forward to).

use std::future::Future;
use std::sync::Arc;

use ministr_core::service::{
    ClaimResult, CompressedItem, DeadSymbol, ImpactResult, QueryService, RelatedClaimResult,
    SectionDetail, SolidFinding, SolidParams, SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{RefKind, RelationType, TocEntry};

use super::{BackendError, QueryBackend};

/// Backend that runs every operation in-process against a [`QueryService`].
pub struct LocalBackend {
    service: Arc<QueryService>,
}

impl LocalBackend {
    #[must_use]
    pub fn new(service: Arc<QueryService>) -> Self {
        Self { service }
    }

    #[must_use]
    pub fn service(&self) -> &Arc<QueryService> {
        &self.service
    }
}

impl QueryBackend for LocalBackend {
    fn survey(
        &self,
        query: &str,
        top_k: usize,
    ) -> impl Future<Output = Result<Vec<SurveyResult>, BackendError>> + Send {
        let service = self.service.clone();
        let query = query.to_string();
        async move { Ok(service.survey(&query, top_k).await?) }
    }

    fn survey_with_exclude(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &std::collections::HashSet<String>,
    ) -> impl Future<Output = Result<(Vec<SurveyResult>, usize), BackendError>> + Send {
        let service = self.service.clone();
        let query = query.to_string();
        let exclude_ids = exclude_ids.clone();
        async move {
            Ok(service
                .survey_excluding(&query, top_k, &exclude_ids)
                .await?)
        }
    }

    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<SectionDetail, BackendError>> + Send {
        let service = self.service.clone();
        let section_id = section_id.to_string();
        async move { Ok(service.read_section(&section_id).await?) }
    }

    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ClaimResult>, BackendError>> + Send {
        let service = self.service.clone();
        let section_id = section_id.to_string();
        let query = query.map(String::from);
        async move {
            Ok(service
                .extract_claims(&section_id, query.as_deref())
                .await?)
        }
    }

    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<Vec<SymbolRecord>, BackendError>> + Send {
        let service = self.service.clone();
        async move { Ok(service.search_symbols(&filter).await?) }
    }

    fn definition(
        &self,
        symbol_id: &str,
    ) -> impl Future<Output = Result<SymbolDefinition, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move { Ok(service.get_symbol_definition(&symbol_id).await?) }
    }

    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> impl Future<Output = Result<Vec<SymbolRefResult>, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move { Ok(service.get_symbol_references(&symbol_id, ref_kind).await?) }
    }

    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
    ) -> impl Future<Output = Result<ImpactResult, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move { Ok(service.compute_impact(&symbol_id, max_depth).await?) }
    }

    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<DeadSymbol>, BackendError>> + Send {
        let service = self.service.clone();
        let kind = kind.map(String::from);
        let module = module.map(String::from);
        async move {
            Ok(service
                .find_dead_code(kind.as_deref(), module.as_deref(), min_lines, limit)
                .await?)
        }
    }

    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<Vec<SolidFinding>, BackendError>> + Send {
        let service = self.service.clone();
        let params = params.clone();
        async move { Ok(service.detect_solid_violations(&params).await?) }
    }

    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<Vec<RelatedClaimResult>, BackendError>> + Send {
        let service = self.service.clone();
        let claim_id = claim_id.to_string();
        let relation_types = relation_types.map(<[RelationType]>::to_vec);
        async move {
            Ok(service
                .related_claims(&claim_id, relation_types.as_deref())
                .await?)
        }
    }

    fn compress(
        &self,
        content_ids: &[String],
    ) -> impl Future<Output = Result<Vec<CompressedItem>, BackendError>> + Send {
        let service = self.service.clone();
        let content_ids = content_ids.to_vec();
        // Extractive (TF-IDF) — fast, no extra cost, no MCP sampling needed.
        // Matches the algorithm the daemon uses for its `/compress` endpoint.
        async move { Ok(service.compress_content(&content_ids).await?) }
    }

    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<Vec<TocEntry>, BackendError>> + Send {
        let service = self.service.clone();
        let document_id = document_id.map(String::from);
        async move { Ok(service.toc(document_id.as_deref()).await?) }
    }

    fn bridges(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> impl Future<Output = Result<Vec<BridgeLinkDetail>, BackendError>> + Send {
        let service = self.service.clone();
        let query = query.map(String::from);
        let kind = kind.map(String::from);
        let language = language.map(String::from);
        let file_path = file_path.map(String::from);
        async move {
            Ok(service
                .query_bridges(
                    query.as_deref(),
                    kind.as_deref(),
                    language.as_deref(),
                    file_path.as_deref(),
                )
                .await?)
        }
    }

    fn symbol_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> impl Future<Output = Result<Option<String>, BackendError>> + Send {
        let service = self.service.clone();
        let file_path = file_path.to_string();
        async move { Ok(service.symbol_at_position(&file_path, line, col).await?) }
    }
}
