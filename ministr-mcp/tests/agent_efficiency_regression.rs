//! Deterministic task-level agent-efficiency regression gate.
//!
//! Retrieval rank alone does not tell us whether an agent reached the right
//! implementation without wasting calls and context. This suite drives the
//! real MCP transport for navigation workflows, counts the literal serialized
//! `CallToolResult`, and combines those observations with committed protocol
//! traces for multi-corpus completeness/failure states.

#![allow(
    clippy::cast_precision_loss,
    reason = "the gate intentionally reports aggregate ratios over small fixture counts"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::{ExactScanIndex, VectorIndex};
use ministr_core::service::QueryService;
use ministr_core::storage::{
    BridgeEndpointRecord, BridgeLinkRecord, SqliteStorage, Storage, SymbolRecord, SymbolRefRecord,
};
use ministr_core::token::count_tokens;
use ministr_core::types::{ContentId, DocumentTree, RefKind, Section, SectionId, SymbolId};
use ministr_mcp::server::MinistrServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
use serde::Deserialize;
use serde_json::{Value, json};

#[path = "../../ministr-daemon/tests/common/mod.rs"]
#[allow(
    dead_code,
    reason = "shared daemon fixture exposes helpers used by sibling test binaries"
)]
mod daemon_fixture;

const FIXTURE_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../eval/agent-efficiency");
const DIMENSION: usize = 128;

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServerHandle = rmcp::service::RunningService<rmcp::RoleServer, MinistrServer>;

#[derive(Debug, Deserialize)]
struct EvaluationSpec {
    version: u32,
    gate: Gate,
    tasks: Vec<TaskSpec>,
}

#[derive(Debug, Deserialize)]
struct Gate {
    min_completed_per_1000_tokens: f64,
    max_repeated_content_rate: f64,
    max_incorrect_absence_claims: usize,
    max_irrelevant_results_opened: usize,
    max_modeled_latency_ms: usize,
    min_inspect_call_savings: f64,
    min_inspect_token_savings: f64,
}

#[derive(Debug, Deserialize)]
struct TaskSpec {
    id: String,
    mode: String,
    required: Vec<String>,
    max_calls: usize,
}

#[derive(Debug, Default)]
struct TaskMetrics {
    id: String,
    completed: bool,
    required_recall: f64,
    calls: usize,
    delivered_bytes: usize,
    delivered_tokens: usize,
    modeled_latency_to_first_correct_ms: usize,
    irrelevant_results_opened: usize,
    repeated_deliveries: usize,
    total_deliveries: usize,
    incorrect_absence_claims: usize,
    status_correct: bool,
}

impl TaskMetrics {
    fn repeated_content_rate(&self) -> f64 {
        if self.total_deliveries == 0 {
            0.0
        } else {
            self.repeated_deliveries as f64 / self.total_deliveries as f64
        }
    }
}

#[derive(Debug)]
struct Observation {
    payload: Value,
    bytes: usize,
    tokens: usize,
    content_text: String,
    structured_bytes: usize,
}

/// Stable, model-free bag-of-words embedder.
///
/// FNV-1a maps lowercase word tokens into a fixed feature vector. Queries and
/// fixture content that share behavioral vocabulary are close, while every run
/// remains byte-identical and network-free.
struct WordHashEmbedder;

impl Embedder for WordHashEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts.iter().map(|text| word_hash_vector(text)).collect())
    }

    fn dimension(&self) -> usize {
        DIMENSION
    }
}

fn word_hash_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; DIMENSION];
    let dimension = u64::try_from(DIMENSION).expect("fixture dimension fits u64");
    for token in text
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|token| token.len() >= 2)
    {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in token.bytes().map(|byte| byte.to_ascii_lowercase()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let slot = usize::try_from(hash % dimension).expect("slot fits usize");
        vector[slot] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn load_spec() -> EvaluationSpec {
    let raw = std::fs::read_to_string(format!("{FIXTURE_ROOT}/tasks.json"))
        .expect("read agent-efficiency tasks.json");
    serde_json::from_str(&raw).expect("parse agent-efficiency tasks.json")
}

fn load_protocol_traces() -> BTreeMap<String, Value> {
    let raw = std::fs::read_to_string(format!("{FIXTURE_ROOT}/protocol-traces.json"))
        .expect("read agent-efficiency protocol traces");
    serde_json::from_str(&raw).expect("parse agent-efficiency protocol traces")
}

#[allow(
    clippy::too_many_arguments,
    reason = "compact fixture constructor mirrors the stored symbol record"
)]
fn symbol(
    id: &str,
    file_path: &str,
    name: &str,
    kind: &str,
    signature: &str,
    doc: &str,
    line_start: u32,
    line_end: u32,
) -> SymbolRecord {
    SymbolRecord {
        id: SymbolId(id.into()),
        file_path: file_path.into(),
        name: name.into(),
        kind: kind.into(),
        visibility: "pub".into(),
        signature: signature.into(),
        doc_comment: Some(doc.into()),
        module_path: file_path.trim_end_matches(".rs").replace(['/', '\\'], "::"),
        line_start,
        line_end,
        cyclomatic_complexity: None,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one explicit fixture builder keeps the task corpus auditable"
)]
async fn setup_server() -> MinistrServer {
    let storage = SqliteStorage::open_in_memory().expect("open in-memory storage");
    let embedder: Arc<dyn Embedder> = Arc::new(WordHashEmbedder);
    let index: Arc<dyn VectorIndex> = Arc::new(ExactScanIndex::new(DIMENSION));

    let routing_doc =
        std::fs::read_to_string(format!("{FIXTURE_ROOT}/corpus-primary/docs/routing.md"))
            .expect("read routing documentation fixture");
    storage
        .insert_document(&DocumentTree {
            id: ContentId("docs/routing.md".into()),
            title: "Routing".into(),
            source_path: "docs/routing.md".into(),
            sections: vec![Section {
                id: SectionId("docs/routing.md#dispatch-contract".into()),
                heading_path: vec!["Routing".into(), "Dispatch contract".into()],
                depth: 2,
                text: routing_doc.clone(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![],
                summary: Some(
                    "Production dispatcher chooses an inbound wire handler and emits a reply."
                        .into(),
                ),
            }],
            summary: Some("Production routing and HTTP dispatch contract.".into()),
        })
        .await
        .expect("insert routing document");
    let router_source =
        std::fs::read_to_string(format!("{FIXTURE_ROOT}/corpus-primary/src/router.rs"))
            .expect("read router source fixture");
    storage
        .insert_document(&DocumentTree {
            id: ContentId("src/router.rs".into()),
            title: "router.rs".into(),
            source_path: "src/router.rs".into(),
            sections: vec![Section {
                id: SectionId("src/router.rs#root".into()),
                heading_path: vec!["router.rs".into()],
                depth: 1,
                text: router_source,
                structural_nodes: vec![],
                children: vec![],
                claims: vec![],
                summary: Some("Production wire dispatcher implementation.".into()),
            }],
            summary: Some("Production wire dispatcher implementation.".into()),
        })
        .await
        .expect("insert router source document");

    let symbols = vec![
        symbol(
            "sym-routing::parse_envelope",
            "src/router.rs",
            "parse_envelope",
            "function",
            "pub fn parse_envelope(bytes: &[u8]) -> Option<u16>",
            "Parses an inbound wire envelope.",
            3,
            5,
        ),
        symbol(
            "sym-routing::deliver_reply",
            "src/router.rs",
            "deliver_reply",
            "function",
            "pub fn deliver_reply(kind: u16) -> Vec<u8>",
            "Emits the selected handler reply.",
            7,
            9,
        ),
        symbol(
            "sym-routing::dispatch_request",
            "src/router.rs",
            "dispatch_request",
            "function",
            "pub fn dispatch_request(bytes: &[u8]) -> Vec<u8>",
            "Chooses the production handler for an inbound wire envelope and emits the reply.",
            11,
            16,
        ),
        symbol(
            "sym-routing::route_loop",
            "src/router.rs",
            "route_loop",
            "function",
            "pub fn route_loop(frame: &[u8]) -> Vec<u8>",
            "Production caller that forwards frames to the dispatcher.",
            18,
            20,
        ),
        symbol(
            "sym-tests::dispatches_ping",
            "tests/router_tests.rs",
            "dispatches_ping",
            "function",
            "fn dispatches_ping()",
            "Relevant regression test for production dispatch.",
            4,
            6,
        ),
        symbol(
            "sym-generated::dispatch_request_generated",
            "generated/bindings.rs",
            "dispatch_request_generated",
            "function",
            "pub fn dispatch_request_generated(bytes: &[u8]) -> Vec<u8>",
            "Generated wrapper, not the production implementation.",
            2,
            4,
        ),
        symbol(
            "sym-vendor::dispatch_request_vendor",
            "vendor/router.rs",
            "dispatch_request_vendor",
            "function",
            "pub fn dispatch_request_vendor(bytes: &[u8]) -> Vec<u8>",
            "Vendored dispatch helper, not the production implementation.",
            1,
            3,
        ),
    ];
    storage
        .insert_symbols(&symbols)
        .await
        .expect("insert symbols");
    storage
        .insert_symbol_refs(&[
            SymbolRefRecord {
                from_symbol_id: SymbolId("sym-routing::route_loop".into()),
                to_symbol_id: SymbolId("sym-routing::dispatch_request".into()),
                ref_kind: RefKind::Calls,
            },
            SymbolRefRecord {
                from_symbol_id: SymbolId("sym-tests::dispatches_ping".into()),
                to_symbol_id: SymbolId("sym-routing::dispatch_request".into()),
                ref_kind: RefKind::Calls,
            },
            SymbolRefRecord {
                from_symbol_id: SymbolId("sym-routing::dispatch_request".into()),
                to_symbol_id: SymbolId("sym-routing::parse_envelope".into()),
                ref_kind: RefKind::Calls,
            },
            SymbolRefRecord {
                from_symbol_id: SymbolId("sym-routing::dispatch_request".into()),
                to_symbol_id: SymbolId("sym-routing::deliver_reply".into()),
                ref_kind: RefKind::Calls,
            },
        ])
        .await
        .expect("insert symbol references");

    let endpoints = [
        BridgeEndpointRecord {
            id: None,
            file_path: "src/router.rs".into(),
            binding_key: "POST /dispatch".into(),
            kind: "http_route".into(),
            role: "export".into(),
            language: "rust".into(),
            line: 11,
            symbol_name: "dispatch_request".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "web/client.ts".into(),
            binding_key: "POST /dispatch".into(),
            kind: "http_route".into(),
            role: "import".into(),
            language: "typescript".into(),
            line: 2,
            symbol_name: "dispatch".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "src/tauri.rs".into(),
            binding_key: "refresh_project".into(),
            kind: "tauri_command".into(),
            role: "export".into(),
            language: "rust".into(),
            line: 2,
            symbol_name: "refresh_project".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "web/tauri.ts".into(),
            binding_key: "refresh_project".into(),
            kind: "tauri_command".into(),
            role: "import".into(),
            language: "typescript".into(),
            line: 3,
            symbol_name: "refreshProject".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "src/python.rs".into(),
            binding_key: "normalize_record".into(),
            kind: "pyo3".into(),
            role: "export".into(),
            language: "rust".into(),
            line: 2,
            symbol_name: "normalize_record".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "python/client.py".into(),
            binding_key: "normalize_record".into(),
            kind: "pyo3".into(),
            role: "import".into(),
            language: "python".into(),
            line: 1,
            symbol_name: "normalize_record".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "src/napi.rs".into(),
            binding_key: "encodePacket".into(),
            kind: "napi".into(),
            role: "export".into(),
            language: "rust".into(),
            line: 2,
            symbol_name: "encode_packet".into(),
            confidence: 1.0,
        },
        BridgeEndpointRecord {
            id: None,
            file_path: "node/client.js".into(),
            binding_key: "encodePacket".into(),
            kind: "napi".into(),
            role: "import".into(),
            language: "javascript".into(),
            line: 1,
            symbol_name: "encodePacket".into(),
            confidence: 1.0,
        },
    ];
    let endpoint_ids = storage
        .insert_bridge_endpoints(&endpoints)
        .await
        .expect("insert bridge endpoints");
    let bridge_kinds = ["http_route", "tauri_command", "pyo3", "napi"];
    let links: Vec<BridgeLinkRecord> = bridge_kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| BridgeLinkRecord {
            export_ep_id: endpoint_ids[index * 2],
            import_ep_id: endpoint_ids[index * 2 + 1],
            kind: (*kind).into(),
            confidence: 1.0,
        })
        .collect();
    storage
        .insert_bridge_links(&links)
        .await
        .expect("insert bridge link");

    let vector_text = [
        (
            "doc-summary::docs/routing.md",
            "Production routing and HTTP dispatch contract",
        ),
        (
            "sec-summary::docs/routing.md#dispatch-contract",
            "Production dispatcher chooses inbound wire handler and emits reply",
        ),
        (
            "section::docs/routing.md#dispatch-contract",
            routing_doc.as_str(),
        ),
        (
            "symbol-stub::sym-routing::dispatch_request",
            "dispatch_request production dispatcher chooses handler inbound wire envelope emits reply",
        ),
        (
            "symbol-full::sym-routing::dispatch_request",
            "pub fn dispatch_request bytes parse_envelope deliver_reply production",
        ),
        (
            "symbol-stub::sym-routing::route_loop",
            "route_loop production caller forwards frames dispatcher",
        ),
        (
            "symbol-stub::sym-tests::dispatches_ping",
            "dispatches_ping relevant regression test production dispatch",
        ),
        (
            "symbol-stub::sym-generated::dispatch_request_generated",
            "generated wrapper dispatch request",
        ),
        (
            "symbol-stub::sym-vendor::dispatch_request_vendor",
            "vendored dispatch helper",
        ),
    ];
    for (id, text) in vector_text {
        let vectors = embedder.embed(&[text]).expect("embed fixture entry");
        index.insert(id, &vectors[0]).expect("index fixture entry");
    }

    MinistrServer::new(Arc::new(QueryService::new(storage, embedder, index)))
}

async fn wrap_as_client(server: MinistrServer) -> (McpClient, McpServerHandle) {
    let (client_write, server_read) = tokio::io::duplex(1 << 20);
    let (server_write, client_read) = tokio::io::duplex(1 << 20);
    let server_task = tokio::spawn(async move {
        server
            .serve((server_read, server_write))
            .await
            .expect("serve test MCP")
    });
    let client = ().serve((client_read, client_write)).await.expect("start test MCP client");
    let server_handle = server_task.await.expect("join test MCP server");
    (client, server_handle)
}

fn payload_of(result: &CallToolResult) -> Value {
    if let Some(payload) = &result.structured_content {
        return payload.clone();
    }
    result
        .content
        .iter()
        .find_map(|content| content.raw.as_text())
        .and_then(|text| serde_json::from_str(&text.text).ok())
        .unwrap_or_else(|| json!({"status": "error", "error_code": "missing_payload"}))
}

async fn observe(client: &McpClient, name: &str, args: Value) -> Observation {
    let result = call_tool(client, name, args).await;
    observe_result(&result)
}

async fn call_tool(client: &McpClient, name: &str, args: Value) -> CallToolResult {
    let arguments = args.as_object().map(|map| {
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    });
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(arguments) = arguments {
        params = params.with_arguments(arguments);
    }
    client
        .peer()
        .call_tool(params)
        .await
        .unwrap_or_else(|error| panic!("{name} failed at MCP transport: {error}"))
}

fn observe_result(result: &CallToolResult) -> Observation {
    let content_text = result
        .content
        .iter()
        .filter_map(|content| content.raw.as_text())
        .map(|text| text.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let structured_bytes = result
        .structured_content
        .as_ref()
        .map(|value| {
            serde_json::to_vec(value)
                .expect("serialize structured content")
                .len()
        })
        .unwrap_or_default();
    let encoded = serde_json::to_vec(&result).expect("serialize literal CallToolResult");
    let text = String::from_utf8(encoded.clone()).expect("MCP result is UTF-8 JSON");
    Observation {
        payload: payload_of(result),
        bytes: encoded.len(),
        tokens: count_tokens(&text),
        content_text,
        structured_bytes,
    }
}

fn result_data(payload: &Value) -> &Value {
    payload.get("result").unwrap_or(payload)
}

fn collect_evidence(value: &Value, evidence: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "content_id"
                        | "symbol_id"
                        | "id"
                        | "from_symbol_id"
                        | "to_symbol_id"
                        | "file"
                        | "file_path"
                        | "from_file"
                        | "to_file"
                        | "export_file"
                        | "import_file"
                        | "error_code"
                        | "status"
                        | "completeness"
                ) && let Some(text) = child.as_str()
                {
                    evidence.insert(text.to_string());
                }
                collect_evidence(child, evidence);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_evidence(child, evidence);
            }
        }
        _ => {}
    }
}

fn status_is_success(payload: &Value) -> bool {
    let Some(status) = payload
        .get("status")
        .or_else(|| result_data(payload).get("status"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    matches!(status, "ok" | "partial")
}

fn metrics_from_observations(
    task: &TaskSpec,
    observations: &[Observation],
    irrelevant_results_opened: usize,
) -> TaskMetrics {
    let mut evidence = BTreeSet::new();
    let mut first_correct_call = None;
    for (call_index, observation) in observations.iter().enumerate() {
        let mut call_evidence = BTreeSet::new();
        collect_evidence(&observation.payload, &mut call_evidence);
        if first_correct_call.is_none()
            && task
                .required
                .iter()
                .any(|required| call_evidence.contains(required.as_str()))
        {
            first_correct_call = Some(call_index + 1);
        }
        evidence.extend(call_evidence);
    }
    let found = task
        .required
        .iter()
        .filter(|required| evidence.contains(required.as_str()))
        .count();
    let required_recall = if task.required.is_empty() {
        1.0
    } else {
        found as f64 / task.required.len() as f64
    };
    let status_correct = observations
        .iter()
        .all(|observation| status_is_success(&observation.payload));
    TaskMetrics {
        id: task.id.clone(),
        completed: found == task.required.len()
            && observations.len() <= task.max_calls
            && status_correct,
        required_recall,
        calls: observations.len(),
        delivered_bytes: observations.iter().map(|item| item.bytes).sum(),
        delivered_tokens: observations.iter().map(|item| item.tokens).sum(),
        modeled_latency_to_first_correct_ms: first_correct_call.unwrap_or(observations.len()) * 40,
        irrelevant_results_opened,
        status_correct,
        incorrect_absence_claims: usize::from(found < task.required.len()),
        ..TaskMetrics::default()
    }
}

fn task<'a>(spec: &'a EvaluationSpec, id: &str) -> &'a TaskSpec {
    spec.tasks
        .iter()
        .find(|task| task.id == id)
        .unwrap_or_else(|| panic!("missing task contract: {id}"))
}

fn result_array<'a>(observation: &'a Observation, field: &str) -> &'a [Value] {
    result_data(&observation.payload)
        .get(field)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn survey_identities(observation: &Observation) -> Vec<String> {
    result_array(observation, "results")
        .iter()
        .filter_map(|result| {
            let locator = result.get("locator");
            let identity = locator.and_then(|value| value.get("identity")).or(locator);
            let corpus = identity
                .and_then(|value| value.get("corpus_id"))
                .or_else(|| result.get("source_corpus"))
                .or_else(|| result.get("project"))
                .and_then(Value::as_str)
                .unwrap_or("primary");
            let content_id = identity
                .and_then(|value| value.get("content_id"))
                .or_else(|| result.get("content_id"))
                .and_then(Value::as_str)?;
            let resolution = identity
                .and_then(|value| value.get("resolution"))
                .or_else(|| result.get("resolution"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("{corpus}|{content_id}|{resolution}"))
        })
        .collect()
}

async fn run_simple_task(
    task: &TaskSpec,
    tool: &str,
    args: Value,
    result_field: &str,
) -> TaskMetrics {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let observation = observe(&client, tool, args).await;
    let irrelevant = result_array(&observation, result_field)
        .iter()
        .filter(|result| {
            let rendered = result.to_string();
            !task
                .required
                .iter()
                .any(|required| rendered.contains(required))
        })
        .count();
    metrics_from_observations(task, &[observation], irrelevant)
}

async fn run_overlap_task(task: &TaskSpec) -> TaskMetrics {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let first = observe(
        &client,
        "ministr_survey",
        json!({
            "query": "production dispatcher inbound wire handler emits reply",
            "top_k": 8
        }),
    )
    .await;
    let second = observe(
        &client,
        "ministr_survey",
        json!({
            "query": "inbound wire dispatcher chooses handler and reply",
            "top_k": 8
        }),
    )
    .await;

    let first_ids: BTreeSet<String> = survey_identities(&first).into_iter().collect();
    let second_ids = survey_identities(&second);
    let repeats = second_ids
        .iter()
        .filter(|identity| first_ids.contains(*identity))
        .count();
    let deduplicated = result_data(&second.payload)
        .get("deduplicated_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let mut metrics = metrics_from_observations(task, &[first, second], 0);
    metrics.repeated_deliveries = repeats;
    metrics.total_deliveries = first_ids.len() + second_ids.len();
    metrics.completed = repeats == 0 && deduplicated > 0;
    metrics.required_recall = f64::from(metrics.completed);
    metrics.incorrect_absence_claims = usize::from(!metrics.completed);
    metrics
}

fn protocol_metrics(task: &TaskSpec, trace: &Value) -> TaskMetrics {
    let mut result = CallToolResult::structured(trace.clone());
    result.content = vec![Content::text(format!("{}: protocol fixture", task.id))];
    let observation = observe_result(&result);
    let mut evidence = BTreeSet::new();
    collect_evidence(trace, &mut evidence);

    if task.id == "colliding-content-ids"
        && let Some(results) = trace.get("results").and_then(Value::as_array)
    {
        for result in results {
            if let Some(locator) = result.get("locator") {
                let identity = locator.get("identity").unwrap_or(locator);
                let corpus = identity["corpus_id"].as_str().unwrap_or_default();
                let content = identity["content_id"].as_str().unwrap_or_default();
                let resolution = identity["resolution"].as_str().unwrap_or_default();
                evidence.insert(format!("{corpus}|{content}|{resolution}"));
            }
        }
    }
    if trace.pointer("/completeness/absence_is_conclusive") == Some(&Value::Bool(false)) {
        evidence.insert("absence_is_conclusive:false".into());
    }

    let found = task
        .required
        .iter()
        .filter(|required| evidence.contains(required.as_str()))
        .count();
    let completed = found == task.required.len();
    TaskMetrics {
        id: task.id.clone(),
        completed,
        required_recall: found as f64 / task.required.len() as f64,
        calls: 1,
        delivered_bytes: observation.bytes,
        delivered_tokens: observation.tokens,
        modeled_latency_to_first_correct_ms: 40,
        incorrect_absence_claims: usize::from(!completed),
        status_correct: completed,
        total_deliveries: trace
            .get("results")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        ..TaskMetrics::default()
    }
}

async fn run_inspect_comparison(task: &TaskSpec, gate: &Gate) -> TaskMetrics {
    let (granular_client, _server) = wrap_as_client(setup_server().await).await;
    let granular = [
        observe(
            &granular_client,
            "ministr_symbols",
            json!({"query": "dispatch_request", "limit": 10}),
        )
        .await,
        observe(
            &granular_client,
            "ministr_definition",
            json!({"symbol_id": "sym-routing::dispatch_request"}),
        )
        .await,
        observe(
            &granular_client,
            "ministr_references",
            json!({"symbol_id": "sym-routing::dispatch_request", "limit": 20}),
        )
        .await,
        observe(
            &granular_client,
            "ministr_bridge",
            json!({"query": "dispatch_request"}),
        )
        .await,
    ];

    let (inspect_client, _server) = wrap_as_client(setup_server().await).await;
    let inspect = observe(
        &inspect_client,
        "ministr_inspect",
        json!({
            "symbol_id": "sym-routing::dispatch_request",
            "include": ["definition", "callers", "callees", "tests", "bridges"],
            "max_per_group": 10,
            "max_source_lines": 160
        }),
    )
    .await;

    let granular_tokens: usize = granular.iter().map(|item| item.tokens).sum();
    let inspect_tokens = inspect.tokens;
    let call_savings = 1.0 - 1.0 / granular.len() as f64;
    let token_savings = 1.0 - inspect_tokens as f64 / granular_tokens as f64;
    let inspect_result = result_data(&inspect.payload);
    let has_definition = inspect_result
        .pointer("/definition/signature")
        .and_then(Value::as_str)
        .is_some_and(|signature| signature.contains("pub fn dispatch_request"))
        && inspect_result
            .pointer("/definition/source_context")
            .and_then(Value::as_str)
            .is_some_and(|source| !source.is_empty());
    let has_callees = inspect_result
        .pointer("/callees/items")
        .and_then(Value::as_array)
        .is_some_and(|items| items.len() >= 2);
    let has_bridges = inspect_result
        .pointer("/bridges/items")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    let mut metrics = metrics_from_observations(task, &[inspect], 0);
    metrics.completed = metrics.completed
        && has_definition
        && has_callees
        && has_bridges
        && call_savings >= gate.min_inspect_call_savings
        && token_savings >= gate.min_inspect_token_savings;
    metrics.incorrect_absence_claims = usize::from(!metrics.completed);
    eprintln!(
        "inspect comparison: granular={} calls/{} tokens, inspect=1 call/{} tokens, savings={:.1}%, definition={has_definition}, callees={has_callees}, bridges={has_bridges}",
        granular.len(),
        granular_tokens,
        inspect_tokens,
        token_savings * 100.0
    );
    metrics
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn agent_efficiency_regression_gate() {
    let spec = load_spec();
    assert_eq!(spec.version, 1, "unsupported task contract version");
    let traces = load_protocol_traces();
    let mut metrics = Vec::new();

    metrics.push(
        run_simple_task(
            task(&spec, "locate-vague-implementation"),
            "ministr_survey",
            json!({
                "query": "where does the production inbound wire envelope choose a handler and emit its reply",
                "top_k": 8
            }),
            "results",
        )
        .await,
    );
    metrics.push(
        run_simple_task(
            task(&spec, "find-exact-identifier"),
            "ministr_symbols",
            json!({"query": "dispatch_request", "limit": 10}),
            "symbols",
        )
        .await,
    );
    metrics.push(
        run_simple_task(
            task(&spec, "find-all-callers"),
            "ministr_references",
            json!({"symbol_id": "sym-routing::dispatch_request", "limit": 20}),
            "references",
        )
        .await,
    );
    metrics.push(
        run_simple_task(
            task(&spec, "find-relevant-tests"),
            "ministr_references",
            json!({"symbol_id": "sym-routing::dispatch_request", "limit": 20}),
            "references",
        )
        .await,
    );
    metrics.push(
        run_simple_task(
            task(&spec, "trace-cross-language-boundaries"),
            "ministr_bridge",
            json!({}),
            "links",
        )
        .await,
    );
    metrics.push(
        run_simple_task(
            task(&spec, "mixed-code-and-documentation"),
            "ministr_survey",
            json!({
                "query": "production dispatcher inbound wire handler emits reply HTTP contract",
                "top_k": 10
            }),
            "results",
        )
        .await,
    );
    metrics.push(run_overlap_task(task(&spec, "overlapping-query-no-redelivery")).await);

    for id in [
        "colliding-content-ids",
        "active-indexing-is-not-conclusive",
        "partial-backend-failure",
    ] {
        let task = task(&spec, id);
        assert_eq!(task.mode, "protocol_trace");
        let trace = traces
            .get(id)
            .unwrap_or_else(|| panic!("missing protocol trace: {id}"));
        metrics.push(protocol_metrics(task, trace));
    }

    metrics.push(run_inspect_comparison(task(&spec, "inspect-versus-granular"), &spec.gate).await);

    assert_eq!(
        metrics.len(),
        spec.tasks.len(),
        "every task contract must be executed exactly once"
    );
    for task in &spec.tasks {
        assert!(
            metrics.iter().any(|metric| metric.id == task.id),
            "task was not executed: {}",
            task.id
        );
        assert!(
            matches!(task.mode.as_str(), "live_mcp" | "protocol_trace"),
            "unknown task mode: {}",
            task.mode
        );
    }

    eprintln!(
        "\n{:<38} {:>4} {:>7} {:>8} {:>7} {:>7} {:>7} {:>7}",
        "task", "ok", "recall", "tokens", "bytes", "calls", "repeat", "abs-err"
    );
    for metric in &metrics {
        eprintln!(
            "{:<38} {:>4} {:>7.2} {:>8} {:>7} {:>7} {:>7.2} {:>7}",
            metric.id,
            if metric.completed { "yes" } else { "NO" },
            metric.required_recall,
            metric.delivered_tokens,
            metric.delivered_bytes,
            metric.calls,
            metric.repeated_content_rate(),
            metric.incorrect_absence_claims
        );
    }

    let completed = metrics.iter().filter(|metric| metric.completed).count();
    let delivered_tokens: usize = metrics.iter().map(|metric| metric.delivered_tokens).sum();
    let delivered_bytes: usize = metrics.iter().map(|metric| metric.delivered_bytes).sum();
    let total_calls: usize = metrics.iter().map(|metric| metric.calls).sum();
    let modeled_latency_ms: usize = metrics
        .iter()
        .map(|metric| metric.modeled_latency_to_first_correct_ms)
        .sum();
    let irrelevant_results: usize = metrics
        .iter()
        .map(|metric| metric.irrelevant_results_opened)
        .sum();
    let repeated: usize = metrics
        .iter()
        .map(|metric| metric.repeated_deliveries)
        .sum();
    let deliveries: usize = metrics.iter().map(|metric| metric.total_deliveries).sum();
    let incorrect_absence_claims: usize = metrics
        .iter()
        .map(|metric| metric.incorrect_absence_claims)
        .sum();
    let efficiency = 1000.0 * completed as f64 / delivered_tokens as f64;
    let repeated_rate = if deliveries == 0 {
        0.0
    } else {
        repeated as f64 / deliveries as f64
    };

    eprintln!(
        "aggregate: {completed}/{} complete, {total_calls} calls, {delivered_tokens} tokens, {delivered_bytes} bytes, {modeled_latency_ms} modeled ms, {irrelevant_results} irrelevant opens, efficiency={efficiency:.3}/1k tokens",
        metrics.len()
    );

    let failures: Vec<&str> = metrics
        .iter()
        .filter(|metric| !metric.completed || !metric.status_correct)
        .map(|metric| metric.id.as_str())
        .collect();
    assert!(failures.is_empty(), "task failures: {failures:?}");
    assert!(
        efficiency >= spec.gate.min_completed_per_1000_tokens,
        "correct completions per 1,000 tokens {efficiency:.3} fell below {:.3}",
        spec.gate.min_completed_per_1000_tokens
    );
    assert!(
        repeated_rate <= spec.gate.max_repeated_content_rate,
        "repeated-content rate {repeated_rate:.3} exceeded {:.3}",
        spec.gate.max_repeated_content_rate
    );
    assert!(
        incorrect_absence_claims <= spec.gate.max_incorrect_absence_claims,
        "incorrect absence claims {incorrect_absence_claims} exceeded {}",
        spec.gate.max_incorrect_absence_claims
    );
    assert!(
        irrelevant_results <= spec.gate.max_irrelevant_results_opened,
        "irrelevant results opened {irrelevant_results} exceeded {}",
        spec.gate.max_irrelevant_results_opened
    );
    assert!(
        modeled_latency_ms <= spec.gate.max_modeled_latency_ms,
        "modeled latency {modeled_latency_ms}ms exceeded {}ms",
        spec.gate.max_modeled_latency_ms
    );
}

async fn code_fixture_tools() -> Vec<rmcp::model::Tool> {
    let mut server = setup_server().await;
    server.prune_tools(
        &[
            "src/router.rs",
            "tests/router_tests.rs",
            "docs/routing.md",
            "web/client.ts",
            "src/tauri.rs",
            "web/tauri.ts",
            "src/python.rs",
            "python/client.py",
            "src/napi.rs",
            "node/client.js",
        ]
        .map(|relative| {
            std::path::PathBuf::from(format!("{FIXTURE_ROOT}/corpus-primary/{relative}"))
        }),
    );
    let (client, _server) = wrap_as_client(server).await;
    client
        .peer()
        .list_all_tools()
        .await
        .expect("list MCP tools")
}

struct SchemaEconomy {
    literal: usize,
    comparable: usize,
    per_tool: Vec<(usize, String)>,
}

fn measure_schema_economy(tools: &[rmcp::model::Tool]) -> SchemaEconomy {
    let wire = serde_json::to_string(&tools).expect("serialize literal tools/list catalog");
    let mut legacy_selection_text = String::new();
    let mut per_tool_tokens = Vec::new();
    for tool in tools {
        let mut selection_text = tool.name.to_string();
        legacy_selection_text.push_str(&tool.name);
        if let Some(description) = &tool.description {
            selection_text.push_str(description);
            legacy_selection_text.push_str(description);
            assert!(
                description.split_whitespace().count() <= 70,
                "{} description is too verbose: {description}",
                tool.name
            );
            assert!(
                !description.starts_with("ministr is a code intelligence MCP server"),
                "{} repeats the generic server preamble",
                tool.name
            );
        }
        let input_schema =
            serde_json::to_string(&tool.input_schema).expect("serialize input schema");
        selection_text.push_str(&input_schema);
        legacy_selection_text.push_str(&input_schema);
        per_tool_tokens.push((count_tokens(&selection_text), tool.name.to_string()));
        assert!(
            tool.output_schema.is_some(),
            "{} must advertise a machine-readable output schema",
            tool.name
        );
    }
    SchemaEconomy {
        literal: count_tokens(&wire),
        comparable: count_tokens(&legacy_selection_text),
        per_tool: per_tool_tokens,
    }
}

fn assert_discriminative_tool_routing(tools: &[rmcp::model::Tool]) -> BTreeSet<&str> {
    let names: BTreeSet<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    for essential in [
        "ministr_survey",
        "ministr_symbols",
        "ministr_inspect",
        "ministr_definition",
        "ministr_references",
        "ministr_read",
    ] {
        assert!(
            names.contains(essential),
            "code-corpus pruning hid essential tool {essential}"
        );
    }

    let description = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .and_then(|tool| tool.description.as_deref())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    assert!(
        description("ministr_survey").contains("vague"),
        "survey description must discriminate vague discovery"
    );
    assert!(
        description("ministr_symbols").contains("name"),
        "symbols description must discriminate exact/name lookup"
    );
    assert!(
        description("ministr_inspect").contains("bounded"),
        "inspect description must advertise bounded compound navigation"
    );
    names
}

#[tokio::test]
async fn schema_and_tool_selection_economy_gate() {
    let tools = code_fixture_tools().await;
    let mut economy = measure_schema_economy(&tools);
    let names = assert_discriminative_tool_routing(&tools);
    let comparable_per_tool = economy.comparable as f64 / tools.len() as f64;

    eprintln!(
        "schema economy: {} pruned tools, {} literal tools/list tokens; legacy-comparable name+description+input={} tokens (pre-overhaul reported 4,783 across 20 tools)",
        tools.len(),
        economy.literal,
        economy.comparable,
    );
    eprintln!("schema economy tools: {names:?}");
    economy
        .per_tool
        .sort_unstable_by(|left, right| right.cmp(left));
    eprintln!("schema economy per tool: {:?}", economy.per_tool);

    assert!(
        tools.len() <= 26,
        "full discoverable catalog unexpectedly grew past 26 tools"
    );
    assert!(
        economy.comparable <= 4_783,
        "legacy-comparable schema cost {} exceeded the pre-overhaul 4,783-token catalog",
        economy.comparable
    );
    assert!(
        comparable_per_tool <= 180.0,
        "legacy-comparable schema cost regressed to {comparable_per_tool:.1} tokens/tool (pre-overhaul average: 239.2)"
    );
    assert!(
        economy.literal <= 40_000,
        "literal tools/list catalog exceeded the 40k-token hard cap: {}",
        economy.literal
    );
}

#[tokio::test]
async fn payload_probe_child() {
    let Some(mode) = std::env::var("MINISTR_EVAL_PAYLOAD_PROBE").ok() else {
        return;
    };
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let result = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "production dispatcher", "top_k": 8}),
    )
    .await;
    let observation = observe_result(&result);
    println!(
        "PAYLOAD_PROBE:{}",
        json!({
            "mode": mode,
            "payload": observation.payload,
            "content_text": observation.content_text,
            "structured_bytes": observation.structured_bytes,
            "wire_bytes": observation.bytes
        })
    );
}

fn run_payload_probe(mode: &str) -> Value {
    let mut command =
        std::process::Command::new(std::env::current_exe().expect("current test exe"));
    command
        .arg("--exact")
        .arg("payload_probe_child")
        .arg("--nocapture")
        .env("MINISTR_EVAL_PAYLOAD_PROBE", mode);
    if mode == "legacy" {
        command.env("MINISTR_MCP_LEGACY_TEXT_CONTENT", "1");
    } else {
        command.env_remove("MINISTR_MCP_LEGACY_TEXT_CONTENT");
    }
    let output = command.output().expect("run isolated payload probe");
    assert!(
        output.status.success(),
        "payload probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("probe output is UTF-8");
    let payload = stdout
        .lines()
        .find_map(|line| line.strip_prefix("PAYLOAD_PROBE:"))
        .unwrap_or_else(|| panic!("payload probe marker missing from: {stdout}"));
    serde_json::from_str(payload).expect("parse payload probe JSON")
}

#[test]
fn mcp_payload_is_compact_by_default_and_legacy_mode_preserves_parity() {
    let default = run_payload_probe("default");
    let legacy = run_payload_probe("legacy");
    let default_text = default["content_text"].as_str().unwrap_or_default();
    let default_structured_bytes = default["structured_bytes"].as_u64().unwrap_or_default();

    assert!(
        default_structured_bytes > 0,
        "structuredContent is canonical"
    );
    assert!(
        u64::try_from(default_text.len()).expect("text length fits u64") * 4
            < default_structured_bytes,
        "default human summary must be materially smaller than structured data: {} versus {} bytes",
        default_text.len(),
        default_structured_bytes
    );
    assert!(
        serde_json::from_str::<Value>(default_text).is_err(),
        "default content must not duplicate the JSON payload"
    );
    let legacy_text: Value = serde_json::from_str(
        legacy["content_text"]
            .as_str()
            .expect("legacy content is text"),
    )
    .expect("legacy content remains full JSON");
    assert_eq!(
        legacy_text, legacy["payload"],
        "legacy text and structured parity"
    );
    assert_eq!(
        default["payload"], legacy["payload"],
        "compatibility does not alter data"
    );
    assert!(
        legacy["wire_bytes"].as_u64().unwrap_or_default()
            >= default["wire_bytes"].as_u64().unwrap_or_default()
                + legacy["structured_bytes"].as_u64().unwrap_or_default() / 2,
        "legacy duplication must be visible in literal boundary bytes"
    );
}

#[tokio::test]
async fn symbol_to_definition_prefetch_has_measurable_value() {
    let (client, _server) = wrap_as_client(setup_server().await).await;
    let _symbols = observe(
        &client,
        "ministr_symbols",
        json!({"query": "dispatch_request", "limit": 10}),
    )
    .await;
    let definition = observe(
        &client,
        "ministr_definition",
        json!({"symbol_id": "sym-routing::dispatch_request"}),
    )
    .await;
    assert_eq!(
        result_data(&definition.payload)["id"],
        "sym-routing::dispatch_request"
    );

    let usage = observe(&client, "ministr_usage", json!({})).await;
    let metrics = &result_data(&usage.payload)["prefetch_metrics"];
    assert!(
        metrics["symbol_search_issued"].as_u64().unwrap_or_default() > 0,
        "symbol search must issue a definition prefetch: {metrics}"
    );
    assert!(
        metrics["symbol_search_hits"].as_u64().unwrap_or_default() > 0,
        "definition must consume the prefetched entry: {metrics}"
    );
    assert!(
        metrics["bytes_saved"].as_u64().unwrap_or_default() > 0
            && metrics["tokens_saved"].as_u64().unwrap_or_default() > 0
            && metrics["latency_saved_ms"].as_u64().unwrap_or_default() > 0,
        "prefetch must report measurable byte/token/latency savings: {metrics}"
    );
}

#[tokio::test]
async fn linked_daemon_survey_to_read_consumes_route_aware_prefetch() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let survey = observe(
        &client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "limit": 5
        }),
    )
    .await;
    let result = result_array(&survey, "results")
        .iter()
        .find(|result| {
            result
                .pointer("/text_metadata/continuation/identity/content_id")
                .and_then(Value::as_str)
                == Some("docs/auth.md#tokens")
        })
        .expect("linked survey should expose a full-read continuation");
    let section_id = result
        .pointer("/text_metadata/continuation/identity/content_id")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(result["text_metadata"]["truncated"], true);
    assert!(
        result["text_metadata"]["original_bytes"]
            .as_u64()
            .unwrap_or_default()
            > result["text_metadata"]["returned_bytes"]
                .as_u64()
                .unwrap_or_default()
    );
    assert!(
        result["text_metadata"]["returned_bytes"]
            .as_u64()
            .unwrap_or(u64::MAX)
            <= 2_048
    );
    let read = observe(
        &client,
        "ministr_read",
        json!({"project": "secondary", "section_id": section_id}),
    )
    .await;
    assert_eq!(
        result_data(&read.payload)["section_id"],
        Value::String(section_id.to_string())
    );
    assert_ne!(read.payload["status"], "error");
    assert!(
        result_data(&read.payload)["text"]
            .as_str()
            .is_some_and(|text| text.contains("FINAL_UNICODE_MARKER_終🙂"))
    );

    let usage = observe(&client, "ministr_usage", json!({})).await;
    let metrics = &result_data(&usage.payload)["prefetch_metrics"];
    assert!(metrics["agent_plan_issued"].as_u64().unwrap_or_default() > 0);
    assert!(metrics["agent_plan_hits"].as_u64().unwrap_or_default() > 0);
    assert!(metrics["tokens_saved"].as_u64().unwrap_or_default() > 0);
}

#[tokio::test]
async fn linked_symbol_next_action_is_directly_executable() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let symbols = observe(
        &client,
        "ministr_symbols",
        json!({"project": "secondary", "query": "MinistrConfig", "limit": 10}),
    )
    .await;
    let action = symbols.payload["next_actions"]
        .as_array()
        .and_then(|actions| actions.first())
        .expect("single linked symbol should suggest its definition");
    assert_eq!(action["action"], "ministr_definition");
    assert_eq!(action["args"]["project"], "secondary");
    let definition = observe(&client, "ministr_definition", action["args"].clone()).await;
    assert_eq!(
        result_data(&definition.payload)["id"],
        "sym-config::MinistrConfig"
    );
    assert_eq!(definition.payload["status"], "partial");
    assert_eq!(
        definition.payload.pointer("/completeness/completeness"),
        Some(&Value::String("partial".into()))
    );
    assert_eq!(
        definition.payload.pointer("/error/error_code"),
        Some(&Value::String("file_unavailable".into()))
    );
    assert_eq!(
        definition
            .payload
            .pointer("/completeness/absence_is_conclusive"),
        Some(&Value::Bool(false))
    );
    let inspect = observe(
        &client,
        "ministr_inspect",
        json!({
            "project": "secondary",
            "symbol_id": "sym-config::MinistrConfig",
            "include": ["definition", "callers"],
            "max_per_group": 5
        }),
    )
    .await;
    let inspected = result_data(&inspect.payload);
    assert_eq!(inspected["locator"]["project"], "secondary");
    assert_eq!(
        inspected["definition"]["locator"]["identity"]["corpus_id"],
        "secondary"
    );
}

#[tokio::test]
async fn cross_corpus_large_excerpts_keep_executable_routing() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let survey = observe(
        &client,
        "ministr_survey",
        json!({
            "corpus_ids": ["primary", "secondary"],
            "query": "JWT RS256 token policy",
            "limit": 10
        }),
    )
    .await;
    let token_hits: Vec<_> = result_array(&survey, "results")
        .iter()
        .filter(|result| {
            result
                .pointer("/text_metadata/continuation/identity/content_id")
                .and_then(Value::as_str)
                == Some("docs/auth.md#tokens")
        })
        .collect();
    let corpora: BTreeSet<_> = token_hits
        .iter()
        .filter_map(|result| result["source_corpus"].as_str())
        .collect();
    assert_eq!(corpora, BTreeSet::from(["primary", "secondary"]));
    for result in token_hits {
        let source = result["source_corpus"].as_str().unwrap();
        assert_eq!(result["text_metadata"]["continuation"]["project"], source);
        assert_eq!(result["text_metadata"]["truncated"], true);
        assert!(
            result["text_metadata"]["returned_bytes"]
                .as_u64()
                .unwrap_or(u64::MAX)
                <= 2_048
        );
    }
    let action = survey.payload["next_actions"]
        .as_array()
        .and_then(|actions| actions.first())
        .expect("top cross-corpus hit should have a routed follow-up");
    assert!(action["args"]["project"].as_str().is_some());
    assert_eq!(action["args"]["project"], action["args"]["source_corpus"]);
}

#[tokio::test]
async fn legacy_bare_drop_is_scoped_to_primary_daemon_corpus() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let primary = observe(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens", "limit": 5}),
    )
    .await;
    let secondary = observe(
        &client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "limit": 5
        }),
    )
    .await;
    let primary_content = result_array(&primary, "results")[0]["content_id"]
        .as_str()
        .unwrap()
        .to_string();
    let secondary_identities: BTreeSet<_> = survey_identities(&secondary).into_iter().collect();
    let dropped = observe(
        &client,
        "ministr_dropped",
        json!({"content_ids": [primary_content]}),
    )
    .await;
    assert_eq!(dropped.payload["status"], "ok");

    let secondary_again = observe(
        &client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "limit": 5
        }),
    )
    .await;
    let repeated: BTreeSet<_> = survey_identities(&secondary_again).into_iter().collect();
    assert!(
        repeated.is_disjoint(&secondary_identities),
        "legacy primary drop must not evict colliding linked identities"
    );
}

#[tokio::test]
async fn linked_daemon_references_to_definition_consumes_prefetch() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let references = observe(
        &client,
        "ministr_references",
        json!({
            "project": "secondary",
            "symbol_id": "sym-config::MinistrConfig",
            "limit": 10
        }),
    )
    .await;
    assert!(!result_array(&references, "references").is_empty());
    let definition = observe(
        &client,
        "ministr_definition",
        json!({
            "project": "secondary",
            "symbol_id": "sym-config::MinistrConfig"
        }),
    )
    .await;
    assert_eq!(
        result_data(&definition.payload)["id"],
        "sym-config::MinistrConfig"
    );
    let usage = observe(&client, "ministr_usage", json!({})).await;
    let metrics = &result_data(&usage.payload)["prefetch_metrics"];
    assert!(
        metrics["reference_follow_issued"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        metrics["reference_follow_hits"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
}

#[test]
fn missing_status_is_not_success() {
    assert!(!status_is_success(&json!({"result": {}})));
}

async fn daemon_multi_client() -> (
    McpClient,
    McpServerHandle,
    daemon_fixture::TestDaemon,
    daemon_fixture::TestDaemon,
) {
    use ministr_mcp::backend::{DaemonBackend, DaemonMultiBackend};
    let primary = daemon_fixture::TestDaemon::start_named("primary").await;
    let secondary = daemon_fixture::TestDaemon::start_named("secondary").await;
    let primary_client = Arc::new(primary.client());
    let secondary_client = Arc::new(secondary.client());
    let primary_session = primary_client
        .create_session(&primary.corpus_id, Some(100_000))
        .await
        .expect("create primary daemon session");
    let secondary_session = secondary_client
        .create_session(&secondary.corpus_id, Some(100_000))
        .await
        .expect("create secondary daemon session");
    let default = Arc::new(DaemonBackend::new(
        primary_client,
        primary.corpus_id.clone(),
        Some(primary_session.session_id.clone()),
    ));
    let linked = std::collections::HashMap::from([(
        "secondary".to_string(),
        Arc::new(DaemonBackend::new(
            secondary_client,
            secondary.corpus_id.clone(),
            Some(secondary_session.session_id),
        )),
    )]);
    let server = MinistrServer::with_daemon_multi_backend(
        DaemonMultiBackend::new(default, linked),
        primary_session.session_id,
    );
    let (client, server_handle) = wrap_as_client(server).await;
    (client, server_handle, primary, secondary)
}

#[tokio::test]
async fn daemon_proxy_toc_reaches_page_two_beyond_old_backend_default() {
    let sections = (0..150)
        .map(|index| Section {
            id: SectionId(format!("large.md#s{index:03}")),
            heading_path: vec!["Large".into(), format!("Section {index:03}")],
            depth: 2,
            text: format!("Section {index:03} body."),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        })
        .collect();
    let daemon = daemon_fixture::TestDaemon::start_with_corpus(vec![DocumentTree {
        id: ContentId("large.md".into()),
        title: "Large".into(),
        source_path: "large.md".into(),
        sections,
        summary: None,
    }])
    .await;
    let daemon_client = Arc::new(daemon.client());
    let session = daemon_client
        .create_session(&daemon.corpus_id, Some(100_000))
        .await
        .unwrap();
    let (client, _server) = wrap_as_client(MinistrServer::with_daemon_backend(
        daemon_client,
        daemon.corpus_id.clone(),
        session.session_id,
    ))
    .await;

    let first = observe(&client, "ministr_toc", json!({"limit": 100})).await;
    let first_result = result_data(&first.payload);
    assert_eq!(first_result["pagination"]["total"], 150);
    assert_eq!(first_result["entries"].as_array().unwrap().len(), 100);
    let cursor = first_result["pagination"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_string();

    let second = observe(
        &client,
        "ministr_toc",
        json!({"cursor": cursor, "limit": 100}),
    )
    .await;
    let second_result = result_data(&second.payload);
    assert_eq!(second_result["pagination"]["total"], 150);
    assert_eq!(second_result["pagination"]["offset"], 100);
    assert_eq!(second_result["entries"].as_array().unwrap().len(), 50);
    assert_eq!(second_result["entries"][0]["section_id"], "large.md#s100");
}

fn assert_cross_corpus_collision(
    primary_results: &Observation,
    secondary_results: &Observation,
) -> (BTreeSet<String>, Value, String) {
    let primary_ids: BTreeSet<String> = survey_identities(primary_results).into_iter().collect();
    let secondary_ids: BTreeSet<String> =
        survey_identities(secondary_results).into_iter().collect();
    assert!(
        primary_ids
            .iter()
            .any(|identity| identity.starts_with("primary|")),
        "primary identities must retain daemon corpus routing: {primary_ids:?}"
    );
    assert!(
        secondary_ids
            .iter()
            .any(|identity| identity.starts_with("secondary|")),
        "same content from linked corpus must not be falsely deduplicated: {secondary_ids:?}"
    );
    let primary_content: BTreeSet<&str> = primary_ids
        .iter()
        .filter_map(|identity| identity.split('|').nth(1))
        .collect();
    assert!(
        secondary_ids.iter().any(|identity| identity
            .split('|')
            .nth(1)
            .is_some_and(|content| primary_content.contains(content))),
        "fixture must prove colliding content IDs across corpora"
    );
    let identity = result_array(secondary_results, "results")[0]["locator"]["identity"].clone();
    let key = format!(
        "{}|{}|{}",
        identity["corpus_id"].as_str().unwrap_or_default(),
        identity["content_id"].as_str().unwrap_or_default(),
        identity["resolution"].as_str().unwrap_or_default()
    );
    (secondary_ids, identity, key)
}

async fn assert_linked_dedup_drop_cycle(
    client: &McpClient,
    secondary_ids: &BTreeSet<String>,
    dropped_identity: &Value,
    dropped_key: &str,
) {
    let overlapping = observe(
        client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "top_k": 5
        }),
    )
    .await;
    let repeated: BTreeSet<String> = survey_identities(&overlapping).into_iter().collect();
    assert!(
        repeated.is_disjoint(secondary_ids),
        "linked overlap redelivered exact identities: {repeated:?}"
    );
    assert!(
        result_data(&overlapping.payload)["deduplicated_count"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "linked overlap must report suppressed deliveries"
    );
    let usage = observe(client, "ministr_usage", json!({})).await;
    assert!(
        result_data(&usage.payload)["session_metrics"]["dedup_hits"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        result_data(&usage.payload)["session_metrics"]["total_tokens_saved"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "dedup must change saved-token accounting"
    );

    let dropped = observe(
        client,
        "ministr_dropped",
        json!({"identities": [dropped_identity.clone()]}),
    )
    .await;
    assert_eq!(dropped.payload["status"], "ok");
    let after_drop = observe(
        client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "top_k": 5
        }),
    )
    .await;
    assert!(
        survey_identities(&after_drop)
            .iter()
            .any(|identity| identity == dropped_key),
        "exact dropped identity must become eligible again"
    );
    let _drop_for_partial = observe(
        client,
        "ministr_dropped",
        json!({"identities": [dropped_identity]}),
    )
    .await;
}

async fn assert_partial_fanout(client: &McpClient) {
    let partial = observe(
        client,
        "ministr_survey",
        json!({
            "corpus_ids": ["secondary", "missing"],
            "query": "JWT authentication tokens",
            "top_k": 5
        }),
    )
    .await;
    assert_eq!(partial.payload["status"], "partial");
    assert!(
        !result_array(&partial, "results").is_empty(),
        "successful linked-corpus data must survive partial fan-out"
    );
    let corpora = partial.payload["corpora"]
        .as_array()
        .expect("partial response has per-corpus status");
    let secondary = corpora
        .iter()
        .find(|corpus| corpus["corpus_id"] == "secondary")
        .expect("successful secondary corpus status is preserved");
    assert_ne!(secondary["status"], "error");
    assert_eq!(
        secondary.pointer("/completeness/completeness"),
        Some(&Value::String("stale".into()))
    );
    assert_eq!(
        secondary.pointer("/completeness/absence_is_conclusive"),
        Some(&Value::Bool(false))
    );
    assert!(
        corpora
            .iter()
            .any(|corpus| { corpus["corpus_id"] == "missing" && corpus["status"] == "error" })
    );
}

#[tokio::test]
async fn live_daemon_multi_corpus_collision_and_partial_fanout() {
    let (client, _server, _primary, _secondary) = daemon_multi_client().await;
    let primary_results = observe(
        &client,
        "ministr_survey",
        json!({"query": "JWT authentication tokens", "top_k": 5}),
    )
    .await;
    let secondary_results = observe(
        &client,
        "ministr_survey",
        json!({
            "project": "secondary",
            "query": "JWT authentication tokens",
            "top_k": 5
        }),
    )
    .await;
    let (secondary_ids, dropped_identity, dropped_key) =
        assert_cross_corpus_collision(&primary_results, &secondary_results);
    assert_linked_dedup_drop_cycle(&client, &secondary_ids, &dropped_identity, &dropped_key).await;
    assert_partial_fanout(&client).await;
}

#[tokio::test]
async fn live_active_indexing_negative_is_not_conclusive() {
    let server = setup_server().await;
    let progress = server.ingestion_progress_arc();
    progress.start(10);
    progress.increment_done();
    progress.increment_done();
    let (client, _server) = wrap_as_client(server).await;
    let response = observe(
        &client,
        "ministr_symbols",
        json!({"query": "definitely_missing_symbol_name", "limit": 10}),
    )
    .await;

    assert!(result_array(&response, "symbols").is_empty());
    assert_eq!(response.payload["status"], "partial");
    assert_eq!(
        response.payload.pointer("/completeness/completeness"),
        Some(&Value::String("partial".into()))
    );
    assert_eq!(
        response
            .payload
            .pointer("/completeness/absence_is_conclusive"),
        Some(&Value::Bool(false))
    );
    assert_eq!(response.payload["indexing_in_progress"], true);
}
