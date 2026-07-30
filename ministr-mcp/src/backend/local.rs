//! [`LocalBackend`] — in-process implementation of [`QueryBackend`].
//!
//! Calls directly into a shared [`QueryService`]. Use when ministr owns
//! the index for this process (no daemon to forward to).

use std::future::Future;
use std::sync::Arc;

use ministr_core::service::{
    CallDirection, ClaimResult, DeadSymbol, DefinitionOptions, Diagnostic, ImpactResult,
    InspectOptions, InspectResult, QueryService, RelatedClaimResult, SectionDetail, SolidFinding,
    SolidParams, SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{DeliveryIdentity, RefKind, RelationType, TocEntry};

use super::{
    BackendError, BackendResponse, CompressedDelivery, QueryBackend, SurveyBackendResponse,
};

/// Backend that runs every operation in-process against a [`QueryService`].
pub struct LocalBackend {
    service: Arc<QueryService>,
    primary_label: super::PrimaryLabel,
}

impl LocalBackend {
    #[must_use]
    pub fn new(service: Arc<QueryService>) -> Self {
        Self {
            service,
            primary_label: super::PrimaryLabel::default(),
        }
    }

    #[must_use]
    pub fn service(&self) -> &Arc<QueryService> {
        &self.service
    }

    /// The current project's label, so a `project` argument naming this
    /// project routes here instead of failing as an unknown corpus.
    #[must_use]
    pub fn primary_label(&self) -> &super::PrimaryLabel {
        &self.primary_label
    }

    /// Corpus local directory roots (path + id) for diff-impact key
    /// reconstruction (ingest-key-locator-decouple).
    pub(crate) async fn local_dir_roots(&self) -> Vec<(std::path::PathBuf, String)> {
        self.service.local_dir_roots().await
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
        exclude_ids: &std::collections::HashSet<DeliveryIdentity>,
    ) -> impl Future<Output = Result<SurveyBackendResponse, BackendError>> + Send {
        let service = self.service.clone();
        let query = query.to_string();
        let exclude_ids = exclude_ids.clone();
        async move {
            let (results, suppressed_identities) = service
                .survey_excluding_identities_detailed_with_options(
                    &query,
                    top_k,
                    "primary",
                    &exclude_ids,
                    super::survey_candidate_options(),
                )
                .await?;
            let deduplicated_count = suppressed_identities.len();
            Ok(SurveyBackendResponse {
                results,
                deduplicated_count,
                suppressed_identities,
                metadata: ministr_api::metadata::QueryMetadata::default(),
            })
        }
    }

    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<BackendResponse<SectionDetail>, BackendError>> + Send {
        let service = self.service.clone();
        let section_id = section_id.to_string();
        async move {
            Ok(BackendResponse::complete(
                service.read_section(&section_id).await?,
            ))
        }
    }

    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<ClaimResult>>, BackendError>> + Send {
        let service = self.service.clone();
        let section_id = section_id.to_string();
        let query = query.map(String::from);
        async move {
            Ok(BackendResponse::complete(
                service
                    .extract_claims(&section_id, query.as_deref())
                    .await?,
            ))
        }
    }

    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRecord>>, BackendError>> + Send {
        let service = self.service.clone();
        async move {
            Ok(BackendResponse::complete(
                service.search_symbols(&filter).await?,
            ))
        }
    }

    fn definition(
        &self,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> impl Future<Output = Result<BackendResponse<SymbolDefinition>, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move {
            let definition = service
                .get_symbol_definition_with_options(&symbol_id, options)
                .await?;
            let mut response = BackendResponse::complete(definition);
            if let Some(code) = response.data.source_error.clone() {
                response.metadata.status = ministr_api::metadata::ResponseStatus::Partial;
                response.metadata.completeness.completeness =
                    ministr_api::metadata::CompletenessState::Partial;
                response.metadata.completeness.absence_is_conclusive = false;
                response
                    .metadata
                    .completeness
                    .affected_capabilities
                    .push("definition".to_string());
                response.metadata.error = Some(ministr_api::metadata::ResponseError {
                    error_code: code.clone(),
                    retryable: code != "permission_denied",
                    message: "Indexed symbol metadata is available, but source could not be read."
                        .to_string(),
                    corpus_id: Some(ministr_core::types::PRIMARY_CORPUS_ID.to_string()),
                    backend: Some("local".to_string()),
                });
            }
            Ok(response)
        }
    }

    fn inspect_symbol(
        &self,
        symbol_id: &str,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move {
            Ok(BackendResponse::complete(
                service.inspect_symbol(&symbol_id, &options).await?,
            ))
        }
    }

    fn inspect_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send {
        let service = self.service.clone();
        let file_path = file_path.to_string();
        async move {
            Ok(BackendResponse::complete(
                service
                    .inspect_at_position(&file_path, line, col, &options)
                    .await?,
            ))
        }
    }

    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRefResult>>, BackendError>> + Send
    {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move {
            if through_implementors {
                Ok(BackendResponse::complete(
                    service
                        .get_symbol_references_through_implementors(&symbol_id, ref_kind, 500)
                        .await?,
                ))
            } else {
                Ok(BackendResponse::complete(
                    service.get_symbol_references(&symbol_id, ref_kind).await?,
                ))
            }
        }
    }

    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> impl Future<Output = Result<BackendResponse<ImpactResult>, BackendError>> + Send {
        let service = self.service.clone();
        let symbol_id = symbol_id.to_string();
        async move {
            Ok(BackendResponse::complete(
                service
                    .compute_impact(&symbol_id, max_depth, direction, tests_only)
                    .await?,
            ))
        }
    }

    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<DeadSymbol>>, BackendError>> + Send {
        let service = self.service.clone();
        let kind = kind.map(String::from);
        let module = module.map(String::from);
        async move {
            Ok(BackendResponse::complete(
                service
                    .find_dead_code(kind.as_deref(), module.as_deref(), min_lines, limit)
                    .await?,
            ))
        }
    }

    fn diagnostics(
        &self,
        languages: Option<&[String]>,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<Diagnostic>>, BackendError>> + Send {
        let service = self.service.clone();
        let languages = languages.map(<[String]>::to_vec);
        async move {
            Ok(BackendResponse::complete(
                service.diagnostics(languages.as_deref(), limit).await?,
            ))
        }
    }

    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SolidFinding>>, BackendError>> + Send {
        let service = self.service.clone();
        let params = params.clone();
        async move {
            Ok(BackendResponse::complete(
                service.detect_solid_violations(&params).await?,
            ))
        }
    }

    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<RelatedClaimResult>>, BackendError>> + Send
    {
        let service = self.service.clone();
        let claim_id = claim_id.to_string();
        let relation_types = relation_types.map(<[RelationType]>::to_vec);
        async move {
            Ok(BackendResponse::complete(
                service
                    .related_claims(&claim_id, relation_types.as_deref())
                    .await?,
            ))
        }
    }

    fn compress(
        &self,
        identities: &[DeliveryIdentity],
    ) -> impl Future<Output = Result<Vec<CompressedDelivery>, BackendError>> + Send {
        let service = self.service.clone();
        let identities = identities.to_vec();
        // Extractive (TF-IDF) — fast, no extra cost, no MCP sampling needed.
        // Matches the algorithm the daemon uses for its `/compress` endpoint.
        async move {
            let mut compressed = Vec::new();
            for identity in identities {
                let Some(item) = service
                    .compress_content(std::slice::from_ref(&identity.content_id))
                    .await?
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                compressed.push(CompressedDelivery { identity, item });
            }
            Ok(compressed)
        }
    }

    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<TocEntry>>, BackendError>> + Send {
        let service = self.service.clone();
        let document_id = document_id.map(String::from);
        async move {
            Ok(BackendResponse::complete(
                service.toc(document_id.as_deref()).await?,
            ))
        }
    }

    fn bridges(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<BridgeLinkDetail>>, BackendError>> + Send
    {
        let service = self.service.clone();
        let query = query.map(String::from);
        let kind = kind.map(String::from);
        let language = language.map(String::from);
        let file_path = file_path.map(String::from);
        async move {
            Ok(BackendResponse::complete(
                service
                    .query_bridges(
                        query.as_deref(),
                        kind.as_deref(),
                        language.as_deref(),
                        file_path.as_deref(),
                    )
                    .await?,
            ))
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
