//! DAEMON1.13 — Integration test: proxy ↔ daemon roundtrip for all query types.

mod common;

use ministr_api::query::{
    BridgeRequest, ExtractRequest, RelatedRequest, SymbolsRequest, TocRequest,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use common::TestDaemon;

#[tokio::test]
async fn test_status() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let status = client.status().await.unwrap();
    assert!(!status.version.is_empty());
    assert_eq!(status.model_dimension, 16);
    assert!(status.uptime_secs < 60);
}

#[tokio::test]
async fn test_list_corpora() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let corpora = client.list_corpora().await.unwrap();
    assert_eq!(corpora.len(), 1);
    assert_eq!(corpora[0].id, daemon.corpus_id);
    assert_eq!(corpora[0].files_indexed, 2);
}

#[tokio::test]
async fn test_corpus_status() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let info = client.corpus_status(&daemon.corpus_id).await.unwrap();
    assert_eq!(info.id, daemon.corpus_id);
    assert_eq!(info.sections_count, 3);
}

#[tokio::test]
async fn test_survey() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let resp = client
        .survey(&daemon.corpus_id, "JWT authentication tokens", Some(5))
        .await
        .unwrap();
    assert!(!resp.results.is_empty(), "survey should return results");
    for r in &resp.results {
        assert!(r.score > 0.0, "score should be positive");
        assert!(!r.content_id.is_empty());
    }
}

#[tokio::test]
async fn test_read_section() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let detail = client
        .read_section(&daemon.corpus_id, "docs/auth.md#tokens")
        .await
        .unwrap();
    assert_eq!(detail.section_id, "docs/auth.md#tokens");
    assert!(detail.text.contains("JWT tokens"));
    assert_eq!(detail.heading_path, vec!["Authentication", "Tokens"]);
    assert_eq!(detail.claims_available, 2);
}

#[tokio::test]
async fn test_symbols() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = SymbolsRequest {
        query: "MinistrConfig".into(),
        kind: None,
        module: None,
        visibility: None,
        limit: None,
        session_id: None,
    };
    let resp = client.symbols(&daemon.corpus_id, &req).await.unwrap();
    assert!(!resp.symbols.is_empty(), "should find MinistrConfig symbol");
    assert_eq!(resp.symbols[0].name, "MinistrConfig");
    assert_eq!(resp.symbols[0].kind, "struct");
}

#[tokio::test]
async fn test_definition() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let def = client
        .definition(&daemon.corpus_id, "sym-config::MinistrConfig", None)
        .await
        .unwrap();
    assert_eq!(def.name, "MinistrConfig");
    assert_eq!(def.kind, "struct");
    assert_eq!(def.visibility, "pub");
    assert_eq!(def.line_start, 10);
    assert_eq!(def.line_end, 25);
}

#[tokio::test]
async fn test_references() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let resp = client
        .references(&daemon.corpus_id, "sym-config::MinistrConfig", None)
        .await
        .unwrap();
    assert!(
        !resp.references.is_empty(),
        "MinistrConfig should have references"
    );
    let r = &resp.references[0];
    assert_eq!(r.from_symbol_id, "sym-service::survey");
    assert_eq!(r.to_symbol_id, "sym-config::MinistrConfig");
}

#[tokio::test]
async fn test_toc() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = TocRequest {
        document_id: None,
        offset: None,
        limit: None,
        session_id: None,
    };
    let resp = client.toc(&daemon.corpus_id, &req).await.unwrap();
    assert!(resp.total >= 3, "should have at least 3 sections");
    assert!(!resp.entries.is_empty());
}

#[tokio::test]
async fn test_extract() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = ExtractRequest {
        section_id: "docs/auth.md#tokens".into(),
        query: None,
        session_id: None,
    };
    let resp = client.extract(&daemon.corpus_id, &req).await.unwrap();
    assert_eq!(resp.claims.len(), 2, "tokens section has 2 claims");
    let claim_texts: Vec<&str> = resp.claims.iter().map(|c| c.text.as_str()).collect();
    assert!(claim_texts.iter().any(|t| t.contains("RS256")));
    assert!(claim_texts.iter().any(|t| t.contains("24 hours")));
}

#[tokio::test]
async fn test_related() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = RelatedRequest {
        claim_id: "auth-c1".into(),
        relation_types: vec![],
        session_id: None,
    };
    let resp = client.related(&daemon.corpus_id, &req).await.unwrap();
    assert!(
        !resp.claims.is_empty(),
        "auth-c1 should have related claims"
    );
    assert!(resp.claims.iter().any(|c| c.claim_id == "auth-c2"));
}

#[tokio::test]
async fn test_bridge() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = BridgeRequest {
        query: None,
        kind: None,
        source_language: None,
        limit: None,
        session_id: None,
    };
    let resp = client.bridge(&daemon.corpus_id, &req).await.unwrap();
    assert!(!resp.links.is_empty(), "should have bridge links");
    let link = &resp.links[0];
    assert_eq!(link.kind, "tauri_command");
    assert!(link.confidence > 0.0);
}

#[tokio::test]
async fn test_session_lifecycle() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // Create session.
    let session = client
        .create_session(&daemon.corpus_id, Some(50_000))
        .await
        .unwrap();
    assert!(session.session_id.starts_with("sess-"));

    // Check budget.
    let budget = client
        .session_budget(&daemon.corpus_id, &session.session_id)
        .await
        .unwrap();
    assert_eq!(budget.tokens_used, 0);
    assert_eq!(budget.tokens_remaining, 50_000);
    assert!(budget.utilization < f64::EPSILON);

    // Destroy session.
    client
        .destroy_session(&daemon.corpus_id, &session.session_id)
        .await
        .unwrap();

    // Budget should now 404.
    let err = client
        .session_budget(&daemon.corpus_id, &session.session_id)
        .await;
    assert!(err.is_err(), "destroyed session should return error");
}

#[tokio::test]
async fn test_compress() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = ministr_api::session::CompressRequest {
        content_ids: vec!["docs/auth.md#tokens".into()],
        session_id: None,
    };
    let resp = client.compress(&daemon.corpus_id, &req).await.unwrap();
    // Extractive compression may skip very short sections, so allow 0 or 1.
    assert!(resp.summaries.len() <= 1);
    if let Some(item) = resp.summaries.first() {
        assert_eq!(item.original_id, "docs/auth.md#tokens");
        assert!(!item.summary.is_empty());
        assert_eq!(item.method, "extractive");
    }
}

#[tokio::test]
async fn test_compress_unknown_ids() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = ministr_api::session::CompressRequest {
        content_ids: vec!["nonexistent#section".into()],
        session_id: None,
    };
    let resp = client.compress(&daemon.corpus_id, &req).await.unwrap();
    assert!(resp.summaries.is_empty());
}

#[tokio::test]
async fn test_evict_content() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // Create a session first.
    let session = client
        .create_session(&daemon.corpus_id, Some(50_000))
        .await
        .unwrap();

    // Evict content IDs (not previously delivered — should be not_found).
    let req = ministr_api::session::EvictRequest {
        content_ids: vec!["docs/auth.md#tokens".into(), "nonexistent".into()],
    };
    let resp = client
        .evict_content(&daemon.corpus_id, &session.session_id, &req)
        .await
        .unwrap();

    // Neither was delivered, so both should be not_found.
    assert!(resp.evicted.is_empty());
    assert_eq!(resp.not_found.len(), 2);
}

#[tokio::test]
async fn test_evict_nonexistent_session() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = ministr_api::session::EvictRequest {
        content_ids: vec!["docs/auth.md#tokens".into()],
    };
    let result = client
        .evict_content(&daemon.corpus_id, "sess-nonexistent", &req)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ingestion_progress_sse() {
    let daemon = TestDaemon::start().await;

    // Connect raw HTTP to the SSE endpoint and read the first event.
    let mut stream = tokio::net::UnixStream::connect(&daemon.socket_path)
        .await
        .unwrap();

    let request = format!(
        "GET /api/v1/corpora/{}/progress HTTP/1.1\r\n\
         Host: localhost\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         \r\n",
        daemon.corpus_id
    );
    stream.write_all(request.as_bytes()).await.unwrap();

    // Read response — may need multiple reads to get headers + first event.
    let mut response = String::new();
    let mut buf = vec![0u8; 4096];
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                response.push_str(&String::from_utf8_lossy(&buf[..n]));
                if response.contains("data:") {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        response.contains("text/event-stream"),
        "should return SSE content type, got: {response}"
    );
    assert!(
        response.contains("data:"),
        "should contain SSE data event, got: {response}"
    );
}

#[tokio::test]
async fn test_session_persistence() {
    use ministr_daemon::persistence;
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("sessions.db");

    // Save a session.
    persistence::save_session(
        &db_path,
        "corpus-1",
        "sess-abc",
        50_000,
        3,
        &std::collections::BTreeMap::new(),
        &[],
    )
    .unwrap();

    // Load it back.
    let sessions = persistence::load_sessions(&db_path, "corpus-1").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-abc");
    assert_eq!(sessions[0].budget_tokens, 50_000);
    assert_eq!(sessions[0].current_turn, 3);

    // Delete it.
    persistence::delete_session(&db_path, "corpus-1", "sess-abc").unwrap();
    let sessions = persistence::load_sessions(&db_path, "corpus-1").unwrap();
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn test_rate_limiting_concurrent_surveys() {
    // Verify that concurrent surveys beyond the semaphore limit are queued (not rejected).
    let daemon = TestDaemon::start().await;
    let num_concurrent = 8; // More than the default concurrency limit of 4.

    let mut handles = Vec::new();
    for i in 0..num_concurrent {
        let client = daemon.client();
        let corpus_id = daemon.corpus_id.clone();
        handles.push(tokio::spawn(async move {
            client
                .survey(&corpus_id, &format!("rate limit test {i}"), Some(3))
                .await
                .unwrap()
        }));
    }

    // All should succeed (queued, not rejected).
    for handle in handles {
        let resp = handle.await.unwrap();
        assert!(resp.results.len() <= 3);
    }
}

#[tokio::test]
async fn test_coherence_sse_endpoint() {
    let daemon = TestDaemon::start().await;

    let mut stream = tokio::net::UnixStream::connect(&daemon.socket_path)
        .await
        .unwrap();

    let request = format!(
        "GET /api/v1/corpora/{}/coherence HTTP/1.1\r\n\
         Host: localhost\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         \r\n",
        daemon.corpus_id
    );
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read(&mut buf))
        .await
        .unwrap()
        .unwrap();

    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.contains("text/event-stream"),
        "should return SSE content type, got: {response}"
    );
}

#[tokio::test]
async fn test_bundle_import_nonexistent() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let req = ministr_api::corpus::ImportBundleRequest {
        bundle_path: "/nonexistent/bundle.ministr-index".into(),
    };
    let result = client.import_bundle(&req).await;
    assert!(result.is_err(), "import of nonexistent bundle should fail");
}
