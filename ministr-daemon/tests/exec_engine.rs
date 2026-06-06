//! Integration tests for the daemon exec run engine (exec-core-engine).
//!
//! Unix-gated: the commands under test use `sh`, and the process-group
//! kill contract is unix-specific (Windows kills the direct child only —
//! a documented gap in `exec.rs`).
#![cfg(unix)]

use std::sync::Arc;
use std::time::Duration;

use ministr_daemon::exec::{
    RunEngine, RunEngineConfig, RunRequest, RunStatus, RunsFilter, StaticRoots,
};

fn engine_at(dir: &std::path::Path, config: RunEngineConfig) -> RunEngine {
    RunEngine::new(
        dir.join("runs.db"),
        Arc::new(StaticRoots(vec![dir.to_path_buf()])),
        config,
    )
    .expect("engine creation")
}

fn request(dir: &std::path::Path, command: &str) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        cwd: dir.to_path_buf(),
        session_id: Some("sess-test".to_string()),
        corpus_id: Some("corpus-test".to_string()),
        timeout: None,
    }
}

/// Is a pid alive? (`kill -0` probes without signaling.)
fn pid_alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .status()
        .is_ok_and(|s| s.success())
}

#[tokio::test]
async fn run_record_persists_across_engine_reopen() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let run_id = {
        let engine = engine_at(tmp.path(), RunEngineConfig::default());
        let record = engine
            .run(request(tmp.path(), "echo hello-exec-engine"))
            .await
            .expect("run");
        assert_eq!(record.status, RunStatus::Exited);
        assert_eq!(record.exit_code, Some(0));
        assert!(record.log.contains("hello-exec-engine"));
        assert!(!record.truncated);
        assert!(record.finished_at_ms.is_some());
        record.run_id
    };

    // A brand-new engine over the same db (daemon restart) sees the run.
    let reopened = engine_at(tmp.path(), RunEngineConfig::default());
    let record = reopened
        .get(&run_id)
        .expect("get")
        .expect("record survives reopen");
    assert_eq!(record.status, RunStatus::Exited);
    assert!(record.log.contains("hello-exec-engine"));

    // And it is queryable by session.
    let listed = reopened
        .list(&RunsFilter {
            session_id: Some("sess-test".to_string()),
            ..RunsFilter::default()
        })
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].run_id, run_id);
}

#[tokio::test]
async fn cancel_kills_the_whole_process_group() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = engine_at(tmp.path(), RunEngineConfig::default());
    let pidfile = tmp.path().join("grandchild.pid");

    // The command forks a grandchild and records its pid, then waits.
    let cmd = format!("sleep 30 & echo $! > {} && wait", pidfile.display());
    let run_id = engine.start(request(tmp.path(), &cmd)).expect("start");

    // Wait for the grandchild pid to land on disk.
    let mut pid = String::new();
    for _ in 0..200 {
        if let Ok(content) = std::fs::read_to_string(&pidfile) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                pid = trimmed.to_string();
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(!pid.is_empty(), "grandchild pid never appeared");
    assert!(pid_alive(&pid), "grandchild should be alive before cancel");

    assert!(engine.cancel(&run_id), "run should still be active");

    // The whole group dies — including the backgrounded sleep.
    let mut dead = false;
    for _ in 0..200 {
        if !pid_alive(&pid) {
            dead = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(dead, "grandchild survived cancel — orphaned process");

    // The record finalizes as killed.
    let mut status = None;
    for _ in 0..200 {
        let record = engine.get(&run_id).expect("get").expect("record");
        if record.status != RunStatus::Running {
            status = Some(record.status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status, Some(RunStatus::Killed));
}

#[tokio::test]
async fn output_guard_caps_volume_without_losing_head_or_exit_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = engine_at(
        tmp.path(),
        RunEngineConfig {
            head_cap: 4 * 1024,
            tail_cap: 4 * 1024,
            default_timeout: Duration::from_secs(60),
        },
    );

    // ~2 MB of output through 8 KB of preserved budget. `yes` dies on
    // SIGPIPE when `head` closes the pipe; the pipeline exits 0.
    let record = engine
        .run(request(
            tmp.path(),
            "echo HEAD-MARKER; yes 0123456789abcdef | head -c 2000000; echo TAIL-MARKER",
        ))
        .await
        .expect("run");

    assert_eq!(record.status, RunStatus::Exited);
    assert_eq!(
        record.exit_code,
        Some(0),
        "exit code must survive the guard"
    );
    assert!(record.truncated, "2 MB through 8 KB caps must truncate");
    assert!(
        record.bytes_total >= 2_000_000,
        "exact byte total survives truncation (got {})",
        record.bytes_total
    );
    assert!(
        record.log.len() < 16 * 1024,
        "stored log stays bounded (got {} bytes)",
        record.log.len()
    );
    assert!(
        record.log.starts_with("HEAD-MARKER"),
        "the head of the output must be preserved verbatim"
    );
    assert!(
        record.log.contains("TAIL-MARKER"),
        "the tail of the output must be preserved"
    );
    assert!(record.log.contains("bytes dropped"));
}

#[tokio::test]
async fn policy_denies_cwd_outside_allowed_roots() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let engine = engine_at(tmp.path(), RunEngineConfig::default());

    let err = engine
        .start(RunRequest {
            command: "echo never".to_string(),
            cwd: outside.path().to_path_buf(),
            session_id: None,
            corpus_id: None,
            timeout: None,
        })
        .expect_err("cwd outside every root must be denied");
    assert!(
        err.to_string().contains("exec policy"),
        "unexpected error: {err}"
    );

    // Nothing was recorded for the denied request.
    let listed = engine.list(&RunsFilter::default()).expect("list");
    assert!(listed.is_empty());
}

#[tokio::test]
async fn timeout_finalizes_as_timed_out() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = engine_at(tmp.path(), RunEngineConfig::default());

    let record = engine
        .run(RunRequest {
            command: "sleep 30".to_string(),
            cwd: tmp.path().to_path_buf(),
            session_id: None,
            corpus_id: None,
            timeout: Some(Duration::from_millis(200)),
        })
        .await
        .expect("run");
    assert_eq!(record.status, RunStatus::TimedOut);
    assert_eq!(record.exit_code, None);
}

#[tokio::test]
async fn list_filters_by_session_and_since() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = engine_at(tmp.path(), RunEngineConfig::default());

    let first = engine
        .run(request(tmp.path(), "echo one"))
        .await
        .expect("run one");
    let mut other = request(tmp.path(), "echo two");
    other.session_id = Some("sess-other".to_string());
    engine.run(other).await.expect("run two");

    let by_session = engine
        .list(&RunsFilter {
            session_id: Some("sess-other".to_string()),
            ..RunsFilter::default()
        })
        .expect("list by session");
    assert_eq!(by_session.len(), 1);
    assert!(by_session[0].log.contains("two"));

    let since = engine
        .list(&RunsFilter {
            since_ms: Some(first.started_at_ms),
            ..RunsFilter::default()
        })
        .expect("list since");
    assert!(since.iter().all(|r| r.started_at_ms > first.started_at_ms));
}
