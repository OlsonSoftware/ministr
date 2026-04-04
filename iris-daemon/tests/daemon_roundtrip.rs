//! DAEMON1.13 — Integration test: proxy ↔ daemon roundtrip for all query types.

mod common;

use iris_api::query::{BridgeRequest, ExtractRequest, RelatedRequest, SymbolsRequest, TocRequest};

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
        query: "IrisConfig".into(),
        kind: None,
        module: None,
        visibility: None,
        limit: None,
    };
    let resp = client.symbols(&daemon.corpus_id, &req).await.unwrap();
    assert!(!resp.symbols.is_empty(), "should find IrisConfig symbol");
    assert_eq!(resp.symbols[0].name, "IrisConfig");
    assert_eq!(resp.symbols[0].kind, "struct");
}

#[tokio::test]
async fn test_definition() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let def = client
        .definition(&daemon.corpus_id, "sym-config::IrisConfig")
        .await
        .unwrap();
    assert_eq!(def.name, "IrisConfig");
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
        .references(&daemon.corpus_id, "sym-config::IrisConfig")
        .await
        .unwrap();
    assert!(
        !resp.references.is_empty(),
        "IrisConfig should have references"
    );
    let r = &resp.references[0];
    assert_eq!(r.from_symbol_id, "sym-service::survey");
    assert_eq!(r.to_symbol_id, "sym-config::IrisConfig");
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
