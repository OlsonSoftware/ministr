//! MCP Tasks primitive implementation (SEP-1686).
//!
//! Provides an [`McpTaskManager`] that implements the MCP task lifecycle
//! (`working` → `completed`/`failed`/`cancelled`) for long-running tool calls
//! like `ministr_fetch` and `ministr_clone`. Replaces the previous custom
//! `TaskManager` with protocol-native task management.
//!
//! Tasks are stored with their metadata ([`rmcp::model::Task`]) and result
//! payloads ([`CallToolResult`]). Completed tasks are retained for a
//! configurable duration and automatically pruned on access.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use rmcp::model::{
    CallToolResult, CancelTaskResult, GetTaskResult, Task, TaskList, TaskStatus as McpTaskStatus,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How long completed/failed/cancelled tasks are retained before pruning.
const TASK_RETENTION: Duration = Duration::from_secs(300); // 5 minutes

/// Default poll interval hint in milliseconds.
const DEFAULT_POLL_INTERVAL_MS: u64 = 2000;

/// Internal entry in the task map.
#[derive(Debug)]
struct TaskEntry {
    /// MCP `Task` metadata (id, status, timestamps, ttl, `poll_interval`).
    task: Task,
    /// The tool call result, available once the task reaches `completed`.
    result: Option<CallToolResult>,
    /// When the task reached a terminal state (for pruning).
    finished_at: Option<Instant>,
    /// Handle to the spawned tokio task, used for cancellation.
    join_handle: Option<JoinHandle<()>>,
    /// Cancellation token for graceful shutdown of the pipeline.
    cancellation_token: Option<CancellationToken>,
}

/// Manages MCP Tasks with automatic pruning of stale entries.
///
/// Thread-safe via interior mutability (`Mutex<…>`).
///
/// # Examples
///
/// ```
/// use ministr_mcp::task::McpTaskManager;
/// use rmcp::model::TaskStatus;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let mgr = McpTaskManager::new();
/// let task = mgr.create("fetching content…", None, None).await;
/// assert_eq!(task.status, TaskStatus::Working);
///
/// // Complete the task
/// let result = rmcp::model::CallToolResult::success(vec![
///     rmcp::model::Content::text("done"),
/// ]);
/// mgr.complete(&task.task_id, result).await;
///
/// let info = mgr.get_task(&task.task_id).await.unwrap();
/// assert_eq!(info.status, TaskStatus::Completed);
/// # });
/// ```
pub struct McpTaskManager {
    tasks: Mutex<HashMap<String, TaskEntry>>,
}

impl McpTaskManager {
    /// Create a new empty task manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new working task and return its [`Task`] metadata.
    ///
    /// The returned `Task` has status `Working` and includes a poll interval
    /// hint. Optionally attach a `JoinHandle` and `CancellationToken` for
    /// graceful cancellation support.
    pub async fn create(
        &self,
        status_message: &str,
        join_handle: Option<JoinHandle<()>>,
        cancellation_token: Option<CancellationToken>,
    ) -> Task {
        let task_id = generate_task_id();
        let now = iso8601_now();
        let task = Task::new(task_id.clone(), McpTaskStatus::Working, now.clone(), now)
            .with_status_message(status_message)
            .with_poll_interval(DEFAULT_POLL_INTERVAL_MS);

        let entry = TaskEntry {
            task: task.clone(),
            result: None,
            finished_at: None,
            join_handle,
            cancellation_token,
        };

        let mut tasks = self.tasks.lock().await;
        tasks.insert(task_id, entry);
        task
    }

    /// Attach a `JoinHandle` to an existing task for cancellation support.
    ///
    /// This is useful when the task ID must be created before spawning the
    /// background work (to avoid a circular dependency).
    pub async fn set_join_handle(&self, task_id: &str, handle: JoinHandle<()>) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(task_id) {
            entry.join_handle = Some(handle);
        }
    }

    /// Mark a task as successfully completed with its [`CallToolResult`].
    pub async fn complete(&self, task_id: &str, result: CallToolResult) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(task_id) {
            entry.task.status = McpTaskStatus::Completed;
            entry.task.status_message = Some("Task completed successfully".to_string());
            entry.task.last_updated_at = iso8601_now();
            entry.result = Some(result);
            entry.finished_at = Some(Instant::now());
            entry.join_handle = None;
        }
    }

    /// Mark a task as failed with an error message.
    pub async fn fail(&self, task_id: &str, error: &str) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(task_id) {
            entry.task.status = McpTaskStatus::Failed;
            entry.task.status_message = Some(error.to_string());
            entry.task.last_updated_at = iso8601_now();
            entry.finished_at = Some(Instant::now());
            entry.join_handle = None;
        }
    }

    /// Update the progress message of a working task.
    pub async fn update_progress(&self, task_id: &str, message: &str) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(task_id)
            && entry.task.status == McpTaskStatus::Working
        {
            entry.task.status_message = Some(message.to_string());
            entry.task.last_updated_at = iso8601_now();
        }
    }

    /// Cancel a task. Aborts the spawned tokio task if still running.
    ///
    /// Returns the updated task metadata, or `None` if the task doesn't exist.
    pub async fn cancel(&self, task_id: &str) -> Option<Task> {
        let mut tasks = self.tasks.lock().await;
        let entry = tasks.get_mut(task_id)?;

        // Only cancel if still working.
        if entry.task.status != McpTaskStatus::Working {
            return Some(entry.task.clone());
        }

        // Signal graceful cancellation first, then abort the handle as fallback.
        if let Some(token) = entry.cancellation_token.take() {
            token.cancel();
        }
        if let Some(handle) = entry.join_handle.take() {
            handle.abort();
        }

        entry.task.status = McpTaskStatus::Cancelled;
        entry.task.status_message = Some("Task cancelled by client".to_string());
        entry.task.last_updated_at = iso8601_now();
        entry.finished_at = Some(Instant::now());

        Some(entry.task.clone())
    }

    /// Get a task's metadata by ID. Triggers pruning of expired entries.
    pub async fn get_task(&self, task_id: &str) -> Option<Task> {
        let mut tasks = self.tasks.lock().await;
        prune_expired(&mut tasks);
        tasks.get(task_id).map(|e| e.task.clone())
    }

    /// Get a task's result payload. Returns `None` if the task doesn't exist
    /// or hasn't completed yet.
    pub async fn get_result(&self, task_id: &str) -> Option<CallToolResult> {
        let mut tasks = self.tasks.lock().await;
        prune_expired(&mut tasks);
        tasks.get(task_id).and_then(|e| e.result.clone())
    }

    /// List all tasks, optionally paginated. Triggers pruning first.
    pub async fn list_tasks(&self) -> TaskList {
        let mut tasks = self.tasks.lock().await;
        prune_expired(&mut tasks);
        let task_list: Vec<Task> = tasks.values().map(|e| e.task.clone()).collect();
        TaskList::new(task_list)
    }

    /// Return the number of currently tracked tasks (for testing).
    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> usize {
        self.tasks.lock().await.len()
    }
}

impl Default for McpTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove entries that finished more than [`TASK_RETENTION`] ago.
fn prune_expired(tasks: &mut HashMap<String, TaskEntry>) {
    let now = Instant::now();
    tasks.retain(|_, entry| match entry.finished_at {
        Some(finished) => now.duration_since(finished) < TASK_RETENTION,
        None => true, // still running — keep
    });
}

/// Generate a UUID-v4-style unique task ID.
fn generate_task_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    #[allow(clippy::cast_possible_truncation)]
    let nanos_lo = nanos as u64;
    let hash = nanos_lo
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(seq);
    format!("task-{nanos_lo:016x}-{hash:016x}")
}

/// Return the current time as an ISO-8601 string.
pub(crate) fn iso8601_now() -> String {
    // Use a simple approach without external chrono dependency.
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Format as seconds since epoch — not ideal but functional.
    // For proper ISO-8601 we'd need chrono, but this is sufficient
    // for the MCP protocol which just needs a sortable timestamp.
    format_timestamp(secs)
}

/// F6.2-a — render an arbitrary Unix-epoch seconds value as an
/// ISO-8601 UTC string. Used by the session export to derive the
/// session's `opened_at` from `Session::elapsed()` since
/// `Session::created_at` is a monotonic `Instant`.
///
/// F6.2-c promoted to `pub` so the cloud-side
/// `CloudSessionBundleStore` can format signed-URL expiry timestamps
/// in the same shape (the inspector compares `expires_at` strings
/// lexically).
#[must_use]
pub fn iso8601_from_secs(secs: u64) -> String {
    format_timestamp(secs)
}

/// Format a Unix timestamp as ISO-8601 UTC string.
fn format_timestamp(secs: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86400;
    const DAYS_PER_400Y: u64 = 146_097;
    const DAYS_PER_100Y: u64 = 36_524;
    const DAYS_PER_4Y: u64 = 1461;
    const DAYS_PER_Y: u64 = 365;

    let time_of_day = secs % SECONDS_PER_DAY;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01
    let mut days = secs / SECONDS_PER_DAY;
    // Shift to 2000-03-01 epoch for easier month calculation
    days += 719_468; // days from 0000-03-01 to 1970-01-01

    let era = days / DAYS_PER_400Y;
    let day_of_era = days % DAYS_PER_400Y;
    let year_of_era = (day_of_era - day_of_era / (DAYS_PER_4Y - 1) + day_of_era / DAYS_PER_100Y
        - day_of_era / (DAYS_PER_400Y - 1))
        / DAYS_PER_Y;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (DAYS_PER_Y * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Helper to build a [`GetTaskResult`] from a [`Task`].
#[must_use]
pub fn task_to_get_result(task: Task) -> GetTaskResult {
    GetTaskResult { meta: None, task }
}

/// Helper to build a [`CancelTaskResult`] from a [`Task`].
#[must_use]
pub fn task_to_cancel_result(task: Task) -> CancelTaskResult {
    CancelTaskResult { meta: None, task }
}

#[cfg(test)]
mod tests {
    use rmcp::model::Content;

    use super::*;

    #[tokio::test]
    async fn task_lifecycle_create_complete() {
        let mgr = McpTaskManager::new();
        let task = mgr.create("fetching…", None, None).await;

        // Should be working
        let info = mgr.get_task(&task.task_id).await.unwrap();
        assert_eq!(info.status, McpTaskStatus::Working);
        assert_eq!(info.status_message.as_deref(), Some("fetching…"));

        // Complete it
        let result = CallToolResult::success(vec![Content::text("done")]);
        mgr.complete(&task.task_id, result).await;
        let info = mgr.get_task(&task.task_id).await.unwrap();
        assert_eq!(info.status, McpTaskStatus::Completed);

        // Result should be available
        let result = mgr.get_result(&task.task_id).await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn task_lifecycle_create_fail() {
        let mgr = McpTaskManager::new();
        let task = mgr.create("cloning…", None, None).await;

        mgr.fail(&task.task_id, "connection refused").await;
        let info = mgr.get_task(&task.task_id).await.unwrap();
        assert_eq!(info.status, McpTaskStatus::Failed);
        assert_eq!(info.status_message.as_deref(), Some("connection refused"));

        // No result for failed tasks
        assert!(mgr.get_result(&task.task_id).await.is_none());
    }

    #[tokio::test]
    async fn task_cancellation() {
        let mgr = McpTaskManager::new();
        // Spawn a long-running dummy task
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        let task = mgr.create("long operation", Some(handle), None).await;

        let cancelled = mgr.cancel(&task.task_id).await.unwrap();
        assert_eq!(cancelled.status, McpTaskStatus::Cancelled);

        // Cancelling again is a no-op (returns current state)
        let again = mgr.cancel(&task.task_id).await.unwrap();
        assert_eq!(again.status, McpTaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn task_cancellation_signals_token() {
        let mgr = McpTaskManager::new();
        let ct = CancellationToken::new();

        // Spawn a task that watches the cancellation token.
        let ct_clone = ct.clone();
        let handle = tokio::spawn(async move {
            ct_clone.cancelled().await;
        });
        let task = mgr
            .create("with token", Some(handle), Some(ct.clone()))
            .await;

        // Token should not be cancelled yet.
        assert!(!ct.is_cancelled());

        // Cancel the task — should signal the token.
        let cancelled = mgr.cancel(&task.task_id).await.unwrap();
        assert_eq!(cancelled.status, McpTaskStatus::Cancelled);
        assert!(ct.is_cancelled());
    }

    #[tokio::test]
    async fn unknown_task_returns_none() {
        let mgr = McpTaskManager::new();
        assert!(mgr.get_task("nonexistent").await.is_none());
        assert!(mgr.get_result("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn update_progress() {
        let mgr = McpTaskManager::new();
        let task = mgr.create("phase 1", None, None).await;

        mgr.update_progress(&task.task_id, "phase 2").await;
        let info = mgr.get_task(&task.task_id).await.unwrap();
        assert_eq!(info.status_message.as_deref(), Some("phase 2"));

        // Progress update on completed task is a no-op
        let result = CallToolResult::success(vec![Content::text("done")]);
        mgr.complete(&task.task_id, result).await;
        mgr.update_progress(&task.task_id, "should not change")
            .await;
        let info = mgr.get_task(&task.task_id).await.unwrap();
        assert_eq!(info.status, McpTaskStatus::Completed);
    }

    #[tokio::test]
    async fn list_tasks_returns_all() {
        let mgr = McpTaskManager::new();
        mgr.create("task 1", None, None).await;
        mgr.create("task 2", None, None).await;

        let list = mgr.list_tasks().await;
        assert_eq!(list.tasks.len(), 2);
    }

    #[tokio::test]
    async fn prune_removes_expired_tasks() {
        let mgr = McpTaskManager::new();
        let task = mgr.create("temp", None, None).await;
        let result = CallToolResult::success(vec![Content::text("done")]);
        mgr.complete(&task.task_id, result).await;

        // Manually set finished_at to the past
        {
            let mut tasks = mgr.tasks.lock().await;
            if let Some(entry) = tasks.get_mut(&task.task_id) {
                entry.finished_at = Some(
                    Instant::now()
                        .checked_sub(Duration::from_secs(600))
                        .unwrap(),
                );
            }
        }

        // Access triggers prune
        assert!(mgr.get_task(&task.task_id).await.is_none());
        assert_eq!(mgr.len().await, 0);
    }

    #[tokio::test]
    async fn running_tasks_not_pruned() {
        let mgr = McpTaskManager::new();
        let task = mgr.create("long running", None, None).await;

        let info = mgr.get_task(&task.task_id).await;
        assert!(info.is_some());
        assert_eq!(info.unwrap().status, McpTaskStatus::Working);
    }

    #[test]
    fn format_timestamp_known_value() {
        // 2025-01-01T00:00:00Z = 1735689600
        let ts = format_timestamp(1_735_689_600);
        assert_eq!(ts, "2025-01-01T00:00:00Z");
    }

    #[tokio::test]
    async fn task_ids_are_unique() {
        let mgr = McpTaskManager::new();
        let t1 = mgr.create("a", None, None).await;
        let t2 = mgr.create("b", None, None).await;
        assert_ne!(t1.task_id, t2.task_id);
    }
}
