//! End-to-end tests for the `ministr_run` exec tool family
//! (exec-mcp-tools): tools are wired through the MCP protocol layer,
//! the digest comes back structured, log paging never re-sends, and the
//! background lifecycle (start → status → kill) works.
//!
//! Unix-gated: the commands under test use `sh`.
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::service::QueryService;
use ministr_core::session::UsageConfig;
use ministr_core::storage::{SqliteStorage, Storage as _};
use ministr_mcp::server::MinistrServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
use serde_json::json;

/// Deterministic, text-sensitive mock embedder (byte-distribution
/// vectors) — distinct texts get distinct directions, so survey ranking
/// over ingested run reports is meaningful.
struct MockEmbedder;

const MOCK_DIM: usize = 16;

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; MOCK_DIM];
                for (i, b) in t.bytes().enumerate() {
                    v[i % MOCK_DIM] += f32::from(b) / 255.0;
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
        MOCK_DIM
    }
}

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServerHandle = rmcp::service::RunningService<rmcp::RoleServer, MinistrServer>;

/// Minimal server with the exec tools wired to a temp workspace + store.
fn setup_exec_server(workdir: &std::path::Path) -> MinistrServer {
    let storage = SqliteStorage::open_in_memory().expect("storage");
    let index = Arc::new(HnswIndex::new(MOCK_DIM, 100).expect("index"));
    let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
    let service = Arc::new(QueryService::new(storage, embedder, index));
    let server = MinistrServer::new(service);
    server.set_exec_roots(vec![workdir.to_path_buf()]);
    server.set_exec_db_path(workdir.join("exec_runs.db"));
    server
}

/// Server with run-log intelligence wired: persistent-session storage +
/// runtime ingest (embedder + index shared with the query service), so
/// finished runs become searchable corpus documents. Returns the storage
/// handle for direct corpus assertions.
async fn setup_intel_server(workdir: &std::path::Path) -> (MinistrServer, Arc<SqliteStorage>) {
    let storage = Arc::new(SqliteStorage::open_in_memory().expect("storage"));
    let index = Arc::new(HnswIndex::new(MOCK_DIM, 1000).expect("index"));
    let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
    let service = Arc::new(QueryService::new(
        (*storage).clone(),
        Arc::clone(&embedder),
        Arc::clone(&index) as Arc<dyn ministr_core::index::VectorIndex>,
    ));
    let server = MinistrServer::with_persistence(
        service,
        UsageConfig::default(),
        Arc::clone(&storage),
        None,
    )
    .await
    .with_runtime_ingest(embedder, index);
    server.set_exec_roots(vec![workdir.to_path_buf()]);
    server.set_exec_db_path(workdir.join("exec_runs.db"));
    (server, storage)
}

async fn wrap_as_client(server: MinistrServer) -> (McpClient, McpServerHandle) {
    let (c2s_w, c2s_r) = tokio::io::duplex(65_536);
    let (s2c_w, s2c_r) = tokio::io::duplex(65_536);
    let server_task = tokio::spawn(async move { server.serve((c2s_r, s2c_w)).await.unwrap() });
    let client = ().serve((s2c_r, c2s_w)).await.unwrap();
    let server_handle = server_task.await.unwrap();
    (client, server_handle)
}

async fn call_tool(client: &McpClient, name: &str, args: serde_json::Value) -> CallToolResult {
    let arguments = args
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(args) = arguments {
        params = params.with_arguments(args);
    }
    client.peer().call_tool(params).await.unwrap()
}

fn extract_text(content: &[Content]) -> &str {
    content[0]
        .raw
        .as_text()
        .expect("expected text content")
        .text
        .as_str()
}

fn tool_result(result: &CallToolResult) -> serde_json::Value {
    serde_json::from_str(extract_text(&result.content)).expect("json response")
}

#[tokio::test]
async fn run_executes_and_returns_digest_with_error_lines() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path())).await;

    let result = call_tool(
        &client,
        "ministr_run",
        json!({"command": "echo build-ok; echo 'error[E0308]: mismatched types' 1>&2; exit 1"}),
    )
    .await;
    assert_eq!(result.is_error, Some(false));
    let tr = tool_result(&result);
    assert_eq!(tr["status"], "exited");
    assert_eq!(tr["exit_code"], 1);
    assert!(tr["run_id"].as_str().unwrap().starts_with("run-"));
    let diagnostics = tr["digest"]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|l| l.as_str().unwrap().contains("error[E0308]")),
        "stderr error line must surface in the digest: {diagnostics:?}"
    );
    assert!(
        tr["digest"]["window"]
            .as_str()
            .unwrap()
            .contains("build-ok"),
        "stdout must be in the window"
    );
}

#[tokio::test]
async fn run_logs_delta_never_resends_already_delivered_spans() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path())).await;

    let run = call_tool(
        &client,
        "ministr_run",
        json!({"command": "i=0; while [ $i -lt 400 ]; do echo entry-$i; i=$((i+1)); done"}),
    )
    .await;
    let run_id = tool_result(&run)["run_id"].as_str().unwrap().to_string();

    let page1 = tool_result(
        &call_tool(
            &client,
            "ministr_run_logs",
            json!({"run_id": run_id, "max_bytes": 512}),
        )
        .await,
    );
    let chunk1 = page1["chunk"].as_str().unwrap().to_string();
    assert!(chunk1.contains("entry-0"), "first page starts at the top");

    let page2 = tool_result(
        &call_tool(
            &client,
            "ministr_run_logs",
            json!({"run_id": run_id, "max_bytes": 512}),
        )
        .await,
    );
    let chunk2 = page2["chunk"].as_str().unwrap();
    let last_line_of_first = chunk1.lines().last().unwrap();
    assert!(
        !chunk2.contains(last_line_of_first),
        "second page must not re-send the first page's content"
    );
    assert!(!chunk2.is_empty(), "second page delivers NEW content");

    // Query mode searches the whole log without touching the cursor.
    let search = tool_result(
        &call_tool(
            &client,
            "ministr_run_logs",
            json!({"run_id": run_id, "query": "entry-399"}),
        )
        .await,
    );
    assert_eq!(search["matched_lines"], 1);
    assert!(search["chunk"].as_str().unwrap().contains("entry-399"));
}

#[tokio::test]
async fn background_lifecycle_start_status_kill() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path())).await;

    let started = tool_result(
        &call_tool(
            &client,
            "ministr_run",
            json!({"command": "sleep 30", "background": true}),
        )
        .await,
    );
    assert_eq!(started["status"], "running");
    let run_id = started["run_id"].as_str().unwrap().to_string();

    let status =
        tool_result(&call_tool(&client, "ministr_run_status", json!({"run_id": run_id})).await);
    assert_eq!(status["status"], "running");

    let killed =
        tool_result(&call_tool(&client, "ministr_run_kill", json!({"run_id": run_id})).await);
    assert_eq!(killed["killed"], true);

    // The record finalizes as killed.
    let mut final_status = String::new();
    for _ in 0..200 {
        let status =
            tool_result(&call_tool(&client, "ministr_run_status", json!({"run_id": run_id})).await);
        if status["status"] != "running" {
            final_status = status["status"].as_str().unwrap().to_string();
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(final_status, "killed");
}

#[tokio::test]
async fn exec_tool_schemas_stay_inside_the_token_budget() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path())).await;

    let tools = client.peer().list_all_tools().await.expect("list tools");
    let exec_tools: Vec<_> = tools
        .iter()
        .filter(|t| t.name.starts_with("ministr_run"))
        .collect();
    assert_eq!(
        exec_tools.len(),
        4,
        "ministr_run / _logs / _status / _kill must all be registered"
    );

    // <2k tokens for the whole family (≈4 chars/token ⇒ 8000 chars),
    // counting what actually lands in the agent's context: name +
    // description + input schema.
    let total_chars: usize = exec_tools
        .iter()
        .map(|t| {
            t.name.len()
                + t.description.as_deref().map_or(0, str::len)
                + serde_json::to_string(&t.input_schema)
                    .expect("schema json")
                    .len()
        })
        .sum();
    assert!(
        total_chars < 8000,
        "exec tool schemas must stay under ~2k tokens (got {total_chars} chars)"
    );
}

#[tokio::test]
async fn run_outside_roots_is_policy_denied() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside");
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path())).await;

    let result = call_tool(
        &client,
        "ministr_run",
        json!({"command": "echo never", "cwd": outside.path().to_string_lossy()}),
    )
    .await;
    let rendered = extract_text(&result.content);
    assert!(
        rendered.contains("exec policy") || rendered.contains("exec_failed"),
        "out-of-root cwd must be denied: {rendered}"
    );
}

// ─── RUN-LOG INTELLIGENCE (exec-log-intelligence) ───────────────────────────

#[tokio::test]
async fn failing_run_is_retrievable_via_survey_by_its_error_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (server, storage) = setup_intel_server(tmp.path()).await;
    let (client, _server) = wrap_as_client(server).await;

    // Plant a failing run with a distinctive error token.
    let run = tool_result(
        &call_tool(
            &client,
            "ministr_run",
            json!({"command": "echo 'error: frobnicator_zx81 panicked at widget.rs:42' 1>&2; exit 3"}),
        )
        .await,
    );
    assert_eq!(run["exit_code"], 3);

    // The run report landed in the corpus under exec-runs/.
    let docs = storage.list_documents().await.expect("list");
    assert!(
        docs.iter()
            .any(|d| d.source_path.starts_with("exec-runs/run-")),
        "run report must be ingested: {:?}",
        docs.iter().map(|d| &d.source_path).collect::<Vec<_>>()
    );

    // And ministr_survey retrieves it by the error text.
    let survey = call_tool(
        &client,
        "ministr_survey",
        json!({"query": "frobnicator_zx81 panicked"}),
    )
    .await;
    let rendered = extract_text(&survey.content);
    assert!(
        rendered.contains("exec-runs/run-") && rendered.contains("frobnicator_zx81"),
        "survey must surface the run report for its own error text: {rendered}"
    );
}

#[tokio::test]
async fn repeated_diagnostics_dedup_with_occurrence_counts_in_the_corpus() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (server, storage) = setup_intel_server(tmp.path()).await;
    let (client, _server) = wrap_as_client(server).await;

    let run = tool_result(
        &call_tool(
            &client,
            "ministr_run",
            json!({"command": "i=0; while [ $i -lt 30 ]; do echo 'error: quux gasket misaligned' 1>&2; i=$((i+1)); done; exit 1"}),
        )
        .await,
    );
    // The digest already collapses the repeats…
    let diags = run["digest"]["diagnostics"].as_array().unwrap();
    assert!(
        diags
            .iter()
            .any(|l| l.as_str().unwrap().starts_with("30× error: quux gasket")),
        "digest must collapse 30 identical lines to one counted line: {diags:?}"
    );

    // …and the INGESTED document carries the same collapsed form.
    let docs = storage.list_documents().await.expect("list");
    let doc = docs
        .iter()
        .find(|d| d.source_path.starts_with("exec-runs/run-"))
        .expect("ingested run report");
    let sections = storage.list_sections(&doc.id).await.expect("sections");
    assert!(
        sections
            .iter()
            .any(|s| s.text.contains("30× error: quux gasket")),
        "corpus section must keep the deduped count, not 30 raw lines"
    );
}

#[tokio::test]
async fn retention_sweep_keeps_only_the_newest_reports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (server, storage) = setup_intel_server(tmp.path()).await;
    server.set_exec_run_retention(3);
    let (client, _server) = wrap_as_client(server).await;

    for i in 0..6 {
        let run = tool_result(
            &call_tool(
                &client,
                "ministr_run",
                json!({"command": format!("echo soak-run-{i}")}),
            )
            .await,
        );
        assert_eq!(run["exit_code"], 0, "soak run {i} must succeed");
    }

    let docs = storage.list_documents().await.expect("list");
    let mut run_docs: Vec<_> = docs
        .iter()
        .filter(|d| d.source_path.starts_with("exec-runs/run-"))
        .map(|d| d.source_path.clone())
        .collect();
    run_docs.sort();
    assert_eq!(
        run_docs.len(),
        3,
        "retention cap of 3 must hold after 6 runs: {run_docs:?}"
    );

    // The survivors are the NEWEST three (run ids embed spawn timestamps,
    // so lexicographic order is chronological).
    let all_sections_text: String = {
        let mut out = String::new();
        for d in docs
            .iter()
            .filter(|d| d.source_path.starts_with("exec-runs/"))
        {
            for s in storage.list_sections(&d.id).await.expect("sections") {
                out.push_str(&s.text);
            }
        }
        out
    };
    assert!(
        all_sections_text.contains("soak-run-5"),
        "newest run must survive the sweep"
    );
    assert!(
        !all_sections_text.contains("soak-run-0"),
        "oldest run must be swept"
    );
}
