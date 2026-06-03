//! DAEMON1.12 — Stress test: multiple concurrent proxy connections.

mod common;

use std::time::Duration;

use ministr_api::query::{ExtractRequest, SymbolsRequest, TocRequest};

use common::TestDaemon;

#[tokio::test]
async fn test_concurrent_proxies_mixed_queries() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let daemon = TestDaemon::start().await;
        let num_proxies = 10;
        let queries_per_proxy = 20;

        let mut handles = Vec::new();
        for proxy_id in 0..num_proxies {
            let client = daemon.client();
            let corpus_id = daemon.corpus_id.clone();
            handles.push(tokio::spawn(async move {
                for q in 0..queries_per_proxy {
                    match q % 5 {
                        0 => {
                            let resp = client
                                .survey(&corpus_id, &format!("query {proxy_id}-{q}"), Some(3))
                                .await
                                .unwrap();
                            assert!(resp.results.len() <= 3);
                        }
                        1 => {
                            let detail = client
                                .read_section(&corpus_id, "docs/auth.md#tokens")
                                .await
                                .unwrap();
                            assert!(detail.text.contains("JWT"));
                        }
                        2 => {
                            let req = TocRequest {
                                document_id: None,
                                offset: None,
                                limit: None,
                                session_id: None,
                            };
                            let resp = client.toc(&corpus_id, &req).await.unwrap();
                            assert!(resp.total >= 3);
                        }
                        3 => {
                            let req = ExtractRequest {
                                section_id: "docs/auth.md#tokens".into(),
                                query: None,
                                session_id: None,
                            };
                            let resp = client.extract(&corpus_id, &req).await.unwrap();
                            assert_eq!(resp.claims.len(), 2);
                        }
                        4 => {
                            let status = client.status().await.unwrap();
                            assert_eq!(status.model_dimension, 16);
                        }
                        _ => unreachable!(),
                    }
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    assert!(result.is_ok(), "test timed out — possible deadlock");
}

#[tokio::test]
async fn test_concurrent_session_isolation() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let daemon = TestDaemon::start().await;
        let num_clients = 5;

        let mut handles = Vec::new();
        for _ in 0..num_clients {
            let client = daemon.client();
            let corpus_id = daemon.corpus_id.clone();
            handles.push(tokio::spawn(async move {
                // Create a session.
                let session = client
                    .create_session(&corpus_id, Some(100_000))
                    .await
                    .unwrap();
                assert!(session.session_id.starts_with("sess-"));

                // Check budget.
                let budget = client
                    .session_usage(&corpus_id, &session.session_id)
                    .await
                    .unwrap();
                assert_eq!(budget.tokens_used, 0);

                // Destroy session.
                client
                    .destroy_session(&corpus_id, &session.session_id)
                    .await
                    .unwrap();

                session.session_id
            }));
        }

        let mut session_ids = Vec::new();
        for handle in handles {
            session_ids.push(handle.await.unwrap());
        }

        // All sessions should have unique IDs.
        session_ids.sort();
        session_ids.dedup();
        assert_eq!(
            session_ids.len(),
            num_clients,
            "all session IDs should be unique"
        );
    })
    .await;

    assert!(result.is_ok(), "test timed out — possible deadlock");
}

#[tokio::test]
async fn test_high_concurrency_survey() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let daemon = TestDaemon::start().await;
        let num_clients = 50;
        let queries_per_client = 10;

        let mut handles = Vec::new();
        for client_id in 0..num_clients {
            let client = daemon.client();
            let corpus_id = daemon.corpus_id.clone();
            handles.push(tokio::spawn(async move {
                for q in 0..queries_per_client {
                    let resp = client
                        .survey(
                            &corpus_id,
                            &format!("concurrent search {client_id}-{q}"),
                            Some(5),
                        )
                        .await
                        .unwrap();
                    // Every survey should return valid results.
                    for r in &resp.results {
                        assert!(r.score > 0.0);
                        assert!(!r.content_id.is_empty());
                    }
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    assert!(result.is_ok(), "test timed out — possible deadlock");
}

#[tokio::test]
async fn test_concurrent_mixed_symbol_queries() {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let daemon = TestDaemon::start().await;
        let num_clients = 10;

        let mut handles = Vec::new();
        for _ in 0..num_clients {
            let client = daemon.client();
            let corpus_id = daemon.corpus_id.clone();
            handles.push(tokio::spawn(async move {
                // Symbols search.
                let req = SymbolsRequest {
                    query: "Config".into(),
                    kind: None,
                    module: None,
                    visibility: None,
                    file_path: None,
                    limit: None,
                    session_id: None,
                };
                let resp = client.symbols(&corpus_id, &req).await.unwrap();
                assert!(!resp.symbols.is_empty());

                // Definition.
                let def = client
                    .definition(&corpus_id, "sym-config::MinistrConfig", None)
                    .await
                    .unwrap();
                assert_eq!(def.name, "MinistrConfig");

                // References.
                let refs = client
                    .references(&corpus_id, "sym-config::MinistrConfig", None, false)
                    .await
                    .unwrap();
                assert!(!refs.references.is_empty());
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    assert!(result.is_ok(), "test timed out — possible deadlock");
}
