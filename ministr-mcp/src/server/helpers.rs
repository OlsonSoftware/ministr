//! Standalone helper functions used by the ministr MCP server.
//!
//! These are pure functions with no `self` receiver — they operate on
//! their arguments and return results. Extracted from the server module
//! to keep handler code focused on MCP protocol logic.

use std::collections::HashMap;
use std::path::PathBuf;

use ministr_core::service::QueryError;
use ministr_core::types::Resolution;
use rmcp::handler::server::tool::ToolRouter;

use super::MinistrServer;

/// Maximum serialized response size in bytes before the guard injects a warning.
pub(crate) const MAX_RESPONSE_BYTES: usize = 100_000;

/// Maximum number of survey results to prefetch via agent intent prediction.
pub(crate) const MAX_INTENT_PREFETCH_SURVEY: usize = 5;

/// Well-known progress token for ministr ingestion notifications.
pub(crate) const INGESTION_PROGRESS_TOKEN: &str = "ministr/ingestion";

/// Compute a 64-char BLAKE3 hex digest of content for change detection.
pub(crate) fn content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// Parse a resolution string back to the enum.
pub(crate) fn parse_resolution(s: &str) -> Resolution {
    match s {
        "summary" => Resolution::Summary,
        "claim" => Resolution::Claim,
        _ => Resolution::Section,
    }
}

/// Convert elapsed duration to milliseconds, saturating at `u64::MAX`.
pub(crate) fn elapsed_millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Extract a human-readable display name from a repository URL.
///
/// Strips the host prefix and `.git` suffix to produce e.g. `"owner/repo"`.
pub(crate) fn repo_display_name(repo_url: &str) -> String {
    let name = repo_url
        .rsplit_once("://")
        .map_or(repo_url, |(_, rest)| rest);
    let name = name.strip_prefix("github.com/").unwrap_or(name);
    let name = name.strip_prefix("gitlab.com/").unwrap_or(name);
    name.strip_suffix(".git").unwrap_or(name).to_string()
}

/// Compute language statistics from a list of file paths.
pub(crate) fn compute_language_stats(files: &[PathBuf]) -> HashMap<String, usize> {
    let mut stats = HashMap::new();
    for file in files {
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = match ext {
            "rs" => "rust",
            "py" => "python",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" => "javascript",
            "go" => "go",
            "rb" => "ruby",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "cxx" | "cc" | "hpp" => "cpp",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "json" => "json",
            "md" => "markdown",
            other if !other.is_empty() => other,
            _ => continue,
        };
        *stats.entry(lang.to_string()).or_insert(0) += 1;
    }
    stats
}

/// Generate a simple UUID v4-style session ID.
pub(crate) fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("sess-{}-{}", now.as_secs(), now.subsec_nanos())
}

/// Format a [`QueryError`] into a user-friendly error message for MCP tool responses.
///
/// Produces structured messages that help the agent understand what went wrong
/// and how to recover, rather than exposing raw internal error strings.
pub(crate) fn format_query_error(err: &QueryError) -> String {
    match err {
        QueryError::SectionNotFound { id } => {
            format!(
                "Section not found: '{id}'. Check the section ID format \
                 (e.g. 'docs/auth.md#tokens') and use ministr_survey to discover valid IDs."
            )
        }
        QueryError::Index(index_err) => {
            format!(
                "Search index error: {index_err}. The index may need to be rebuilt. \
                 Try a different query or check server logs for details."
            )
        }
        QueryError::Storage(storage_err) => {
            format!(
                "Storage error: {storage_err}. The corpus database may be unavailable. \
                 Check server logs for details."
            )
        }
        QueryError::ClaimNotFound { id } => {
            format!(
                "Claim not found: '{id}'. Use ministr_extract to discover valid claim IDs \
                 within a section."
            )
        }
        QueryError::SymbolNotFound { id } => {
            format!("Symbol not found: '{id}'. Use ministr_symbols to search for valid symbol IDs.")
        }
    }
}

/// Check whether a directory tree contains any code files (by extension).
///
/// Uses a bounded BFS (max depth 6, max 500 entries) to keep this fast.
/// Returns `true` as soon as a file with a known code extension is found.
pub(crate) fn has_code_files_in_dir(root: &std::path::Path) -> bool {
    use ministr_core::code::grammar::ALL_CODE_EXTENSIONS;
    use std::collections::VecDeque;

    const SKIP_DIRS: &[&str] = &[
        "node_modules",
        "target",
        "__pycache__",
        "vendor",
        ".git",
        ".hg",
        "dist",
        "build",
    ];

    if !root.is_dir() {
        return false;
    }

    let mut queue: VecDeque<(PathBuf, u8)> = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0));
    let mut checked = 0u32;

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_str().unwrap_or("");

            if path.is_dir() {
                if depth < 6 && !name_str.starts_with('.') && !SKIP_DIRS.contains(&name_str) {
                    queue.push_back((path, depth + 1));
                }
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str())
                    && ALL_CODE_EXTENSIONS.contains(&ext)
                {
                    return true;
                }
                checked += 1;
                if checked >= 500 {
                    return false;
                }
            }
        }
    }
    false
}

/// Build the dynamic instructions string based on which tools are registered.
pub(crate) fn build_instructions(router: &ToolRouter<MinistrServer>) -> String {
    // Map of tool name → description fragment for the instructions string
    let tool_descriptions: &[(&str, &str)] = &[
        (
            "ministr_toc",
            "ministr_toc to get a structural overview of the indexed corpus",
        ),
        (
            "ministr_survey",
            "ministr_survey to search for relevant content",
        ),
        ("ministr_read", "ministr_read to retrieve full section text"),
        (
            "ministr_extract",
            "ministr_extract to get atomic claims from a section",
        ),
        (
            "ministr_related",
            "ministr_related to follow dependency chains between claims",
        ),
        // ministr_budget is intentionally not advertised here. It remains
        // callable for deliberate use, but surfacing it in the agent
        // instructions made agents proactively "check budget" and then
        // wrongly conclude they were almost out of context. Context
        // pressure is tracked internally for compression/dedup; it is no
        // longer pushed at the agent.
        (
            "ministr_compress",
            "ministr_compress to generate compressed summaries of content you want to evict",
        ),
        (
            "ministr_evicted",
            "ministr_evicted to signal when content has been dropped from your context window",
        ),
        (
            "ministr_fetch",
            "ministr_fetch to fetch web content by URL and add it to the corpus",
        ),
        (
            "ministr_refresh",
            "ministr_refresh to check cached web sources for staleness and re-fetch changed content",
        ),
        (
            "ministr_clone",
            "ministr_clone to clone a git repository and index its content",
        ),
        (
            "ministr_task",
            "ministr_task to poll background fetch/clone tasks (deprecated — prefer MCP tasks/get)",
        ),
        (
            "ministr_symbols",
            "ministr_symbols to search the code symbol index",
        ),
        (
            "ministr_definition",
            "ministr_definition to get the full source definition of a symbol",
        ),
        (
            "ministr_references",
            "ministr_references to find all references to a symbol",
        ),
        (
            "ministr_bridge",
            "ministr_bridge to query cross-language bridge links",
        ),
    ];

    let mut parts: Vec<&str> = Vec::new();
    for (name, desc) in tool_descriptions {
        if router.has_route(name) {
            parts.push(desc);
        }
    }

    format!(
        "ministr is a context cache for LLM agents. Use {}.",
        parts.join(", "),
    )
}

/// Serialize a value into a `CallToolResult` with structured content.
///
/// Sets both `structured_content` (JSON object) and `content` (text fallback)
/// for backward compatibility with clients that don't support structured output.
///
/// Includes a response size guard: if the serialized JSON exceeds
/// [`MAX_RESPONSE_BYTES`], a `_truncation_warning` is injected into the
/// response object advising the caller to use pagination parameters.
pub(crate) fn structured_result(
    value: &impl serde::Serialize,
) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
    let v = serde_json::to_value(value).map_err(|e| {
        rmcp::model::ErrorData::internal_error(format!("serialization failed: {e}"), None)
    })?;

    let v = apply_response_size_guard(v);

    Ok(rmcp::model::CallToolResult::structured(v))
}

/// If the serialized JSON exceeds [`MAX_RESPONSE_BYTES`], inject a
/// `_truncation_warning` field advising the caller to paginate.
pub(crate) fn apply_response_size_guard(mut v: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = v.as_object_mut() {
        let size = serde_json::to_string(obj).map_or(0, |s| s.len());
        if size > MAX_RESPONSE_BYTES {
            obj.insert(
                "_truncation_warning".to_string(),
                serde_json::json!({
                    "message": "Response exceeds size threshold. Use offset/limit parameters to paginate.",
                    "response_bytes": size,
                    "threshold_bytes": MAX_RESPONSE_BYTES,
                }),
            );
        }
    }
    v
}
