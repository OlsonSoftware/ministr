//! [`AstSparseEncoder`] — zero-model sparse encoding over AST-derived roles.
//!
//! A BM25F-style lexical encoder where the fields are the structural roles
//! ministr already derives from indexed content: the definition **name**
//! (from the vector ID's anchor), the **doc comment** prose, the
//! **signature** line, and the **body**. Field weights are pre-declared
//! (3.0 / 2.0 / 1.5 / 0.5, never tuned). Terms are camelCase/snake_case
//! subtokens *plus* the raw token, hashed with FNV-1a; the same analyzer
//! runs on corpus and query text. Documents carry saturating-TF BM25F
//! folds; queries carry corpus IDF weights derived live from the inverted
//! index's posting lists (Lucene's design: `idf(docFreq, docCount)` is
//! computed at query time from index statistics, so deletes and re-ingests
//! can never leave a stale IDF table behind).
//!
//! Measured against the bge-m3 neural sparse encoder under a
//! byte-deterministic gate (optimize-ingest Phases 9–10, 2026-06): hybrid
//! nDCG@5 .891 vs .833 on an 871-query Rust eval and .960 vs .927 on a
//! 22-query six-language eval; on a never-seen third corpus (641 queries)
//! dense .611 → hybrid .853; on 100 hostile paraphrases with lexical
//! overlap deliberately broken, dense .367 → .453 — it is not just
//! verbatim matching. Encoding ~7,300 entries took 0.17–0.21 s vs ~10 min
//! for bge-m3 (≈3,000×), with no model download and full determinism.
//!
//! Honest caveats carried from the measurements:
//! 1. On the 26 *human-authored* queries it scores .839 vs bge-m3's .858
//!    (−.019) — neural sparse keeps a small semantic-expansion edge.
//! 2. bge-m3 was never measured on the paraphrase set (runs cut for cost),
//!    so its robustness there is unquantified.
//! 3. The BM25F fold uses saturating TF *without* length normalization — a
//!    declared simplification, not an oversight.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{SparseEmbedder, SparseVector};
use crate::error::IndexError;
use crate::index::InvertedIndex;

/// Field weight for the definition-name role (vector-ID anchor subtokens).
const W_NAME: f32 = 3.0;
/// Field weight for leading doc-comment prose.
const W_DOC: f32 = 2.0;
/// Field weight for the first source (signature) line.
const W_SIG: f32 = 1.5;
/// Field weight for the remaining body.
const W_BODY: f32 = 0.5;

/// FNV-1a 32-bit hash — the term-id space shared by corpus and queries.
fn fnv1a(token: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in token.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// camelCase/snake_case subtokens *plus* the raw token, lowercased.
///
/// Keeping the raw form alongside the splits matters: the model bake-off
/// showed splitting alone helps weak tokenizers and hurts strong ones, and
/// queries often quote identifiers verbatim.
fn subtokens(ident: &str) -> Vec<String> {
    let mut out = vec![ident.to_lowercase()];
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in ident.chars() {
        if ch == '_' {
            if !cur.is_empty() {
                out.push(cur.to_lowercase());
                cur.clear();
            }
            prev_lower = false;
        } else if ch.is_uppercase() && prev_lower {
            out.push(cur.to_lowercase());
            cur = ch.to_string();
            prev_lower = false;
        } else {
            prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out.dedup();
    out
}

/// Shared analyzer: alphanumeric/underscore words → subtokens + raw, length > 1.
fn analyze(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if raw.is_empty() {
            continue;
        }
        for t in subtokens(raw) {
            if t.len() > 1 {
                out.push(t);
            }
        }
    }
    out
}

/// Split a section's text into (doc prose, signature line, body) by the first
/// line that looks like source. Heuristic by design: ministr places extracted
/// doc text before the code, so "first source-looking line" approximates the
/// signature without re-parsing.
fn split_roles(text: &str) -> (String, String, String) {
    const SOURCE_PREFIXES: &[&str] = &[
        "///",
        "//!",
        "//",
        "#",
        "/*",
        "* ",
        "pub ",
        "fn ",
        "def ",
        "class ",
        "func ",
        "function ",
        "struct ",
        "impl ",
        "public ",
        "private ",
        "type ",
        "@",
        "package ",
        "import ",
        "use ",
        "template",
    ];
    let mut doc = String::new();
    let mut sig = String::new();
    let mut body = String::new();
    let mut in_source = false;
    for line in text.lines() {
        let t = line.trim_start();
        let looks_source = SOURCE_PREFIXES.iter().any(|p| t.starts_with(p));
        if !in_source && looks_source {
            in_source = true;
            sig = line.to_string();
            continue;
        }
        if in_source {
            body.push_str(line);
            body.push(' ');
        } else {
            doc.push_str(line);
            doc.push(' ');
        }
    }
    (doc, sig, body)
}

/// Derive the definition-name role from a vector ID.
///
/// IDs look like `section::path.rs#module::func` or
/// `symbol-stub::sym-config::MinistrConfig`; the anchor after the last `#`
/// (or, failing that, the last `::` segment) is the owning definition path.
fn name_from_id(id: &str) -> String {
    let anchor = id
        .rsplit_once('#')
        .map_or_else(|| id.rsplit("::").next().unwrap_or(""), |(_, a)| a);
    anchor.replace("::", " ")
}

/// Zero-model sparse encoder over AST-derived roles (see module docs).
///
/// Holds the corpus's [`InvertedIndex`] so query-time IDF derives from the
/// live posting lists — document frequency *is* the persisted sidecar, so
/// there is no separate IDF table to drift out of coherence on deletes or
/// re-ingests.
pub struct AstSparseEncoder {
    index: Arc<InvertedIndex>,
}

impl AstSparseEncoder {
    /// Create an encoder backed by the corpus's inverted index.
    #[must_use]
    pub fn new(index: Arc<InvertedIndex>) -> Self {
        Self { index }
    }

    /// Saturating-TF BM25F document vector over the four role fields.
    /// Deterministic: terms accumulate in a `BTreeMap`, so the output order
    /// is the term-id order regardless of input order.
    fn doc_vector(name: &str, doc: &str, sig: &str, body: &str) -> (Vec<u32>, Vec<f32>) {
        let fields: [(f32, &str); 4] = [(W_NAME, name), (W_DOC, doc), (W_SIG, sig), (W_BODY, body)];
        let mut acc: BTreeMap<u32, f32> = BTreeMap::new();
        for (w, field) in fields {
            let mut tf: BTreeMap<u32, f32> = BTreeMap::new();
            for tok in analyze(field) {
                *tf.entry(fnv1a(&tok)).or_insert(0.0) += 1.0;
            }
            for (term, f) in tf {
                *acc.entry(term).or_insert(0.0) += w * f / (1.0 + f);
            }
        }
        acc.into_iter().unzip()
    }
}

impl SparseEmbedder for AstSparseEncoder {
    /// Document encoding without the name role (no ID available). Prefer
    /// [`SparseEmbedder::embed_sparse_docs`], which carries the vector ID and
    /// therefore the strongest (name) field.
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        Ok(texts
            .iter()
            .map(|t| {
                let (doc, sig, body) = split_roles(t);
                let (indices, values) = Self::doc_vector("", &doc, &sig, &body);
                SparseVector { indices, values }
            })
            .collect())
    }

    fn embed_sparse_docs(&self, entries: &[(&str, &str)]) -> Result<Vec<SparseVector>, IndexError> {
        Ok(entries
            .iter()
            .map(|(id, text)| {
                let name = name_from_id(id);
                let (doc, sig, body) = split_roles(text);
                let (indices, values) = Self::doc_vector(&name, &doc, &sig, &body);
                SparseVector { indices, values }
            })
            .collect())
    }

    /// Query encoding: each analyzed term carries its corpus IDF, derived
    /// live from the inverted index's document frequencies.
    fn embed_sparse_query(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        #[allow(clippy::cast_precision_loss)] // doc counts are far below f32 precision loss
        let n_docs = self.index.doc_count() as f32;
        Ok(texts
            .iter()
            .map(|t| {
                let mut acc: BTreeMap<u32, f32> = BTreeMap::new();
                for tok in analyze(t) {
                    let h = fnv1a(&tok);
                    #[allow(clippy::cast_precision_loss)]
                    let df = self.index.doc_frequency(h) as f32;
                    let idf = (1.0 + (n_docs - df + 0.5) / (df + 0.5)).ln();
                    acc.entry(h).or_insert(idf);
                }
                let (indices, values): (Vec<u32>, Vec<f32>) = acc.into_iter().unzip();
                SparseVector { indices, values }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SparseIndex;

    #[test]
    fn analyzer_keeps_raw_token_and_subtokens() {
        let toks = analyze("scan_space_inner camelCase");
        assert!(toks.contains(&"scan_space_inner".to_string()), "raw kept");
        assert!(toks.contains(&"scan".to_string()));
        assert!(toks.contains(&"space".to_string()));
        assert!(toks.contains(&"inner".to_string()));
        assert!(toks.contains(&"camelcase".to_string()), "raw kept");
        assert!(toks.contains(&"camel".to_string()));
        assert!(toks.contains(&"case".to_string()));
    }

    #[test]
    fn split_roles_separates_doc_signature_body() {
        let text = "Computes the retry delay.\npub fn retry_delay(n: u32) -> u64 {\n    n * 2\n}\n";
        let (doc, sig, body) = split_roles(text);
        assert!(doc.contains("Computes the retry delay."));
        assert!(sig.contains("pub fn retry_delay"));
        assert!(body.contains("n * 2"));
    }

    #[test]
    fn name_from_id_handles_sections_and_symbols() {
        assert_eq!(
            name_from_id("section::lib.rs#config::MinistrConfig"),
            "config MinistrConfig"
        );
        assert_eq!(
            name_from_id("symbol-stub::sym-config::RetryPolicy"),
            "RetryPolicy"
        );
    }

    #[test]
    fn doc_encoding_is_deterministic() {
        let index = Arc::new(InvertedIndex::new());
        let enc = AstSparseEncoder::new(index);
        let entries = [("section::a.rs#m::f", "Doc text.\nfn f() {}\nbody body")];
        let a = enc.embed_sparse_docs(&entries).unwrap();
        let b = enc.embed_sparse_docs(&entries).unwrap();
        assert_eq!(a, b, "same input must produce byte-identical vectors");
        assert!(!a[0].indices.is_empty());
        assert!(
            a[0].indices.windows(2).all(|w| w[0] < w[1]),
            "term ids ascend (BTreeMap order)"
        );
    }

    #[test]
    fn name_role_outweighs_body_mentions() {
        // A term appearing once in the NAME field must outweigh the same term
        // appearing once in the BODY of another doc (3.0 vs 0.5 fold).
        let index = Arc::new(InvertedIndex::new());
        let enc = AstSparseEncoder::new(Arc::clone(&index));
        let entries = [
            ("section::a.rs#mod::retry_delay", "fn retry_delay() {}\n"),
            (
                "section::b.rs#mod::other_thing",
                "fn other_thing() {}\n retry delay mentioned\n",
            ),
        ];
        let vecs = enc.embed_sparse_docs(&entries).unwrap();
        for ((id, _), v) in entries.iter().zip(vecs.iter()) {
            index.insert_sparse(id, &v.indices, &v.values).unwrap();
        }
        let q = enc.embed_sparse_query(&["retry delay"]).unwrap();
        let results = index
            .search_sparse(&q[0].indices, &q[0].values, 10)
            .unwrap();
        assert_eq!(
            results[0].id, "section::a.rs#mod::retry_delay",
            "name-field match ranks first"
        );
    }

    #[test]
    fn query_idf_downweights_common_terms() {
        // "common" appears in every doc; "rare" in one. After indexing, the
        // query weight for "rare" must exceed the weight for "common".
        let index = Arc::new(InvertedIndex::new());
        let enc = AstSparseEncoder::new(Arc::clone(&index));
        let entries: Vec<(String, String)> = (0..10)
            .map(|i| {
                let text = if i == 0 {
                    "fn f0() {}\n common rare\n".to_string()
                } else {
                    format!("fn f{i}() {{}}\n common\n")
                };
                (format!("section::x.rs#m::f{i}"), text)
            })
            .collect();
        let refs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(id, t)| (id.as_str(), t.as_str()))
            .collect();
        let vecs = enc.embed_sparse_docs(&refs).unwrap();
        for ((id, _), v) in refs.iter().zip(vecs.iter()) {
            index.insert_sparse(id, &v.indices, &v.values).unwrap();
        }
        let q = enc.embed_sparse_query(&["common rare"]).unwrap();
        let weight = |term: &str| -> f32 {
            let h = fnv1a(term);
            q[0].indices
                .iter()
                .position(|&i| i == h)
                .map(|p| q[0].values[p])
                .expect("term present in query vector")
        };
        assert!(
            weight("rare") > weight("common"),
            "rare {} must outweigh common {}",
            weight("rare"),
            weight("common")
        );
    }
}
