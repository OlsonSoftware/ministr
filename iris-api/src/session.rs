//! Session management API types.
//!
//! Wire types for creating, querying, and destroying daemon-side sessions.
//! Sessions track delivered content, enabling deduplication, delta delivery,
//! and token budget management across MCP proxy reconnections.

use serde::{Deserialize, Serialize};

/// Request to create a new session for a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionRequest {
    /// Maximum context budget in tokens (default: 100 000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<usize>,
}

/// Response after creating a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CreateSessionResponse {
    /// Unique session identifier (use in subsequent query requests).
    pub session_id: String,
}

/// Budget status snapshot for a session.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionBudgetResponse {
    /// Pressure level: `"normal"`, `"elevated"`, or `"critical"`.
    pub pressure_level: String,
    /// Estimated tokens consumed by delivered content.
    pub tokens_used: usize,
    /// Estimated tokens remaining before budget pressure.
    pub tokens_remaining: usize,
    /// Utilization ratio (0.0–1.0).
    pub utilization: f64,
}
