//! Shared test infrastructure for daemon integration tests.

use std::path::PathBuf;
use std::sync::Arc;

use ministr_api::IpcAddr;
use ministr_api::client::DaemonClient;
use ministr_api::corpus::{CorpusInfo, IndexingStatus};
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::{HnswIndex, VectorIndex};
use ministr_core::ingestion::IngestionProgress;
use ministr_core::service::QueryService;
use ministr_core::session::prefetch::PrefetchEngine;
use ministr_core::session::{BudgetConfig, SessionRegistry};
use ministr_core::storage::{
    BridgeEndpointRecord, BridgeLinkRecord, SqliteStorage, Storage, SymbolRecord, SymbolRefRecord,
};
use ministr_core::types::{
    Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, RefKind, RelationType, Section,
    SectionId, SymbolId,
};
use ministr_daemon::registry::{CorpusHandle, CorpusRegistry};
use ministr_daemon::state::AppState;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Deterministic mock embedder that produces consistent vectors from text bytes.
struct MockEmbedder {
    dim: usize,
}

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.bytes().enumerate() {
                    v[i % self.dim] += f32::from(b) / 255.0;
                }
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// A running test daemon bound to a per-test IPC endpoint.
///
/// On Unix the endpoint is a UDS inside the test's tempdir. On Windows
/// it's a named pipe whose name embeds the process id and a counter so
/// parallel tests don't collide.
pub struct TestDaemon {
    pub addr: IpcAddr,
    pub corpus_id: String,
    _tmp_dir: tempfile::TempDir,
}

impl TestDaemon {
    /// Start a daemon with a pre-populated test corpus.
    pub async fn start() -> Self {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let addr = test_ipc_addr(&tmp_dir);
        let db_path = tmp_dir.path().join("content.db");

        let dim = 16;
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim });
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = Arc::new(SqliteStorage::open(&db_path).unwrap());

        populate_storage(&storage, &embedder, &index).await;

        // Build QueryService with its own connection to the same DB file.
        let query_storage = SqliteStorage::open(&db_path).unwrap();
        let service = QueryService::new(query_storage, Arc::clone(&embedder), Arc::clone(&index));

        let corpus_id = "test-corpus".to_string();
        let handle = build_corpus_handle(
            corpus_id.clone(),
            storage,
            index,
            service,
            tmp_dir.path().to_path_buf(),
        );

        let config = ministr_core::config::MinistrConfig {
            data_dir: tmp_dir.path().to_path_buf(),
            ..ministr_core::config::MinistrConfig::default()
        };
        let registry = Arc::new(CorpusRegistry::new(Arc::clone(&embedder), config));
        registry
            .corpora()
            .write()
            .await
            .insert(corpus_id.clone(), handle);

        let state = AppState::from_arc(registry);

        let listener = ministr_daemon::transport::Listener::bind(&addr).unwrap();
        tokio::spawn(async move {
            ministr_daemon::daemon::serve(state, listener)
                .await
                .unwrap();
        });

        // Wait briefly for the server to be ready.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Self {
            addr,
            corpus_id,
            _tmp_dir: tmp_dir,
        }
    }

    /// Create a new `DaemonClient` connected to this test daemon.
    pub fn client(&self) -> DaemonClient {
        DaemonClient::with_addr(self.addr.clone())
    }
}

/// Build a per-test IPC endpoint. Unix uses a UDS in the tempdir; Windows
/// uses a named pipe whose name combines pid + a process-local counter so
/// parallel tests in the same binary don't collide.
fn test_ipc_addr(tmp_dir: &tempfile::TempDir) -> IpcAddr {
    #[cfg(unix)]
    {
        IpcAddr::Unix(tmp_dir.path().join("test.sock"))
    }
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let _ = tmp_dir;
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        IpcAddr::NamedPipe(format!(
            r"\\.\pipe\ministr-test-{}-{}",
            std::process::id(),
            id
        ))
    }
}

/// Populate storage with documents, embeddings, symbols, bridges, and relationships.
async fn populate_storage(
    storage: &SqliteStorage,
    embedder: &Arc<dyn Embedder>,
    index: &Arc<dyn VectorIndex>,
) {
    let corpus = build_corpus();
    for doc in &corpus {
        storage.insert_document(doc).await.unwrap();
    }

    // Index all content at multiple resolutions.
    let texts_and_ids = [
        (
            "doc-summary::docs/auth.md",
            "Complete authentication reference.",
        ),
        (
            "doc-summary::docs/api.md",
            "Full API reference documentation.",
        ),
        (
            "sec-summary::docs/auth.md#tokens",
            "Token authentication details.",
        ),
        (
            "sec-summary::docs/auth.md#oauth",
            "OAuth 2.0 integration details.",
        ),
        (
            "sec-summary::docs/api.md#rate-limits",
            "Rate limiting policy.",
        ),
        (
            "section::docs/auth.md#tokens",
            "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
        ),
        (
            "section::docs/auth.md#oauth",
            "OAuth 2.0 authorization code flow with PKCE is required for public clients.",
        ),
        (
            "section::docs/api.md#rate-limits",
            "Rate limits are 100 requests per minute per API key. Exceeding the limit returns HTTP 429.",
        ),
        ("claim::auth-c1", "JWT tokens use RS256 signing algorithm."),
        ("claim::auth-c2", "Tokens expire after 24 hours by default."),
        (
            "claim::auth-c3",
            "OAuth 2.0 authorization code flow is supported.",
        ),
        (
            "claim::api-c1",
            "Rate limit is 100 requests per minute per API key.",
        ),
        (
            "claim::api-c2",
            "Exceeding the rate limit returns HTTP 429.",
        ),
    ];

    for (id, text) in &texts_and_ids {
        let vecs = embedder.embed(&[*text]).unwrap();
        index.insert(id, &vecs[0]).unwrap();
    }

    storage
        .insert_claim_relationships(&[
            ClaimRelationship {
                source_claim_id: ClaimId("auth-c1".into()),
                target_claim_id: ClaimId("auth-c2".into()),
                relation_type: RelationType::References,
                confidence: 0.9,
            },
            ClaimRelationship {
                source_claim_id: ClaimId("api-c1".into()),
                target_claim_id: ClaimId("api-c2".into()),
                relation_type: RelationType::DependsOn,
                confidence: 0.85,
            },
        ])
        .await
        .unwrap();

    storage.insert_symbols(&build_test_symbols()).await.unwrap();
    storage
        .insert_symbol_refs(&build_test_refs())
        .await
        .unwrap();

    let ep_ids = storage
        .insert_bridge_endpoints(&build_test_bridge_endpoints())
        .await
        .unwrap();
    storage
        .insert_bridge_links(&build_test_bridge_links(&ep_ids))
        .await
        .unwrap();
}

fn build_corpus_handle(
    corpus_id: String,
    storage: Arc<SqliteStorage>,
    index: Arc<dyn VectorIndex>,
    service: QueryService,
    data_dir: PathBuf,
) -> CorpusHandle {
    CorpusHandle {
        info: RwLock::new(CorpusInfo {
            id: corpus_id,
            paths: vec!["/test/corpus".into()],
            status: IndexingStatus::Idle,
            files_indexed: 2,
            sections_count: 3,
            embeddings_count: index.len(),
            active_sessions: 0,
            last_indexed: None,
            symbols_count: 0,
        }),
        storage,
        index,
        service,
        sessions: tokio::sync::Mutex::new(SessionRegistry::new(BudgetConfig::default())),
        prefetch: Arc::new(tokio::sync::Mutex::new(
            PrefetchEngine::with_default_capacity(),
        )),
        progress: Arc::new(IngestionProgress::new()),
        cancel: CancellationToken::new(),
        data_dir,
        coherence_tx: tokio::sync::broadcast::channel(16).0,
    }
}

fn build_corpus() -> Vec<DocumentTree> {
    vec![
        DocumentTree {
            id: ContentId("docs/auth.md".into()),
            title: "Authentication Guide".into(),
            source_path: "docs/auth.md".into(),
            sections: vec![
                Section {
                    id: SectionId("docs/auth.md#tokens".into()),
                    heading_path: vec!["Authentication".into(), "Tokens".into()],
                    depth: 2,
                    text: "JWT tokens use RS256 signing. Tokens expire after 24 hours.".into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![
                        Claim {
                            id: ClaimId("auth-c1".into()),
                            text: "JWT tokens use RS256 signing algorithm.".into(),
                            section_id: SectionId("docs/auth.md#tokens".into()),
                        },
                        Claim {
                            id: ClaimId("auth-c2".into()),
                            text: "Tokens expire after 24 hours by default.".into(),
                            section_id: SectionId("docs/auth.md#tokens".into()),
                        },
                    ],
                    summary: Some("Token authentication details.".into()),
                },
                Section {
                    id: SectionId("docs/auth.md#oauth".into()),
                    heading_path: vec!["Authentication".into(), "OAuth".into()],
                    depth: 2,
                    text: "OAuth 2.0 authorization code flow with PKCE is required for public clients.".into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![Claim {
                        id: ClaimId("auth-c3".into()),
                        text: "OAuth 2.0 authorization code flow is supported.".into(),
                        section_id: SectionId("docs/auth.md#oauth".into()),
                    }],
                    summary: Some("OAuth 2.0 integration details.".into()),
                },
            ],
            summary: Some("Complete authentication reference.".into()),
        },
        DocumentTree {
            id: ContentId("docs/api.md".into()),
            title: "API Reference".into(),
            source_path: "docs/api.md".into(),
            sections: vec![Section {
                id: SectionId("docs/api.md#rate-limits".into()),
                heading_path: vec!["API Reference".into(), "Rate Limits".into()],
                depth: 2,
                text: "Rate limits are 100 requests per minute per API key. Exceeding the limit returns HTTP 429.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![
                    Claim {
                        id: ClaimId("api-c1".into()),
                        text: "Rate limit is 100 requests per minute per API key.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    },
                    Claim {
                        id: ClaimId("api-c2".into()),
                        text: "Exceeding the rate limit returns HTTP 429.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    },
                ],
                summary: Some("Rate limiting policy.".into()),
            }],
            summary: Some("Full API reference documentation.".into()),
        },
    ]
}

fn build_test_symbols() -> Vec<SymbolRecord> {
    vec![
        SymbolRecord {
            id: SymbolId("sym-config::MinistrConfig".into()),
            file_path: "src/config.rs".into(),
            name: "MinistrConfig".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct MinistrConfig".into(),
            doc_comment: Some("Configuration for the ministr context cache.".into()),
            module_path: "config".into(),
            line_start: 10,
            line_end: 25,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-service::QueryService".into()),
            file_path: "src/service.rs".into(),
            name: "QueryService".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct QueryService".into(),
            doc_comment: Some("High-level query service composing storage and index.".into()),
            module_path: "service".into(),
            line_start: 50,
            line_end: 55,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-service::survey".into()),
            file_path: "src/service.rs".into(),
            name: "survey".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub async fn survey(&self, query: &str, top_k: usize) -> Result<Vec<SurveyResult>, QueryError>".into(),
            doc_comment: Some("Search the corpus for content relevant to a query.".into()),
            module_path: "service".into(),
            line_start: 60,
            line_end: 80,
            cyclomatic_complexity: None,
        },
    ]
}

fn build_test_refs() -> Vec<SymbolRefRecord> {
    vec![SymbolRefRecord {
        from_symbol_id: SymbolId("sym-service::survey".into()),
        to_symbol_id: SymbolId("sym-config::MinistrConfig".into()),
        ref_kind: RefKind::Uses,
    }]
}

fn build_test_bridge_endpoints() -> Vec<BridgeEndpointRecord> {
    vec![
        BridgeEndpointRecord {
            id: None,
            file_path: "src/auth.ts".into(),
            binding_key: "auth.parseToken".into(),
            kind: "tauri_command".into(),
            role: "export".into(),
            language: "typescript".into(),
            line: 15,
            symbol_name: "parseToken".into(),
            confidence: 0.95,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "src/auth/token.rs".into(),
            binding_key: "auth.parseToken".into(),
            kind: "tauri_command".into(),
            role: "import".into(),
            language: "rust".into(),
            line: 42,
            symbol_name: "parse_token".into(),
            confidence: 0.90,
        },
    ]
}

fn build_test_bridge_links(ep_ids: &[i64]) -> Vec<BridgeLinkRecord> {
    if ep_ids.len() < 2 {
        return vec![];
    }
    vec![BridgeLinkRecord {
        export_ep_id: ep_ids[0],
        import_ep_id: ep_ids[1],
        kind: "tauri_command".into(),
        confidence: 0.90,
    }]
}
