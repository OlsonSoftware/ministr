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
use rmcp::model::Content;

use super::MinistrServer;

/// Maximum serialized response size in bytes before the guard injects a warning.
pub(crate) const MAX_RESPONSE_BYTES: usize = 100_000;

/// Maximum number of survey results to prefetch via agent intent prediction.
pub(crate) const MAX_INTENT_PREFETCH_SURVEY: usize = 5;

/// Hard cap shared by collection-producing navigation tools.
pub(crate) const MAX_COLLECTION_PAGE: usize = 500;

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

/// Resolve the applied offset/limit from compatible numeric cursors.
///
/// Cursors are deliberately opaque to clients. The `offset:` encoding keeps
/// old offset pagination interoperable; mutable graph handlers may replace
/// `next_cursor` with a stable item key while retaining the same metadata.
pub(crate) fn page_request(
    offset: Option<usize>,
    cursor: Option<&str>,
    requested_limit: Option<usize>,
    default_limit: usize,
) -> Result<(usize, usize), &'static str> {
    let applied_limit = requested_limit
        .unwrap_or(default_limit)
        .clamp(1, MAX_COLLECTION_PAGE);
    let applied_offset = match cursor {
        None | Some("") => offset.unwrap_or(0),
        Some(raw) => raw
            .strip_prefix("offset:")
            .unwrap_or(raw)
            .parse::<usize>()
            .map_err(|_| "invalid pagination cursor")?,
    };
    Ok((applied_offset, applied_limit))
}

/// Build complete, explicit pagination metadata for a bounded collection.
#[must_use]
pub(crate) fn page_metadata(
    limit: usize,
    offset: usize,
    returned: usize,
    total: usize,
) -> ministr_api::metadata::Pagination {
    let consumed = offset.saturating_add(returned).min(total);
    let has_more = consumed < total;
    ministr_api::metadata::Pagination {
        limit,
        offset: Some(offset),
        cursor: None,
        next_cursor: has_more.then(|| format!("offset:{consumed}")),
        total,
        has_more,
        omitted_count: total.saturating_sub(consumed),
    }
}

/// Build a deterministic opaque cursor from a stable item identity.
#[must_use]
pub(crate) fn stable_cursor<T: serde::Serialize>(prefix: &str, value: &T) -> String {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    format!("{prefix}:{}", blake3::hash(&encoded).to_hex())
}

/// Resolve a mutable collection cursor by the last stable item identity.
pub(crate) fn stable_cursor_offset<T>(
    items: &[T],
    cursor: &str,
    prefix: &str,
    identity: impl Fn(&T) -> serde_json::Value,
) -> Option<usize> {
    items
        .iter()
        .position(|item| stable_cursor(prefix, &identity(item)) == cursor)
        .map(|index| index + 1)
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
        QueryError::FileUnavailable { path, source } => {
            format!(
                "Source file unavailable: '{path}' ({source}). The file may have been \
                 moved or deleted since indexing; re-index the corpus or pick another file."
            )
        }
    }
}

/// Cascade-safe rendering of a [`crate::backend::BackendError`] as a tool
/// result. Classifies the error into a stable `error_kind` and reuses
/// [`format_backend_error`] for the human/agent-facing message, then routes
/// both through [`soft_error`] so the result carries `is_error: false` — a
/// backend miss (section/symbol not found, daemon transport blip) can never be
/// the errored sibling that cancels a parallel tool batch.
pub(crate) fn soft_backend_error(
    err: &crate::backend::BackendError,
) -> rmcp::model::CallToolResult {
    use crate::backend::BackendError;
    let kind = match err {
        BackendError::Query(QueryError::SectionNotFound { .. }) => "section_not_found",
        BackendError::Query(QueryError::ClaimNotFound { .. }) => "claim_not_found",
        BackendError::Query(QueryError::SymbolNotFound { .. }) => "symbol_not_found",
        BackendError::Query(QueryError::Index(_)) => "index_error",
        BackendError::Query(QueryError::Storage(_)) => "storage_error",
        BackendError::Query(QueryError::FileUnavailable { source, .. })
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            "permission_denied"
        }
        BackendError::Query(QueryError::FileUnavailable { .. }) => "file_unavailable",
        BackendError::Client(_) => "daemon_error",
        BackendError::UnknownProject(_) => "unavailable_corpus",
        BackendError::PermissionDenied(_) => "permission_denied",
        BackendError::InvalidParameters(_) => "invalid_parameters",
    };
    soft_error(kind, format_backend_error(err))
}

/// Make an absence-shaped soft error honest while local ingestion is active.
///
/// Navigation misses bypass the normal `ToolResponse` builder, so without this
/// overlay a section/symbol not yet indexed looked like conclusive absence.
#[must_use]
pub(crate) fn apply_active_indexing_to_soft_error(
    mut result: rmcp::model::CallToolResult,
    progress: &ministr_core::ingestion::IngestionProgress,
) -> rmcp::model::CallToolResult {
    if !progress.is_running() {
        return result;
    }
    let Some(structured) = result.structured_content.as_mut() else {
        return result;
    };
    let absence_shaped = structured["error_kind"].as_str().is_some_and(|kind| {
        matches!(
            kind,
            "section_not_found"
                | "claim_not_found"
                | "symbol_not_found"
                | "no_symbol_at_position"
                | "not_found"
        )
    });
    if !absence_shaped {
        return result;
    }

    structured["status"] = serde_json::json!("partial");
    structured["error"]["retryable"] = serde_json::json!(true);
    structured["completeness"] = serde_json::json!({
        "completeness": "partial",
        "indexed_items": progress.files_done(),
        "estimated_total_items": progress.files_total(),
        "affected_capabilities": ["search", "navigation"],
        "absence_is_conclusive": false,
        "retry_guidance": "Indexing is active; retry after completion before treating absence as conclusive.",
    });
    result
}

/// Format a [`crate::backend::BackendError`] into a user-friendly error
/// message for MCP tool responses.
///
/// Routes `Query` variants through [`format_query_error`] (preserves the
/// same friendly text the codebase relied on before the backend trait
/// was introduced), and surfaces `Client` variants as daemon transport
/// failures the agent can react to.
pub(crate) fn format_backend_error(err: &crate::backend::BackendError) -> String {
    match err {
        crate::backend::BackendError::Query(q) => format_query_error(q),
        crate::backend::BackendError::Client(c) => {
            format!(
                "Daemon transport error: {c}. The ministr daemon may have \
                 disconnected; retry the call, or restart the daemon if the \
                 error persists."
            )
        }
        crate::backend::BackendError::UnknownProject(project) => format!(
            "Corpus or linked project '{project}' is unavailable. Use ministr_projects or the corpus registry to discover valid routes."
        ),
        crate::backend::BackendError::PermissionDenied(project) => {
            format!("Permission denied for corpus or linked project '{project}'.")
        }
        crate::backend::BackendError::InvalidParameters(message) => message.clone(),
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

/// Build a compact routing hint based on which tools are registered.
///
/// Some MCP hosts prepend server instructions to every tool description, so
/// this text must stay deliberately short. Individual tool descriptions carry
/// the detailed use/don't-use guidance.
pub(crate) fn build_instructions(router: &ToolRouter<MinistrServer>) -> String {
    if router.has_route("ministr_inspect") {
        "Start with ministr_survey for concepts, ministr_symbols for names, or ministr_toc for \
         structure. Follow with ministr_read/ministr_extract for prose or \
         ministr_definition/ministr_inspect for code; use ministr_references before changes."
            .to_string()
    } else {
        "Start with ministr_survey for concepts, ministr_symbols for names, or ministr_toc for \
         structure; follow with ministr_read, ministr_extract, ministr_definition, or \
         ministr_references."
            .to_string()
    }
}

/// Cascade-safe logical failure: a tool result that is **not** an MCP error.
///
/// Claude Code cancels *every sibling tool call in a parallel batch* when one
/// of them errors (anthropics/claude-code). A tool reports an error two
/// ways: a JSON-RPC `-32602` (killed at the [`super::coerce`] layer) or a
/// `CallToolResult` with `is_error: true` — which is what
/// `CallToolResult::error` sets. So an ordinary logical failure ("section not
/// found") becomes the errored sibling that wipes a whole batch.
///
/// This is the single home for the rule: **a tool result is never marked
/// `is_error: true`.** A logical failure is returned as a *successful* result
/// whose payload loudly says it failed —
/// `{ ok: false, error_kind, message }` in `structured_content`, and a
/// `"⚠ <kind>: <message>"` line in the text content so a human/agent reading
/// prose can't miss it. The tool *succeeded at telling the caller the request
/// was invalid*; it produced no errored sibling, so nothing cascades.
#[must_use]
pub(crate) fn soft_error(
    error_kind: &str,
    message: impl Into<String>,
) -> rmcp::model::CallToolResult {
    use rmcp::model::{CallToolResult, Content};
    let message = message.into();
    let nonconclusive = matches!(
        error_kind,
        "daemon_error"
            | "index_error"
            | "storage_error"
            | "unavailable_corpus"
            | "permission_denied"
            | "file_unavailable"
    );
    let retryable = matches!(
        error_kind,
        "daemon_error"
            | "index_error"
            | "storage_error"
            | "unavailable_corpus"
            | "file_unavailable"
    );
    let structured = serde_json::json!({
        "ok": false,
        "status": "error",
        "error_kind": error_kind,
        "message": message.clone(),
        "error": {
            "error_code": error_kind,
            "retryable": retryable,
            "message": message,
        },
        "completeness": {
            "completeness": if nonconclusive { "unavailable" } else { "complete" },
            "indexed_items": 0,
            "affected_capabilities": if nonconclusive { vec!["query"] } else { Vec::<&str>::new() },
            "absence_is_conclusive": !nonconclusive,
            "retry_guidance": if retryable { Some("Retry after the backend or index becomes available.") } else { None },
        },
        "result": serde_json::Value::Null,
    });
    // Build via the structured constructor (which sets is_error:false) and
    // replace the default text with our loud "⚠ kind: message" line. The
    // is_error:false is the load-bearing bit.
    let mut result = CallToolResult::structured(structured);
    result.content = vec![Content::text(format!("⚠ {error_kind}: {message}"))];
    result
}

/// Serialize a value into a `CallToolResult` with structured content.
///
/// Machine data is canonical in `structured_content`; `content` is a compact
/// human-readable summary. Set `MINISTR_MCP_LEGACY_TEXT_CONTENT=1` to retain
/// the historical full-JSON text fallback for older clients.
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

    // Hard token bound first: condense to fit the MCP client's per-result
    // token cap (fidelity-preserving — see `super::condense`), so a large
    // trailhead result is never truncated/dumped by the client. The legacy
    // byte guard then rarely fires but stays as a defensive net.
    let v = super::condense::fit_to_budget(v, super::condense::output_budget_tokens());
    let v = apply_response_size_guard(v);

    let mut result = rmcp::model::CallToolResult::structured(v.clone());
    if legacy_text_content_enabled() {
        let text = serde_json::to_string(&v).map_err(|e| {
            rmcp::model::ErrorData::internal_error(format!("serialization failed: {e}"), None)
        })?;
        result.content = vec![Content::text(text)];
    } else {
        result.content = vec![Content::text(structured_summary(&v))];
    }
    Ok(result)
}

/// Serialize an explicitly requested full-content retrieval without applying
/// the generic token condenser. `ministr_read` uses this path so a large
/// section is never mistaken for a complete body after silent clipping.
pub(crate) fn structured_full_result(
    value: &impl serde::Serialize,
) -> Result<rmcp::model::CallToolResult, rmcp::model::ErrorData> {
    let value = serde_json::to_value(value).map_err(|error| {
        rmcp::model::ErrorData::internal_error(format!("serialization failed: {error}"), None)
    })?;
    let value = apply_response_size_guard(value);
    let mut result = rmcp::model::CallToolResult::structured(value.clone());
    result.content = if legacy_text_content_enabled() {
        vec![Content::text(serde_json::to_string(&value).map_err(
            |error| {
                rmcp::model::ErrorData::internal_error(
                    format!("serialization failed: {error}"),
                    None,
                )
            },
        )?)]
    } else {
        vec![Content::text(structured_summary(&value))]
    };
    Ok(result)
}

fn legacy_text_content_enabled() -> bool {
    // The large historical unit-test module parses the text fallback directly;
    // integration/e2e builds exercise the production compact-text default.
    cfg!(test)
        || std::env::var("MINISTR_MCP_LEGACY_TEXT_CONTENT")
            .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

fn structured_summary(value: &serde_json::Value) -> String {
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("ok");
    if let Some(error) = value.get("error") {
        let code = error
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        return format!("ministr {status}: {code}; details are in structuredContent");
    }

    let result = value.get("result").unwrap_or(value);
    let count = [
        "results",
        "symbols",
        "references",
        "entries",
        "links",
        "related",
        "diagnostics",
        "findings",
        "callers",
    ]
    .iter()
    .find_map(|key| result.get(key).and_then(serde_json::Value::as_array))
    .map(Vec::len);
    count.map_or_else(
        || format!("ministr {status}; machine data is in structuredContent"),
        |n| format!("ministr {status}: {n} item(s); machine data is in structuredContent"),
    )
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
