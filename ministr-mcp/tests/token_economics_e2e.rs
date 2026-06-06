//! Real end-to-end token-economics measurement.
//!
//! This is the REAL benchmark behind the public "fewer tokens" claim. It does
//! **not** model token cost with a fabricated constant; it measures what
//! actually crosses an agent's context window:
//!
//! - **ministr cost** = `count_tokens` of the *literal bytes* a `ministr_survey`
//!   call returns through the real MCP `call_tool` path (including the condensed
//!   `_condensed` envelope and the `next_actions`). This is exactly what the
//!   agent receives — not a hand-picked extract.
//! - **grep + read cost** = `count_tokens` of every real corpus file that shares
//!   a salient term with the query (the "grep for the terms, then read each
//!   candidate file to be sure" workflow), over the *same real files* the survey
//!   ran against.
//!
//! Both sides run against the committed real, multi-language corpus
//! `eval/corpus-code` (Rust/Python/Go/TypeScript/Java/C++), ingested through the
//! real [`IngestionPipeline`]. The embedder is the deterministic hash mock: it
//! has no semantic skill, but **token VOLUME — how much text crosses the wire —
//! does not depend on embedding quality**, only on the corpus, the query, and
//! the response format, so the numbers are reproducible run to run.
//!
//! HONEST FRAMING (what the old synthetic micro-benchmark hid): a ministr lookup
//! does NOT cost "a flat 68 tokens". A survey returns the top-k ranked slices
//! under a per-result budget. The win over grep+read is real but *nuanced*: it
//! grows with corpus size and with how much irrelevant code grep drags in, and
//! on a tiny corpus a broad top-k survey can occasionally cost as much as a tight
//! grep. The figure to publish is the measured aggregate here, with its method.
//!
//! Reproduce the table:
//! ```text
//! cargo test -p ministr-mcp --test token_economics_e2e report_real_token_economics -- --nocapture
//! ```

use std::collections::BTreeSet;
use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::service::QueryService;
use ministr_core::storage::SqliteStorage;
use ministr_core::token::count_tokens;
use ministr_mcp::server::MinistrServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
use serde_json::json;

/// Deterministic hash-based mock embedder (mirrors the e2e suite). Token volume
/// is independent of embedding quality, so this keeps the measurement
/// reproducible and model-free.
struct MockEmbedder {
    dim: usize,
}

impl Embedder for MockEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for (i, b) in t.bytes().enumerate() {
                    v[i % self.dim] += f32::from(b) / 255.0;
                }
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServerHandle = rmcp::service::RunningService<rmcp::RoleServer, MinistrServer>;

async fn wrap_as_client(server: MinistrServer) -> (McpClient, McpServerHandle) {
    let (c2s_w, c2s_r) = tokio::io::duplex(1 << 20);
    let (s2c_w, s2c_r) = tokio::io::duplex(1 << 20);
    let server_task = tokio::spawn(async move { server.serve((c2s_r, s2c_w)).await.unwrap() });
    let client = ().serve((s2c_r, c2s_w)).await.unwrap();
    let server_handle = server_task.await.unwrap();
    (client, server_handle)
}

async fn call_tool(client: &McpClient, name: &str, args: serde_json::Value) -> CallToolResult {
    let arguments = args
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    let mut params = CallToolRequestParams::new(name.to_string());
    if let Some(args) = arguments {
        params = params.with_arguments(args);
    }
    client.peer().call_tool(params).await.unwrap()
}

/// The full literal text payload the agent receives for a tool call — every
/// `Content` text part concatenated. This is what we tokenize.
fn response_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| c.raw.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// English function words excluded from the grep term set, so a file becomes a
/// "candidate" only when it shares a *content* word with the query — the same
/// thing an agent would actually grep for. Deliberately conservative (only
/// genuine stopwords) so the grep baseline is not inflated in ministr's favor.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "over", "into", "from", "when", "whether", "should",
    "right", "now", "must", "before", "next", "instead", "are", "its", "this", "than", "then",
    "once", "how", "long", "run", "per", "they", "keep", "after", "each", "until", "while", "two",
    "one", "out", "off", "not", "but", "has", "have", "was", "were", "been", "their", "them",
    "all", "any", "can", "may", "will", "would", "could", "about", "your", "you", "use", "using",
    "via",
];

/// Salient query terms: alphanumeric words of length >= 4 that are not common
/// stopwords, lowercased. The grep model matches a file if it contains any of
/// these (case-insensitive substring) — "grep the query terms, read the hits".
fn salient_terms(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(&w.as_str()))
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

/// grep + read token cost for `query` over the real files: the union of files
/// containing any salient term, each read whole. Returns `(matched_files, tokens)`.
fn grep_read_cost(query: &str, files: &[(String, String)]) -> (usize, usize) {
    let terms = salient_terms(query);
    let mut tokens = 0;
    let mut matched = 0;
    for (_name, content) in files {
        let hay = content.to_lowercase();
        if terms.iter().any(|t| hay.contains(t.as_str())) {
            matched += 1;
            tokens += count_tokens(content);
        }
    }
    (matched, tokens)
}

/// Locate `eval/corpus-code` relative to the workspace root, returning the
/// directory path and the (filename, contents) of each file in it. `None` when
/// the eval fixtures aren't checked out (keeps CI green on a sparse checkout).
fn load_code_corpus() -> Option<(std::path::PathBuf, Vec<(String, String)>)> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir).parent()?;
    let dir = workspace_root.join("eval/corpus-code");
    if !dir.exists() {
        eprintln!("Skipping token-economics e2e: eval/corpus-code not found");
        return None;
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir).ok()? {
        let path = entry.ok()?.path();
        if path.is_file() {
            let name = path.file_name()?.to_string_lossy().into_owned();
            let content = std::fs::read_to_string(&path).ok()?;
            files.push((name, content));
        }
    }
    files.sort();
    Some((dir, files))
}

/// The real natural-language queries from `eval/ground-truth-code.json` — the
/// same intents the retrieval-quality eval uses, so the token measurement runs
/// on representative agent questions, not invented ones.
fn load_queries() -> Vec<String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = std::path::Path::new(manifest_dir)
        .parent()
        .expect("workspace root");
    let gt_path = workspace_root.join("eval/ground-truth-code.json");
    let Ok(text) = std::fs::read_to_string(&gt_path) else {
        return Vec::new();
    };
    let json: serde_json::Value = serde_json::from_str(&text).expect("ground-truth-code.json");
    json["queries"]
        .as_array()
        .map(|qs| {
            qs.iter()
                .filter_map(|q| q["query"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Ingest `eval/corpus-code` once and return the shared storage + index + the
/// deterministic embedder, ready to spin a fresh session per query.
async fn ingest_code_corpus(
    dir: &std::path::Path,
) -> (Arc<SqliteStorage>, Arc<HnswIndex>, Arc<MockEmbedder>) {
    let dim = 16;
    let embedder = Arc::new(MockEmbedder { dim });
    let index = Arc::new(HnswIndex::new(dim, 10_000).unwrap());
    let storage = SqliteStorage::open_in_memory().unwrap();

    let stats = IngestionPipeline::new()
        .ingest_directory_with_embeddings(dir, &storage, embedder.as_ref(), index.as_ref())
        .await
        .expect("ingestion failed");
    assert!(stats.total_sections > 0, "corpus produced no sections");

    (Arc::new(storage), index, embedder)
}

/// One ministr lookup's real token cost: a fresh session (so survey
/// deduplication can't shrink later queries) runs `ministr_survey` through the
/// real MCP path; we tokenize the literal response. `top_k` is the survey
/// breadth an agent would request.
async fn ministr_survey_cost(
    storage: &Arc<SqliteStorage>,
    index: &Arc<HnswIndex>,
    embedder: &Arc<MockEmbedder>,
    query: &str,
    top_k: usize,
) -> usize {
    let service = Arc::new(QueryService::new(
        (**storage).clone(),
        embedder.clone(),
        index.clone(),
    ));
    let (client, _server) = wrap_as_client(MinistrServer::new(service)).await;
    let result = call_tool(
        &client,
        "ministr_survey",
        json!({ "query": query, "top_k": top_k }),
    )
    .await;
    count_tokens(&response_text(&result.content))
}

// ---------------------------------------------------------------------------
// The measurement
// ---------------------------------------------------------------------------

/// Reproducible report: prints a per-query and aggregate table of the REAL token
/// economics (grep+read vs a `ministr_survey` lookup) over `eval/corpus-code`.
/// This is the source of the published figure; it is also a regression gate
/// (see the asserts at the end). Run with `--nocapture` to see the table.
#[tokio::test]
#[allow(clippy::cast_precision_loss)]
async fn report_real_token_economics() {
    let Some((dir, files)) = load_code_corpus() else {
        return;
    };
    let queries = load_queries();
    if queries.is_empty() {
        eprintln!("Skipping: no ground-truth-code queries found");
        return;
    }

    let (storage, index, embedder) = ingest_code_corpus(&dir).await;
    let top_k = 5;

    let corpus_files = files.len();
    let corpus_tokens: usize = files.iter().map(|(_, c)| count_tokens(c)).sum();

    eprintln!();
    eprintln!(
        "=== Real token economics — {corpus_files} files, {corpus_tokens} tokens total (eval/corpus-code) ==="
    );
    eprintln!(
        "(ministr = literal ministr_survey response, top_k={top_k}; grep+read = whole candidate files)"
    );
    eprintln!(
        "{:<58} {:>5} {:>9} {:>9} {:>8}",
        "query", "files", "grep+rd", "ministr", "saved"
    );

    let mut total_grep = 0usize;
    let mut total_ministr = 0usize;
    let mut wins = 0usize;
    let mut measured = 0usize;

    for q in &queries {
        let (matched, grep_tokens) = grep_read_cost(q, &files);
        // A query whose terms match nothing on disk isn't a meaningful grep+read
        // comparison (the agent would refine), so skip it from the aggregate.
        if matched == 0 {
            continue;
        }
        let ministr_tokens = ministr_survey_cost(&storage, &index, &embedder, q, top_k).await;

        total_grep += grep_tokens;
        total_ministr += ministr_tokens;
        measured += 1;
        if ministr_tokens < grep_tokens {
            wins += 1;
        }

        let saved = if grep_tokens > 0 {
            100.0 * (1.0 - ministr_tokens as f64 / grep_tokens as f64)
        } else {
            0.0
        };
        let short: String = q.chars().take(56).collect();
        eprintln!("{short:<58} {matched:>5} {grep_tokens:>9} {ministr_tokens:>9} {saved:>7.0}%");
    }

    assert!(
        measured > 0,
        "no queries matched any file — corpus/query mismatch"
    );

    let agg_saved = 100.0 * (1.0 - total_ministr as f64 / total_grep as f64);
    let avg_grep = total_grep / measured;
    let avg_ministr = total_ministr / measured;
    eprintln!("{:-<92}", "");
    eprintln!(
        "AGGREGATE over {measured} queries: grep+read {total_grep} tok  vs  ministr {total_ministr} tok  →  {agg_saved:.1}% fewer"
    );
    eprintln!(
        "PER-LOOKUP MEAN: grep+read {avg_grep} tok  vs  ministr {avg_ministr} tok   ({wins}/{measured} queries ministr cheaper)"
    );
    eprintln!(
        "(deterministic: committed corpus + hash embedder + cl100k tokenizer; re-seed docs only when one of those changes)"
    );

    // --- Regression gate (robust, honest invariants — NOT a fixed headline %) ---

    // 1. A ministr lookup is always bounded well under the MCP per-result budget
    //    (22k tokens) — the property that makes the cost predictable.
    assert!(
        avg_ministr < 22_000,
        "a survey lookup must stay well under the 22k budget, got {avg_ministr}"
    );

    // 2. Directionally, reading whole candidate files costs more than a targeted
    //    survey in aggregate over this real corpus. This is the real claim — it
    //    holds without inventing a percentage.
    assert!(
        total_ministr < total_grep,
        "aggregate grep+read ({total_grep}) should exceed aggregate ministr ({total_ministr})"
    );

    // 3. ministr is cheaper on a clear majority of real queries.
    assert!(
        wins * 2 > measured,
        "ministr should be cheaper on a majority of queries, won {wins}/{measured}"
    );
}
