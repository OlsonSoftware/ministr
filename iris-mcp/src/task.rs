//! Background task manager for long-running operations.
//!
//! Provides a [`TaskManager`] that tracks spawned async operations (like
//! `iris_fetch` and `iris_clone`) and lets callers poll for completion via
//! task handles. Completed tasks are retained for a configurable duration
//! and automatically pruned on access.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

/// How long completed/failed tasks are retained before pruning.
const TASK_RETENTION: Duration = Duration::from_secs(300); // 5 minutes

/// Unique identifier for a background task.
pub type TaskId = String;

/// Current status of a background task.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task is still running.
    Running {
        /// Human-readable progress message (e.g. "cloning repository…").
        message: String,
    },
    /// Task completed successfully.
    Completed {
        /// The JSON result payload (same as what the sync tool would return).
        result: serde_json::Value,
    },
    /// Task failed with an error.
    Failed {
        /// Error description.
        error: String,
    },
}

/// Internal entry in the task map.
#[derive(Debug, Clone)]
struct TaskEntry {
    status: TaskStatus,
    /// When the task reached a terminal state (completed/failed).
    finished_at: Option<Instant>,
}

/// Manages background tasks with automatic pruning of stale entries.
///
/// Thread-safe via interior mutability (`Arc<Mutex<…>>`).
///
/// # Examples
///
/// ```
/// use iris_mcp::task::{TaskManager, TaskStatus};
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let mgr = TaskManager::new();
/// let id = mgr.create("starting…").await;
///
/// // Task is running
/// let status = mgr.get(&id).await.unwrap();
/// assert!(matches!(status, TaskStatus::Running { .. }));
///
/// // Complete the task
/// mgr.complete(&id, serde_json::json!({"ok": true})).await;
/// let status = mgr.get(&id).await.unwrap();
/// assert!(matches!(status, TaskStatus::Completed { .. }));
/// # });
/// ```
pub struct TaskManager {
    tasks: Mutex<HashMap<TaskId, TaskEntry>>,
}

impl TaskManager {
    /// Create a new empty task manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new running task and return its ID.
    pub async fn create(&self, message: &str) -> TaskId {
        let id = uuid_v4();
        let entry = TaskEntry {
            status: TaskStatus::Running {
                message: message.to_string(),
            },
            finished_at: None,
        };
        let mut tasks = self.tasks.lock().await;
        tasks.insert(id.clone(), entry);
        id
    }

    /// Get the current status of a task, or `None` if it doesn't exist.
    ///
    /// Also triggers pruning of expired entries.
    pub async fn get(&self, id: &str) -> Option<TaskStatus> {
        let mut tasks = self.tasks.lock().await;
        prune_expired(&mut tasks);
        tasks.get(id).map(|e| e.status.clone())
    }

    /// Mark a task as successfully completed with its result payload.
    pub async fn complete(&self, id: &str, result: serde_json::Value) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(id) {
            entry.status = TaskStatus::Completed { result };
            entry.finished_at = Some(Instant::now());
        }
    }

    /// Mark a task as failed with an error message.
    pub async fn fail(&self, id: &str, error: String) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(id) {
            entry.status = TaskStatus::Failed { error };
            entry.finished_at = Some(Instant::now());
        }
    }

    /// Update the progress message of a running task.
    pub async fn update_progress(&self, id: &str, message: &str) {
        let mut tasks = self.tasks.lock().await;
        if let Some(entry) = tasks.get_mut(id) {
            if matches!(entry.status, TaskStatus::Running { .. }) {
                entry.status = TaskStatus::Running {
                    message: message.to_string(),
                };
            }
        }
    }

    /// Return the number of currently tracked tasks (for testing).
    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> usize {
        self.tasks.lock().await.len()
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove entries that finished more than [`TASK_RETENTION`] ago.
fn prune_expired(tasks: &mut HashMap<TaskId, TaskEntry>) {
    let now = Instant::now();
    tasks.retain(|_, entry| match entry.finished_at {
        Some(finished) => now.duration_since(finished) < TASK_RETENTION,
        None => true, // still running — keep
    });
}

/// Generate a short UUID-v4-style hex string.
fn uuid_v4() -> String {
    use std::time::SystemTime;

    // Simple unique ID using timestamp + random bits from hashing.
    // No external dependency needed — collisions are negligible for
    // task IDs within a single server process.
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    #[allow(clippy::cast_possible_truncation)]
    let nanos_lo = nanos as u64;
    let random_bits: u64 = nanos_lo
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1);
    format!("task-{nanos:x}-{random_bits:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_lifecycle_create_complete() {
        let mgr = TaskManager::new();
        let id = mgr.create("fetching…").await;

        // Should be running
        let status = mgr.get(&id).await.unwrap();
        assert!(matches!(status, TaskStatus::Running { .. }));
        if let TaskStatus::Running { message } = status {
            assert_eq!(message, "fetching…");
        }

        // Complete it
        mgr.complete(&id, serde_json::json!({"pages": 5})).await;
        let status = mgr.get(&id).await.unwrap();
        assert!(matches!(status, TaskStatus::Completed { .. }));
        if let TaskStatus::Completed { result } = status {
            assert_eq!(result["pages"], 5);
        }
    }

    #[tokio::test]
    async fn task_lifecycle_create_fail() {
        let mgr = TaskManager::new();
        let id = mgr.create("cloning…").await;

        mgr.fail(&id, "connection refused".to_string()).await;
        let status = mgr.get(&id).await.unwrap();
        assert!(matches!(status, TaskStatus::Failed { .. }));
        if let TaskStatus::Failed { error } = status {
            assert_eq!(error, "connection refused");
        }
    }

    #[tokio::test]
    async fn unknown_task_returns_none() {
        let mgr = TaskManager::new();
        assert!(mgr.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn update_progress() {
        let mgr = TaskManager::new();
        let id = mgr.create("phase 1").await;

        mgr.update_progress(&id, "phase 2").await;
        let status = mgr.get(&id).await.unwrap();
        if let TaskStatus::Running { message } = status {
            assert_eq!(message, "phase 2");
        } else {
            panic!("expected Running status");
        }

        // Progress update on completed task is a no-op
        mgr.complete(&id, serde_json::json!(null)).await;
        mgr.update_progress(&id, "should not change").await;
        let status = mgr.get(&id).await.unwrap();
        assert!(matches!(status, TaskStatus::Completed { .. }));
    }

    #[tokio::test]
    async fn prune_removes_expired_tasks() {
        let mgr = TaskManager::new();
        let id = mgr.create("temp").await;
        mgr.complete(&id, serde_json::json!(null)).await;

        // Manually set finished_at to the past
        {
            let mut tasks = mgr.tasks.lock().await;
            if let Some(entry) = tasks.get_mut(&id) {
                entry.finished_at = Some(
                    Instant::now()
                        .checked_sub(Duration::from_secs(600))
                        .unwrap(),
                );
            }
        }

        // Access triggers prune
        assert!(mgr.get(&id).await.is_none());
        assert_eq!(mgr.len().await, 0);
    }

    #[tokio::test]
    async fn running_tasks_not_pruned() {
        let mgr = TaskManager::new();
        let id = mgr.create("long running").await;

        // Even after prune cycle, running tasks survive
        let status = mgr.get(&id).await;
        assert!(status.is_some());
        assert!(matches!(status.unwrap(), TaskStatus::Running { .. }));
    }
}
