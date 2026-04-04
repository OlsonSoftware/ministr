//! Agent-to-Agent (A2A) protocol support for iris.
//!
//! Implements the A2A protocol (Google/Linux Foundation) so other AI agents
//! can discover iris's code intelligence capabilities via an Agent Card and
//! submit tasks via HTTP endpoints.
//!
//! # Endpoints
//!
//! - `GET /.well-known/agent.json` — public Agent Card for capability discovery
//! - `POST /a2a/tasks` — submit a code intelligence task
//! - `GET /a2a/tasks/{id}` — retrieve task state and result

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use iris_core::service::QueryService;
use iris_core::session::SessionRegistry;

// ---------------------------------------------------------------------------
// Agent Card types
// ---------------------------------------------------------------------------

/// A2A Agent Card describing iris's capabilities.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Agent name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Project URL.
    pub url: String,
    /// Agent version.
    pub version: String,
    /// Protocol version supported.
    pub protocol_version: String,
    /// Supported capabilities.
    pub capabilities: AgentCapabilities,
    /// Skills this agent can perform.
    pub skills: Vec<AgentSkill>,
    /// Default input content modes.
    pub default_input_modes: Vec<String>,
    /// Default output content modes.
    pub default_output_modes: Vec<String>,
}

/// A2A capability flags.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses.
    pub streaming: bool,
    /// Whether the agent supports push notifications.
    pub push_notifications: bool,
}

/// A skill the agent can perform.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// Unique skill identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of what this skill does.
    pub description: String,
    /// Example input texts for this skill.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// Build the iris Agent Card.
#[must_use]
pub fn iris_agent_card() -> AgentCard {
    AgentCard {
        name: "iris".to_string(),
        description: "Code intelligence MCP server with semantic search, symbol navigation, \
                      cross-language bridge tracing, and context-aware budget management"
            .to_string(),
        url: "https://github.com/anthropics/iris-rs".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: "0.2.1".to_string(),
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
        },
        skills: vec![
            AgentSkill {
                id: "code_survey".to_string(),
                name: "Semantic Code Search".to_string(),
                description: "Search code and documentation by natural language query. \
                             Returns ranked results across sections, symbols, and claims."
                    .to_string(),
                examples: vec![
                    "Find authentication middleware".to_string(),
                    "How does the session manager work?".to_string(),
                ],
            },
            AgentSkill {
                id: "code_symbols".to_string(),
                name: "Symbol Navigation".to_string(),
                description: "Find structs, functions, traits, and enums by name, kind, or module."
                    .to_string(),
                examples: vec![
                    "Find all structs in the auth module".to_string(),
                    "Look up the QueryService struct".to_string(),
                ],
            },
            AgentSkill {
                id: "code_references".to_string(),
                name: "Reference Finder".to_string(),
                description: "Find all callers, implementors, and importers of a given symbol."
                    .to_string(),
                examples: vec!["Who calls the survey() function?".to_string()],
            },
            AgentSkill {
                id: "code_bridge".to_string(),
                name: "Cross-Language Bridge Tracer".to_string(),
                description:
                    "Trace FFI, IPC, and API links across language boundaries (Rust↔TypeScript, \
                     Rust↔Python, etc.)."
                        .to_string(),
                examples: vec![
                    "Find all Tauri command bridges".to_string(),
                    "Trace NAPI bindings".to_string(),
                ],
            },
        ],
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
    }
}

// ---------------------------------------------------------------------------
// Task types
// ---------------------------------------------------------------------------

/// A2A task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    /// Task is queued and awaiting processing.
    Pending,
    /// Task is currently being processed.
    Working,
    /// Task completed successfully.
    Completed,
    /// Task failed.
    Failed,
}

/// A2A message part.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    /// Text content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Structured JSON content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
}

/// A2A message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aMessage {
    /// Message role: "user" or "agent".
    pub role: String,
    /// Message content parts.
    pub parts: Vec<MessagePart>,
}

/// A2A task.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2aTask {
    /// Unique task ID.
    pub id: String,
    /// Current task state.
    pub state: TaskState,
    /// The original request message.
    pub message: A2aMessage,
    /// The result message (populated when completed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<A2aMessage>,
    /// Error description if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request body for submitting an A2A task.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    /// The skill to invoke.
    pub skill_id: String,
    /// The input message.
    pub message: A2aMessage,
}

// ---------------------------------------------------------------------------
// Shared state for A2A handlers
// ---------------------------------------------------------------------------

/// Shared state passed to A2A axum handlers.
#[derive(Clone)]
pub struct A2aState {
    /// The query service for executing code intelligence operations.
    pub service: Arc<QueryService>,
    /// Session registry for multi-agent session sharing.
    pub registry: Arc<Mutex<SessionRegistry>>,
    /// In-memory task store.
    pub tasks: Arc<Mutex<HashMap<String, A2aTask>>>,
}

// ---------------------------------------------------------------------------
// Axum route builder
// ---------------------------------------------------------------------------

/// Build the axum router for A2A endpoints.
///
/// Mounts:
/// - `GET /.well-known/agent.json` — public Agent Card
/// - `POST /a2a/tasks` — submit a task
/// - `GET /a2a/tasks/:id` — get task state
pub fn a2a_routes(state: A2aState) -> Router {
    Router::new()
        .route("/.well-known/agent.json", get(agent_card_handler))
        .route("/a2a/tasks", post(send_message_handler))
        .route("/a2a/tasks/{id}", get(get_task_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Return the iris Agent Card.
async fn agent_card_handler() -> impl IntoResponse {
    Json(iris_agent_card())
}

/// Accept an A2A task, dispatch to the appropriate `QueryService` method.
async fn send_message_handler(
    State(state): State<A2aState>,
    Json(req): Json<SendMessageRequest>,
) -> impl IntoResponse {
    let task_id = uuid_v4();
    let mut task = A2aTask {
        id: task_id.clone(),
        state: TaskState::Working,
        message: req.message.clone(),
        result: None,
        error: None,
    };

    // Extract the query text from the first text part.
    let query = req
        .message
        .parts
        .iter()
        .find_map(|p| p.text.as_deref())
        .unwrap_or("")
        .to_string();

    // Dispatch based on skill ID.
    let result = match req.skill_id.as_str() {
        "code_survey" => dispatch_survey(&state.service, &query).await,
        "code_symbols" => dispatch_symbols(&state.service, &query).await,
        "code_references" => dispatch_references(&state.service, &query).await,
        "code_bridge" => dispatch_bridge(&state.service, &query).await,
        unknown => Err(format!("unknown skill: {unknown}")),
    };

    match result {
        Ok(response_json) => {
            task.state = TaskState::Completed;
            task.result = Some(A2aMessage {
                role: "agent".to_string(),
                parts: vec![MessagePart {
                    text: None,
                    json: Some(response_json),
                }],
            });
        }
        Err(err) => {
            task.state = TaskState::Failed;
            task.error = Some(err);
        }
    }

    let mut tasks = state.tasks.lock().await;
    tasks.insert(task_id, task.clone());

    (StatusCode::OK, Json(task))
}

/// Retrieve an existing task by ID.
async fn get_task_handler(
    State(state): State<A2aState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let tasks = state.tasks.lock().await;
    match tasks.get(&id) {
        Some(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(task).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "task not found", "taskId": id })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Skill dispatchers
// ---------------------------------------------------------------------------

async fn dispatch_survey(service: &QueryService, query: &str) -> Result<serde_json::Value, String> {
    let results = service.survey(query, 10).await.map_err(|e| e.to_string())?;
    serde_json::to_value(&results).map_err(|e| e.to_string())
}

async fn dispatch_symbols(
    service: &QueryService,
    query: &str,
) -> Result<serde_json::Value, String> {
    let filter = iris_core::storage::SymbolFilter {
        name: Some(query.to_string()),
        name_exact: None,
        kind: None,
        visibility: None,
        module: None,
        file_path: None,
    };
    let results = service
        .search_symbols(&filter)
        .await
        .map_err(|e| e.to_string())?;
    // SymbolRecord doesn't derive Serialize — convert to JSON manually
    let symbols: Vec<serde_json::Value> = results
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id.0,
                "name": s.name,
                "kind": s.kind,
                "module": s.module_path,
                "file_path": s.file_path,
                "visibility": s.visibility,
            })
        })
        .collect();
    Ok(serde_json::json!({ "symbols": symbols }))
}

async fn dispatch_references(
    service: &QueryService,
    symbol_id: &str,
) -> Result<serde_json::Value, String> {
    let results = service
        .get_symbol_references(symbol_id, None)
        .await
        .map_err(|e| e.to_string())?;
    // SymbolRefResult doesn't derive Serialize — convert manually
    let refs: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "from_symbol": r.from_name,
                "from_file": r.from_file,
                "from_line": r.from_line,
                "to_symbol": r.to_name,
                "to_file": r.to_file,
                "to_line": r.to_line,
                "ref_kind": r.ref_kind,
            })
        })
        .collect();
    Ok(serde_json::json!({ "references": refs }))
}

async fn dispatch_bridge(service: &QueryService, query: &str) -> Result<serde_json::Value, String> {
    let results = service
        .query_bridges(Some(query), None, None, None)
        .await
        .map_err(|e| e.to_string())?;
    // BridgeLinkDetail doesn't derive Serialize — convert manually
    let links: Vec<serde_json::Value> = results
        .iter()
        .map(|l| {
            serde_json::json!({
                "kind": l.kind,
                "export_file": l.export_file,
                "export_symbol": l.export_symbol,
                "export_language": l.export_language,
                "import_file": l.import_file,
                "import_symbol": l.import_symbol,
                "import_language": l.import_language,
                "confidence": l.confidence,
            })
        })
        .collect();
    Ok(serde_json::json!({ "bridges": links }))
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Generate a simple UUID v4 without external dependency.
#[allow(clippy::cast_possible_truncation)]
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seed = now.as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (seed & 0xFFFF_FFFF) as u32,
        ((seed >> 32) & 0xFFFF) as u16,
        ((seed >> 48) & 0x0FFF) as u16,
        (((seed >> 60) & 0x3F) | 0x80) as u16,
        (seed.wrapping_mul(6_364_136_223_846_793_005)) & 0xFFFF_FFFF_FFFF
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_card_has_required_fields() {
        let card = iris_agent_card();
        assert_eq!(card.name, "iris");
        assert!(!card.description.is_empty());
        assert!(!card.version.is_empty());
        assert_eq!(card.protocol_version, "0.2.1");
        assert!(!card.skills.is_empty());
    }

    #[test]
    fn agent_card_serializes_to_valid_json() {
        let card = iris_agent_card();
        let json = serde_json::to_value(&card).unwrap();

        assert_eq!(json["name"], "iris");
        assert!(json["capabilities"]["streaming"].is_boolean());
        assert!(json["skills"].is_array());
        assert_eq!(json["skills"].as_array().unwrap().len(), 4);

        // Verify camelCase serialization
        assert!(json["protocolVersion"].is_string());
        assert!(json["defaultInputModes"].is_array());
        assert!(json["defaultOutputModes"].is_array());
    }

    #[test]
    fn agent_card_skills_have_ids() {
        let card = iris_agent_card();
        let ids: Vec<&str> = card.skills.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"code_survey"));
        assert!(ids.contains(&"code_symbols"));
        assert!(ids.contains(&"code_references"));
        assert!(ids.contains(&"code_bridge"));
    }

    #[test]
    fn task_state_serializes_lowercase() {
        let json = serde_json::to_string(&TaskState::Completed).unwrap();
        assert_eq!(json, "\"completed\"");
        let json = serde_json::to_string(&TaskState::Failed).unwrap();
        assert_eq!(json, "\"failed\"");
    }

    #[test]
    fn send_message_request_deserializes() {
        let json = r#"{
            "skillId": "code_survey",
            "message": {
                "role": "user",
                "parts": [{"text": "find authentication code"}]
            }
        }"#;
        let req: SendMessageRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.skill_id, "code_survey");
        assert_eq!(req.message.role, "user");
        assert_eq!(
            req.message.parts[0].text.as_deref(),
            Some("find authentication code")
        );
    }

    #[test]
    fn a2a_task_serializes_with_camel_case() {
        let task = A2aTask {
            id: "test-123".to_string(),
            state: TaskState::Completed,
            message: A2aMessage {
                role: "user".to_string(),
                parts: vec![MessagePart {
                    text: Some("query".to_string()),
                    json: None,
                }],
            },
            result: Some(A2aMessage {
                role: "agent".to_string(),
                parts: vec![MessagePart {
                    text: None,
                    json: Some(serde_json::json!({"results": []})),
                }],
            }),
            error: None,
        };
        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["id"], "test-123");
        assert_eq!(json["state"], "completed");
        assert!(json["error"].is_null() || !json.as_object().unwrap().contains_key("error"));
    }

    #[test]
    fn uuid_v4_has_correct_format() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36, "UUID should be 36 chars: {id}");
        assert_eq!(&id[8..9], "-");
        assert_eq!(&id[13..14], "-");
        assert_eq!(&id[14..15], "4", "version nibble should be 4");
        assert_eq!(&id[18..19], "-");
        assert_eq!(&id[23..24], "-");
    }
}
