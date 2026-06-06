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
use ministr_core::storage::SqliteStorage;
use ministr_mcp::server::MinistrServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
use serde_json::json;

struct MockEmbedder;

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts.iter().map(|_| vec![0.5f32; 4]).collect())
    }

    fn dimension(&self) -> usize {
        4
    }
}

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServerHandle = rmcp::service::RunningService<rmcp::RoleServer, MinistrServer>;

/// Minimal server with the exec tools wired to a temp workspace + store.
async fn setup_exec_server(workdir: &std::path::Path) -> MinistrServer {
    let storage = SqliteStorage::open_in_memory().expect("storage");
    let index = Arc::new(HnswIndex::new(4, 100).expect("index"));
    let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
    let service = Arc::new(QueryService::new(storage, embedder, index));
    let server = MinistrServer::new(service);
    server.set_exec_roots(vec![workdir.to_path_buf()]);
    server.set_exec_db_path(workdir.join("exec_runs.db"));
    server
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
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path()).await).await;

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
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path()).await).await;

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
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path()).await).await;

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
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path()).await).await;

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
    let (client, _server) = wrap_as_client(setup_exec_server(tmp.path()).await).await;

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
