//! [`DaemonBackend`] — HTTP-forwarding implementation of [`QueryBackend`].
//!
//! Every call becomes a JSON-RPC request to a running `ministr-daemon` via
//! the shared [`DaemonClient`]. Use when ministr is running as a stdio MCP
//! proxy that delegates the index to a separate daemon process.

use std::future::Future;
use std::sync::Arc;

use ministr_api::client::DaemonClient;
use ministr_core::service::{
    CallDirection, ClaimResult, DeadSymbol, DefinitionOptions, Diagnostic, ImpactResult,
    InspectOptions, InspectResult, RelatedClaimResult, SectionDetail, SolidFinding, SolidParams,
    SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::traits::{OccurrenceRecord, occurrence_at};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{DeliveryIdentity, RefKind, RelationType, SymbolId, TocEntry};

use super::convert::{
    api_bridge_to_storage, api_claim_to_service, api_compressed_delivery_to_service,
    api_dead_symbol_to_service, api_diagnostic_to_service, api_impact_to_service,
    api_inspect_to_service, api_related_to_service, api_section_to_service,
    api_solid_finding_to_service, api_survey_to_service, api_symbol_def_to_record,
    api_symbol_def_to_service, api_symbol_reference_to_service, api_toc_to_service,
    service_solid_params_to_api,
};
use super::{
    BackendError, BackendResponse, CollectionBackendResponse, CompressedDelivery,
    ImpactBackendResponse, QueryBackend, ReferencesBackendResponse, SurveyBackendResponse,
    TocBackendResponse,
};

/// Backend that forwards every operation to a running `ministr-daemon`.
pub struct DaemonBackend {
    client: Arc<DaemonClient>,
    corpus_id: String,
    session_id: Option<String>,
}

#[allow(clippy::missing_errors_doc)] // every forwarding method returns the same BackendError transport contract
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

    /// Remove daemon-owned delivery state so an exact result can be sent again.
    pub async fn drop_deliveries(
        &self,
        identities: &[DeliveryIdentity],
        content_ids: &[String],
    ) -> Result<(), BackendError> {
        let Some(session_id) = self.session_id.as_deref() else {
            return Ok(());
        };
        if identities.is_empty() && content_ids.is_empty() {
            return Ok(());
        }
        let request = ministr_api::session::DropRequest {
            content_ids: content_ids.to_vec(),
            identities: identities
                .iter()
                .map(|identity| ministr_api::metadata::DeliveryIdentity {
                    corpus_id: identity.corpus_id.clone(),
                    content_id: identity.content_id.clone(),
                    resolution: identity.resolution.clone(),
                })
                .collect(),
        };
        self.client
            .drop_content(&self.corpus_id, session_id, &request)
            .await?;
        Ok(())
    }

    /// Fetch one daemon reference page with its true total and cursor.
    pub async fn references_page(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ReferencesBackendResponse, BackendError> {
        let request = ministr_api::query::ReferencesRequest {
            session_id: self.session_id.clone(),
            ref_kind: ref_kind.map(|kind| kind.as_str().to_string()),
            through_implementors: Some(through_implementors),
            offset,
            limit: Some(limit),
            cursor: cursor.map(str::to_string),
        };
        let response = self
            .client
            .references_req(&self.corpus_id, symbol_id, &request)
            .await?;
        Ok(ReferencesBackendResponse {
            references: response
                .references
                .into_iter()
                .map(api_symbol_reference_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn search_symbols_page(
        &self,
        filter: SymbolFilter,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<SymbolRecord>, BackendError> {
        let request = ministr_api::query::SymbolsRequest {
            query: filter.name.unwrap_or_default(),
            kind: filter.kind,
            module: filter.module,
            visibility: filter.visibility,
            file_path: filter.file_path,
            limit: Some(limit),
            offset: Some(offset),
            session_id: self.session_id.clone(),
        };
        let response = self.client.symbols(&self.corpus_id, &request).await?;
        Ok(CollectionBackendResponse {
            data: response
                .symbols
                .into_iter()
                .map(api_symbol_def_to_record)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn impact_page(
        &self,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ImpactBackendResponse, BackendError> {
        let response = self
            .client
            .impact_page(
                &self.corpus_id,
                symbol_id,
                Some(max_depth),
                Some(direction.as_str()),
                tests_only,
                self.session_id.as_deref(),
                offset,
                cursor,
                Some(limit),
            )
            .await?;
        let pagination = response.pagination.clone();
        let metadata = response.metadata.clone();
        Ok(ImpactBackendResponse {
            impact: api_impact_to_service(response),
            pagination,
            metadata,
        })
    }

    pub async fn dead_code_page(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<DeadSymbol>, BackendError> {
        let request = ministr_api::query::DeadCodeRequest {
            kind: kind.map(str::to_string),
            module: module.map(str::to_string),
            min_lines: Some(min_lines),
            limit: Some(limit),
            offset: Some(offset),
        };
        let response = self
            .client
            .dead_code(&self.corpus_id, &request, self.session_id.as_deref())
            .await?;
        Ok(CollectionBackendResponse {
            data: response
                .symbols
                .into_iter()
                .map(api_dead_symbol_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn diagnostics_page(
        &self,
        languages: Option<&[String]>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<Diagnostic>, BackendError> {
        let request = ministr_api::query::DiagnosticsRequest {
            languages: languages.map(<[String]>::to_vec),
            limit: Some(limit),
            offset: Some(offset),
        };
        let response = self
            .client
            .diagnostics(&self.corpus_id, &request, self.session_id.as_deref())
            .await?;
        Ok(CollectionBackendResponse {
            data: response
                .diagnostics
                .into_iter()
                .map(api_diagnostic_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn solid_page(
        &self,
        params: &SolidParams,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<SolidFinding>, BackendError> {
        let mut request = service_solid_params_to_api(params.clone());
        request.limit = Some(limit);
        request.offset = Some(offset);
        let response = self
            .client
            .solid(&self.corpus_id, &request, self.session_id.as_deref())
            .await?;
        Ok(CollectionBackendResponse {
            data: response
                .findings
                .into_iter()
                .map(api_solid_finding_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn related_claims_page(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<CollectionBackendResponse<RelatedClaimResult>, BackendError> {
        let request = ministr_api::query::RelatedRequest {
            claim_id: claim_id.to_string(),
            relation_types: relation_types
                .map(|relations| relations.iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            session_id: self.session_id.clone(),
            offset,
            cursor: cursor.map(str::to_string),
            limit: Some(limit),
        };
        let response = self.client.related(&self.corpus_id, &request).await?;
        Ok(CollectionBackendResponse {
            data: response
                .claims
                .into_iter()
                .map(api_related_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn toc_page(
        &self,
        document_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<TocBackendResponse, BackendError> {
        let request = ministr_api::query::TocRequest {
            document_id: document_id.map(str::to_string),
            offset: Some(offset),
            limit: Some(limit),
            session_id: self.session_id.clone(),
        };
        let response = self.client.toc(&self.corpus_id, &request).await?;
        Ok(TocBackendResponse {
            entries: response
                .entries
                .into_iter()
                .map(api_toc_to_service)
                .collect(),
            documents: response.documents,
            claims: response.claims,
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn bridges_page(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<BridgeLinkDetail>, BackendError> {
        let request = ministr_api::query::BridgeRequest {
            query: query.map(str::to_string),
            kind: kind.map(str::to_string),
            source_language: language.map(str::to_string),
            file_path: file_path.map(str::to_string),
            limit: Some(limit),
            offset: Some(offset),
            session_id: self.session_id.clone(),
        };
        let response = self.client.bridge(&self.corpus_id, &request).await?;
        Ok(CollectionBackendResponse {
            data: response
                .links
                .into_iter()
                .map(api_bridge_to_storage)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    pub async fn extract_claims_page(
        &self,
        section_id: &str,
        query: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<ClaimResult>, BackendError> {
        let request = ministr_api::query::ExtractRequest {
            section_id: section_id.to_string(),
            query: query.map(str::to_string),
            session_id: self.session_id.clone(),
            offset: Some(offset),
            cursor: None,
            limit: Some(limit),
        };
        let response = self.client.extract(&self.corpus_id, &request).await?;
        Ok(CollectionBackendResponse {
            data: response
                .claims
                .into_iter()
                .map(api_claim_to_service)
                .collect(),
            pagination: response.pagination,
            metadata: response.metadata,
        })
    }

    /// Fetch a ranked survey window without daemon-owned session exclusions.
    /// Continuation paging applies the offset to this stable window before the
    /// MCP layer filters any exact identities already in context.
    pub async fn survey_window(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<SurveyBackendResponse, BackendError> {
        let request = ministr_api::query::SurveyRequest {
            query: query.to_string(),
            top_k: Some(top_k),
            limit: None,
            offset: None,
            cursor: None,
            session_id: None,
            exclude: Vec::new(),
            max_result_bytes: None,
            max_result_tokens: None,
            max_total_bytes: Some(1_048_576),
            max_total_tokens: Some(262_144),
        };
        let response = self.client.survey_req(&self.corpus_id, &request).await?;
        Ok(SurveyBackendResponse {
            results: response
                .results
                .into_iter()
                .map(api_survey_to_service)
                .collect(),
            deduplicated_count: response.deduplicated_count.unwrap_or(0),
            suppressed_identities: response
                .suppressed_identities
                .into_iter()
                .map(|identity| {
                    DeliveryIdentity::new(
                        identity.corpus_id,
                        identity.content_id,
                        identity.resolution,
                    )
                })
                .collect(),
            metadata: response.metadata,
        })
    }

    fn inspect_request(
        &self,
        symbol_id: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        async move {
            let req = ministr_api::query::InspectRequest {
                symbol_id,
                file,
                line,
                col,
                include: options
                    .include
                    .into_iter()
                    .map(|include| match include {
                        ministr_core::service::InspectInclude::Definition => {
                            ministr_api::query::InspectInclude::Definition
                        }
                        ministr_core::service::InspectInclude::Callers => {
                            ministr_api::query::InspectInclude::Callers
                        }
                        ministr_core::service::InspectInclude::Callees => {
                            ministr_api::query::InspectInclude::Callees
                        }
                        ministr_core::service::InspectInclude::Implementors => {
                            ministr_api::query::InspectInclude::Implementors
                        }
                        ministr_core::service::InspectInclude::Imports => {
                            ministr_api::query::InspectInclude::Imports
                        }
                        ministr_core::service::InspectInclude::Tests => {
                            ministr_api::query::InspectInclude::Tests
                        }
                        ministr_core::service::InspectInclude::Bridges => {
                            ministr_api::query::InspectInclude::Bridges
                        }
                    })
                    .collect(),
                max_per_group: Some(options.max_per_group),
                max_source_lines: Some(options.max_source_lines),
                session_id,
            };
            let response = client.inspect(&corpus_id, &req).await?;
            let metadata = response.metadata.clone();
            Ok(BackendResponse {
                data: api_inspect_to_service(response),
                metadata,
            })
        }
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
            limit: None,
            offset: None,
            cursor: None,
            session_id: self.session_id.clone(),
            exclude: Vec::new(),
            max_result_bytes: None,
            max_result_tokens: None,
            max_total_bytes: Some(1_048_576),
            max_total_tokens: Some(262_144),
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
        exclude_ids: &std::collections::HashSet<DeliveryIdentity>,
    ) -> impl Future<Output = Result<SurveyBackendResponse, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::SurveyRequest {
            query: query.to_string(),
            top_k: Some(top_k),
            limit: None,
            offset: None,
            cursor: None,
            // The MCP proxy is the delivery/dedup authority and forwards its
            // exact exclusion set. Direct daemon API sessions remain
            // daemon-owned, but proxy requests must not layer a second shadow.
            session_id: None,
            exclude: exclude_ids
                .iter()
                .map(|identity| ministr_api::metadata::DeliveryIdentity {
                    corpus_id: identity.corpus_id.clone(),
                    content_id: identity.content_id.clone(),
                    resolution: identity.resolution.clone(),
                })
                .collect(),
            max_result_bytes: None,
            max_result_tokens: None,
            max_total_bytes: Some(1_048_576),
            max_total_tokens: Some(262_144),
        };
        async move {
            let resp = client.survey_req(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            let results: Vec<SurveyResult> = resp
                .results
                .into_iter()
                .map(api_survey_to_service)
                .collect();
            let deduplicated = resp.deduplicated_count.unwrap_or(0);
            let suppressed_identities = resp
                .suppressed_identities
                .into_iter()
                .map(|identity| {
                    DeliveryIdentity::new(
                        identity.corpus_id,
                        identity.content_id,
                        identity.resolution,
                    )
                })
                .collect();
            Ok(SurveyBackendResponse {
                results,
                deduplicated_count: deduplicated,
                suppressed_identities,
                metadata,
            })
        }
    }

    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<BackendResponse<SectionDetail>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let section_id = section_id.to_string();
        async move {
            let resp = client.read_section(&corpus_id, &section_id).await?;
            let metadata = resp.metadata.clone();
            Ok(BackendResponse {
                data: api_section_to_service(resp),
                metadata,
            })
        }
    }

    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<ClaimResult>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::ExtractRequest {
            section_id: section_id.to_string(),
            query: query.map(String::from),
            session_id: self.session_id.clone(),
            offset: None,
            cursor: None,
            limit: Some(500),
        };
        async move {
            let resp = client.extract(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp.claims.into_iter().map(api_claim_to_service).collect(),
                metadata,
            })
        }
    }

    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRecord>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        async move {
            let req = ministr_api::query::SymbolsRequest {
                query: filter.name.clone().unwrap_or_default(),
                kind: filter.kind.clone(),
                module: filter.module.clone(),
                visibility: filter.visibility.clone(),
                // The daemon honors a file_path filter, so the daemon-backend
                // forwards it instead of silently dropping it.
                file_path: filter.file_path.clone(),
                limit: Some(500),
                offset: None,
                session_id,
            };
            let resp = client.symbols(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp
                    .symbols
                    .into_iter()
                    .map(api_symbol_def_to_record)
                    .collect(),
                metadata,
            })
        }
    }

    fn definition(
        &self,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> impl Future<Output = Result<BackendResponse<SymbolDefinition>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .definition_response_req(
                    &corpus_id,
                    &symbol_id,
                    &ministr_api::query::DefinitionRequest {
                        session_id,
                        max_lines: Some(options.max_lines),
                        context_lines: Some(options.context_lines),
                        include_body: Some(options.include_body),
                        outline_only: Some(options.outline_only),
                        start_line: options.start_line,
                        start_byte: options.start_byte,
                    },
                )
                .await?;
            Ok(BackendResponse {
                data: api_symbol_def_to_service(resp.definition),
                metadata: resp.metadata,
            })
        }
    }

    fn inspect_symbol(
        &self,
        symbol_id: &str,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send {
        self.inspect_request(Some(symbol_id.to_string()), None, None, None, options)
    }

    fn inspect_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send {
        self.inspect_request(
            None,
            Some(file_path.to_string()),
            Some(line),
            Some(col),
            options,
        )
    }

    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRefResult>>, BackendError>> + Send
    {
        // The daemon HTTP route doesn't accept a ref_kind filter, so apply it
        // client-side here for parity with `LocalBackend`. `through_implementors`
        // DOES go server-side (a query param): the type-hierarchy hop needs the
        // daemon's graph, and its peer-caller (Calls) refs survive the
        // client-side `ref_kind` retain below.
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let symbol_id = symbol_id.to_string();
        let session_id = self.session_id.clone();
        async move {
            let resp = client
                .references(
                    &corpus_id,
                    &symbol_id,
                    session_id.as_deref(),
                    through_implementors,
                )
                .await?;
            let metadata = resp.metadata;
            let mut refs: Vec<SymbolRefResult> = resp
                .references
                .into_iter()
                .map(api_symbol_reference_to_service)
                .collect();
            retain_ref_kind(&mut refs, ref_kind);
            Ok(BackendResponse {
                data: refs,
                metadata,
            })
        }
    }

    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> impl Future<Output = Result<BackendResponse<ImpactResult>, BackendError>> + Send {
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
                    Some(direction.as_str()),
                    tests_only,
                    session_id.as_deref(),
                )
                .await?;
            let metadata = resp.metadata.clone();
            Ok(BackendResponse {
                data: api_impact_to_service(resp),
                metadata,
            })
        }
    }

    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<DeadSymbol>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        let req = ministr_api::query::DeadCodeRequest {
            kind: kind.map(String::from),
            module: module.map(String::from),
            min_lines: Some(min_lines),
            limit: Some(limit),
            offset: None,
        };
        async move {
            let resp = client
                .dead_code(&corpus_id, &req, session_id.as_deref())
                .await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp
                    .symbols
                    .into_iter()
                    .map(api_dead_symbol_to_service)
                    .collect(),
                metadata,
            })
        }
    }

    fn diagnostics(
        &self,
        languages: Option<&[String]>,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<Diagnostic>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        let req = ministr_api::query::DiagnosticsRequest {
            languages: languages.map(<[String]>::to_vec),
            limit: Some(limit),
            offset: None,
        };
        async move {
            let resp = client
                .diagnostics(&corpus_id, &req, session_id.as_deref())
                .await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp
                    .diagnostics
                    .into_iter()
                    .map(api_diagnostic_to_service)
                    .collect(),
                metadata,
            })
        }
    }

    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SolidFinding>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let session_id = self.session_id.clone();
        let req = service_solid_params_to_api(params.clone());
        async move {
            let resp = client
                .solid(&corpus_id, &req, session_id.as_deref())
                .await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp
                    .findings
                    .into_iter()
                    .map(api_solid_finding_to_service)
                    .collect(),
                metadata,
            })
        }
    }

    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<RelatedClaimResult>>, BackendError>> + Send
    {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::RelatedRequest {
            claim_id: claim_id.to_string(),
            relation_types: relation_types
                .map(|rs| rs.iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            session_id: self.session_id.clone(),
            offset: None,
            cursor: None,
            limit: Some(500),
        };
        async move {
            let resp = client.related(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp
                    .claims
                    .into_iter()
                    .map(api_related_to_service)
                    .collect(),
                metadata,
            })
        }
    }

    fn compress(
        &self,
        identities: &[DeliveryIdentity],
    ) -> impl Future<Output = Result<Vec<CompressedDelivery>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::session::CompressRequest {
            content_ids: Vec::new(),
            identities: identities
                .iter()
                .map(|identity| ministr_api::metadata::DeliveryIdentity {
                    corpus_id: identity.corpus_id.clone(),
                    content_id: identity.content_id.clone(),
                    resolution: identity.resolution.clone(),
                })
                .collect(),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.compress(&corpus_id, &req).await?;
            Ok(resp
                .summaries
                .into_iter()
                .map(|item| api_compressed_delivery_to_service(item, &corpus_id))
                .collect())
        }
    }

    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<TocEntry>>, BackendError>> + Send {
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::TocRequest {
            document_id: document_id.map(String::from),
            offset: None,
            limit: Some(500),
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.toc(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp.entries.into_iter().map(api_toc_to_service).collect(),
                metadata,
            })
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
        let client = self.client.clone();
        let corpus_id = self.corpus_id.clone();
        let req = ministr_api::query::BridgeRequest {
            query: query.map(String::from),
            kind: kind.map(String::from),
            source_language: language.map(String::from),
            file_path: file_path.map(String::from),
            limit: Some(500),
            offset: None,
            session_id: self.session_id.clone(),
        };
        async move {
            let resp = client.bridge(&corpus_id, &req).await?;
            let metadata = resp.metadata;
            Ok(BackendResponse {
                data: resp.links.into_iter().map(api_bridge_to_storage).collect(),
                metadata,
            })
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
