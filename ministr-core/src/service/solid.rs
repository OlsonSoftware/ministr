//! Deterministic SOLID-violation detection.
//!
//! Surfaces four classes of structural smell from the existing symbol /
//! reference / embedding index without any LLM judgement:
//!
//! * **DRY / OCP** — Type-4 semantic clone clusters (embedding cosine ≥ τ
//!   AND callee-set Jaccard ≥ τ). Suggests an extracted abstraction.
//! * **SRP** — pseudo-LCOM4: a container whose methods split into multiple
//!   weakly-connected cohesion components when linked by shared callees.
//! * **ISP** — a fat interface where most implementors override only a small
//!   fraction of the methods.
//! * **DIP** — a high-level consumer depending on a concrete cross-package
//!   target whose implemented trait the consumer never references.
//!
//! Every heuristic consumes only normalised graph signals (`kind`,
//! [`RefKind`], `module_path`, callee/use sets, embeddings) that
//! tree-sitter ingestion produces for every supported language, so the
//! detector is language-agnostic — callers tune `container_kinds` /
//! `interface_kinds` for languages whose kind tokens differ from the
//! defaults.

use std::collections::{HashMap, HashSet};

use tracing::{instrument, warn};

use crate::storage::{Storage, SymbolFilter, SymbolRecord};
use crate::types::{RefKind, SymbolId, VectorId};

use super::code::is_test_path;
use super::{
    QueryError, QueryService, SolidComponent, SolidEdge, SolidFinding, SolidParams, SolidPrinciple,
    SolidSymbolRef, cosine_similarity,
};

impl QueryService {
    /// Run all enabled SOLID-violation detectors against the corpus.
    ///
    /// See the module docs for the heuristics behind each principle.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] on database errors.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, params))]
    pub async fn detect_solid_violations(
        &self,
        params: &SolidParams,
    ) -> Result<Vec<SolidFinding>, QueryError> {
        let filter = SymbolFilter {
            kind: params.kind.clone(),
            module: params.module.clone(),
            ..Default::default()
        };
        let all_symbols = self.storage.list_symbols(&filter).await?;

        let by_id: HashMap<SymbolId, SymbolRecord> = all_symbols
            .iter()
            .map(|s| (s.id.clone(), s.clone()))
            .collect();

        let candidates: Vec<&SymbolRecord> = all_symbols
            .iter()
            .filter(|s| {
                let lines = s.line_end.saturating_sub(s.line_start).saturating_add(1);
                lines >= params.min_lines
            })
            .collect();

        // Outgoing edges per symbol. We sweep `all_symbols` (not just the
        // candidate set) because `min_lines` is a *candidate* filter — a
        // 1-line re-export still produces a real cross-package edge that
        // matters for cycle detection.
        let mut callees_map: HashMap<SymbolId, HashSet<SymbolId>> = HashMap::new();
        let mut uses_map: HashMap<SymbolId, HashSet<SymbolId>> = HashMap::new();
        for sym in &all_symbols {
            let refs = self.storage.query_refs(&sym.id, None).await?;
            let mut calls: HashSet<SymbolId> = HashSet::new();
            let mut uses: HashSet<SymbolId> = HashSet::new();
            for r in refs {
                if r.from_symbol_id != sym.id {
                    continue;
                }
                match r.ref_kind {
                    RefKind::Calls => {
                        calls.insert(r.to_symbol_id);
                    }
                    RefKind::Uses | RefKind::Imports => {
                        uses.insert(r.to_symbol_id);
                    }
                    RefKind::Implements | RefKind::Bridge => {}
                }
            }
            callees_map.insert(sym.id.clone(), calls);
            uses_map.insert(sym.id.clone(), uses);
        }

        // Incoming `Implements` edges for every interface candidate — driven
        // by the configured `interface_kinds` so ISP and DIP both see them.
        let mut implementors_of: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        let mut implements_of: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        let interfaces: Vec<&SymbolRecord> = all_symbols
            .iter()
            .filter(|s| params.interface_kinds.contains(&s.kind))
            .collect();
        for iface in &interfaces {
            let refs = self
                .storage
                .query_refs(&iface.id, Some(RefKind::Implements))
                .await?;
            let mut impls: Vec<SymbolId> = Vec::new();
            for r in refs {
                if r.to_symbol_id == iface.id {
                    impls.push(r.from_symbol_id.clone());
                    implements_of
                        .entry(r.from_symbol_id)
                        .or_default()
                        .push(iface.id.clone());
                }
            }
            implementors_of.insert(iface.id.clone(), impls);
        }

        // Batch-fetch full-dim vectors for every candidate.
        let vid_strings: Vec<String> = candidates
            .iter()
            .map(|s| VectorId::symbol_stub(s.id.as_ref()).as_str().to_string())
            .collect();
        let vid_refs: Vec<&str> = vid_strings.iter().map(String::as_str).collect();
        let raw_vecs = self.storage.get_full_dim_vectors(&vid_refs).await?;
        let mut vectors: HashMap<SymbolId, Vec<f32>> = HashMap::with_capacity(raw_vecs.len());
        for (vid, mut vec) in raw_vecs {
            if let Some(content) = vid.strip_prefix("symbol-stub::") {
                normalize_in_place(&mut vec);
                vectors.insert(SymbolId(content.to_string()), vec);
            }
        }

        let wants =
            |p: SolidPrinciple| params.principles.is_empty() || params.principles.contains(&p);

        // One-shot corpus root computation, so every package-aware
        // detector sees the same workspace-relative paths regardless of
        // whether the corpus uses absolute or relative file paths.
        let corpus_root = corpus_root_prefix(&all_symbols);

        let mut findings: Vec<SolidFinding> = Vec::new();

        if wants(SolidPrinciple::DryOcp) {
            findings.extend(detect_redundancy(
                &candidates,
                &callees_map,
                &uses_map,
                &vectors,
                params,
            ));
        }
        if wants(SolidPrinciple::Srp) {
            findings.extend(detect_srp(&all_symbols, &callees_map, &vectors, params));
        }
        if wants(SolidPrinciple::Isp) {
            findings.extend(detect_isp(&all_symbols, &implementors_of, &by_id, params));
        }
        if wants(SolidPrinciple::Dip) {
            findings.extend(detect_dip(
                &candidates,
                &callees_map,
                &uses_map,
                &implements_of,
                &by_id,
                &corpus_root,
                params,
            ));
        }
        if wants(SolidPrinciple::ShotgunSurgery) {
            findings.extend(detect_shotgun_surgery(
                &candidates,
                &callees_map,
                &uses_map,
                &corpus_root,
                params,
            ));
        }
        if wants(SolidPrinciple::CyclicDependency) {
            findings.extend(detect_cyclic_dependency(
                &all_symbols,
                &by_id,
                &callees_map,
                &uses_map,
                &corpus_root,
                params,
            ));
        }

        if findings.len() > params.limit {
            findings.truncate(params.limit);
        }
        Ok(findings)
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn to_ref(s: &SymbolRecord) -> SolidSymbolRef {
    SolidSymbolRef {
        symbol_id: s.id.0.clone(),
        name: s.name.clone(),
        kind: s.kind.clone(),
        file: s.file_path.clone(),
        line: s.line_start,
    }
}

/// Truncate `items` to at most `cap` entries, returning `(kept, omitted)`.
/// `cap` of 0 keeps everything (treating the cap as "off") so 0 stays a
/// safe override.
fn truncate_with_count<T>(items: Vec<T>, cap: usize) -> (Vec<T>, usize) {
    if cap == 0 || items.len() <= cap {
        return (items, 0);
    }
    let omitted = items.len() - cap;
    let mut kept = items;
    kept.truncate(cap);
    (kept, omitted)
}

fn normalize_in_place(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Count parameters in a signature's first parenthesised group as a coarse
/// bucket (0, 1, 2, 3, 4, 5+) for pairwise candidate pruning.
fn arity_bucket(signature: &str) -> u32 {
    let bytes = signature.as_bytes();
    let mut depth: i32 = 0;
    let mut commas: u32 = 0;
    let mut started = false;
    let mut had_content = false;
    for &b in bytes {
        match b {
            b'(' => {
                if depth == 0 {
                    started = true;
                }
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 && started {
                    break;
                }
            }
            b',' if depth == 1 => {
                commas += 1;
                had_content = true;
            }
            b' ' | b'\t' | b'\n' => {}
            _ if depth >= 1 => {
                had_content = true;
            }
            _ => {}
        }
    }
    if !started || !had_content {
        0
    } else {
        (commas + 1).min(5)
    }
}

/// Container-method matcher.
///
/// A symbol `m` is a method of container `c` when its `module_path` equals
/// the container's qualified path (`c.module_path::c.name`, or just `c.name`
/// when the module is empty) and they live in the same file.
fn methods_of<'a>(
    container: &SymbolRecord,
    all_symbols: &'a [SymbolRecord],
) -> Vec<&'a SymbolRecord> {
    let expected = if container.module_path.is_empty() {
        container.name.clone()
    } else {
        format!("{}::{}", container.module_path, container.name)
    };
    all_symbols
        .iter()
        .filter(|s| {
            (s.kind == "function" || s.kind == "method")
                && s.module_path == expected
                && s.file_path == container.file_path
        })
        .collect()
}

/// Returns true for method names that are universally idiomatic across the
/// supported languages — Rust trait conformance (`fmt`, `clone`, `eq`, ...),
/// constructor conventions (`new`, `default`, `build`), Serde plumbing
/// (`serialize`, `deserialize`), and universal entry points (`main`).
///
/// The Shotgun-Surgery detector treats these as noise by default: a 50-site
/// `new` cluster across unrelated types is the language asking us to write
/// constructors, not a fan-out smell.
fn is_conventional_method_name(name: &str) -> bool {
    matches!(
        name,
        // Construction
        "new" | "default" | "build" | "create" | "init"
        // Universal entry points / scripts
        | "main" | "run"
        // Rust core traits
        | "fmt" | "clone" | "clone_from" | "eq" | "ne"
        | "hash" | "cmp" | "partial_cmp" | "lt" | "le" | "gt" | "ge"
        | "drop" | "deref" | "deref_mut"
        | "as_ref" | "as_mut" | "as_str" | "as_bytes"
        | "from" | "into" | "try_from" | "try_into"
        | "borrow" | "borrow_mut" | "to_owned" | "to_string"
        | "is_empty" | "len"
        // Iteration
        | "iter" | "iter_mut" | "into_iter" | "next"
        // Serde / parsing universals
        | "serialize" | "deserialize" | "parse"
    )
}

/// Compute the longest path-component prefix shared by every file in
/// `all_symbols`. Path-component-aware: only whole segments shared by *all*
/// inputs count. Returns an empty string when there's no usable shared
/// prefix (e.g. one symbol, or filesystem-root-only commonality).
fn corpus_root_prefix(all_symbols: &[SymbolRecord]) -> String {
    if all_symbols.is_empty() {
        return String::new();
    }
    let split_segments =
        |p: &str| -> Vec<String> { p.split('/').map(str::to_owned).collect::<Vec<_>>() };
    let mut shared: Vec<String> = split_segments(&all_symbols[0].file_path);
    for sym in &all_symbols[1..] {
        let segs = split_segments(&sym.file_path);
        let common = shared
            .iter()
            .zip(segs.iter())
            .take_while(|(a, b)| a == b)
            .count();
        shared.truncate(common);
        if shared.is_empty() {
            return String::new();
        }
    }
    // The shared segments make up the corpus root. We deliberately do NOT
    // strip a trailing file-basename: when every file in a small corpus
    // happens to live in one directory, the package segment is the
    // basename and stripping it would collapse every file to the same
    // empty package. The first segment *after* this prefix is the package
    // — see `package_of`.
    let mut out = shared.join("/");
    if !out.is_empty() {
        out.push('/');
    }
    out
}

/// The "package" segment of a file path — the first path component
/// remaining after the corpus root prefix is stripped.
///
/// For a Rust workspace, this is usually the crate name
/// (`ministr-api`). For a monorepo with `packages/<name>/...`, this is
/// `packages` — adjust the corpus layout or pass an empty `root` to opt
/// out of stripping.
fn package_of(path: &str, root: &str) -> String {
    let stripped = path.strip_prefix(root).unwrap_or(path);
    stripped
        .split('/')
        .find(|seg| !seg.is_empty())
        .unwrap_or("")
        .to_string()
}

// ── union-find ──────────────────────────────────────────────────────────

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression.
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

// ── detectors ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn detect_redundancy(
    candidates: &[&SymbolRecord],
    callees_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    uses_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    vectors: &HashMap<SymbolId, Vec<f32>>,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    // Bucket candidates that have vectors by (kind, arity).
    let mut buckets: HashMap<(String, u32), Vec<usize>> = HashMap::new();
    for (idx, sym) in candidates.iter().enumerate() {
        if !vectors.contains_key(&sym.id) {
            continue;
        }
        let key = (sym.kind.clone(), arity_bucket(&sym.signature));
        buckets.entry(key).or_default().push(idx);
    }

    let mut findings: Vec<SolidFinding> = Vec::new();

    for indices in buckets.values() {
        if indices.len() < 2 {
            continue;
        }
        let pair_count = indices.len() * (indices.len() - 1) / 2;
        if pair_count > params.max_pairs {
            warn!(
                bucket_size = indices.len(),
                pairs = pair_count,
                max_pairs = params.max_pairs,
                "ministr_solid: skipping redundancy bucket — pair count exceeds max_pairs"
            );
            continue;
        }

        let mut uf = UnionFind::new(indices.len());
        // (local_a, local_b, cos, jac) — kept to compute cluster averages.
        let mut edges: Vec<(usize, usize, f32, f32)> = Vec::new();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = candidates[indices[i]];
                let b = candidates[indices[j]];
                let va = &vectors[&a.id];
                let vb = &vectors[&b.id];
                if va.len() != vb.len() {
                    continue;
                }
                let cos = cosine_similarity(va, vb);
                if cos < params.similarity_threshold {
                    continue;
                }
                let empty: HashSet<SymbolId> = HashSet::new();
                let a_set: HashSet<&SymbolId> = callees_map
                    .get(&a.id)
                    .unwrap_or(&empty)
                    .iter()
                    .chain(uses_map.get(&a.id).unwrap_or(&empty).iter())
                    .collect();
                let b_set: HashSet<&SymbolId> = callees_map
                    .get(&b.id)
                    .unwrap_or(&empty)
                    .iter()
                    .chain(uses_map.get(&b.id).unwrap_or(&empty).iter())
                    .collect();
                let inter = a_set.intersection(&b_set).count();
                let union = a_set.union(&b_set).count();
                #[allow(clippy::cast_precision_loss)]
                let jac = if union == 0 {
                    0.0
                } else {
                    inter as f32 / union as f32
                };
                if jac < params.jaccard_threshold {
                    continue;
                }
                uf.union(i, j);
                edges.push((i, j, cos, jac));
            }
        }

        // Group local indices by union-find root.
        let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..indices.len() {
            let root = uf.find(i);
            clusters.entry(root).or_default().push(i);
        }
        for local_indices in clusters.into_values() {
            if local_indices.len() < 2 {
                continue;
            }
            let cluster_set: HashSet<usize> = local_indices.iter().copied().collect();
            let mut sum_cos = 0.0f32;
            let mut sum_jac = 0.0f32;
            let mut edge_count = 0u32;
            for &(la, lb, cos, jac) in &edges {
                if cluster_set.contains(&la) && cluster_set.contains(&lb) {
                    sum_cos += cos;
                    sum_jac += jac;
                    edge_count += 1;
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let n = edge_count.max(1) as f32;
            let members: Vec<&SymbolRecord> = local_indices
                .iter()
                .map(|li| candidates[indices[*li]])
                .collect();
            let canonical = members
                .iter()
                .max_by_key(|s| {
                    (
                        s.line_end.saturating_sub(s.line_start),
                        std::cmp::Reverse(s.file_path.clone()),
                    )
                })
                .copied()
                .unwrap_or(members[0]);
            let file_set: HashSet<&String> = members.iter().map(|m| &m.file_path).collect();
            let cross_module = file_set.len() > 1;
            let total = members.len();
            let refs: Vec<SolidSymbolRef> = members.iter().map(|s| to_ref(s)).collect();
            let (kept, omitted) = truncate_with_count(refs, params.representative_count);
            findings.push(SolidFinding::Redundancy {
                principle: SolidPrinciple::DryOcp,
                members: kept,
                members_omitted: omitted,
                members_total: total,
                canonical: to_ref(canonical),
                avg_cosine: sum_cos / n,
                avg_jaccard: sum_jac / n,
                cross_module,
            });
        }
    }

    findings
}

fn detect_srp(
    all_symbols: &[SymbolRecord],
    callees_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    vectors: &HashMap<SymbolId, Vec<f32>>,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    let containers: Vec<&SymbolRecord> = all_symbols
        .iter()
        .filter(|s| params.container_kinds.contains(&s.kind))
        .collect();

    // Multiple containers can resolve to the same method set — a Rust type
    // often has both a `struct` symbol and one or more `impl` blocks sharing
    // the qualified path. Dedupe by `(file, module_path, name)` and keep the
    // one with the smallest start-line as the canonical representative.
    let mut by_key: HashMap<(String, String, String), &SymbolRecord> = HashMap::new();
    for c in containers {
        let key = (c.file_path.clone(), c.module_path.clone(), c.name.clone());
        by_key
            .entry(key)
            .and_modify(|prev| {
                if c.line_start < prev.line_start {
                    *prev = c;
                }
            })
            .or_insert(c);
    }

    let mut findings: Vec<SolidFinding> = Vec::new();
    for container in by_key.into_values() {
        let methods = methods_of(container, all_symbols);
        if methods.len() < 4 {
            continue;
        }

        let n = methods.len();
        let mut uf = UnionFind::new(n);
        for i in 0..n {
            for j in (i + 1)..n {
                let a = methods[i];
                let b = methods[j];
                let empty: HashSet<SymbolId> = HashSet::new();
                let a_callees = callees_map.get(&a.id).unwrap_or(&empty);
                let b_callees = callees_map.get(&b.id).unwrap_or(&empty);
                let shared = a_callees.intersection(b_callees).count();
                let mut connected = shared >= 1;
                if !connected
                    && let (Some(va), Some(vb)) = (vectors.get(&a.id), vectors.get(&b.id))
                    && va.len() == vb.len()
                {
                    let cos = cosine_similarity(va, vb);
                    if cos >= params.srp_cohesion_threshold {
                        connected = true;
                    }
                }
                if connected {
                    uf.union(i, j);
                }
            }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let r = uf.find(i);
            groups.entry(r).or_default().push(i);
        }
        // The SRP signal is "two or more cohesion clusters, each non-trivial."
        // A single big cluster plus scattered singletons isn't a split
        // candidate — it's a god-object / wide-API shape that this detector
        // is intentionally silent about (see `ministr_dead` / impact for
        // those). Require ≥ 2 components of size ≥ 2.
        let real_clusters = groups.values().filter(|g| g.len() >= 2).count();
        if real_clusters >= 2 {
            let mut raw: Vec<Vec<SolidSymbolRef>> = groups
                .into_values()
                .map(|idxs| idxs.into_iter().map(|i| to_ref(methods[i])).collect())
                .collect();
            raw.sort_by_key(|c| std::cmp::Reverse(c.len()));
            let components: Vec<SolidComponent> = raw
                .into_iter()
                .map(|members| {
                    let size = members.len();
                    let (kept, omitted) = truncate_with_count(members, params.representative_count);
                    SolidComponent {
                        size,
                        members: kept,
                        members_omitted: omitted,
                    }
                })
                .collect();
            findings.push(SolidFinding::LowCohesion {
                principle: SolidPrinciple::Srp,
                container: to_ref(container),
                method_count: methods.len(),
                components,
            });
        }
    }
    findings
}

#[allow(clippy::too_many_lines)]
fn detect_isp(
    all_symbols: &[SymbolRecord],
    implementors_of: &HashMap<SymbolId, Vec<SymbolId>>,
    by_id: &HashMap<SymbolId, SymbolRecord>,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    let interfaces: Vec<&SymbolRecord> = all_symbols
        .iter()
        .filter(|s| params.interface_kinds.contains(&s.kind))
        .collect();

    let mut findings: Vec<SolidFinding> = Vec::new();
    for iface in interfaces {
        let methods = methods_of(iface, all_symbols);
        if methods.len() < params.isp_min_methods {
            continue;
        }
        let method_names: HashSet<String> = methods.iter().map(|m| m.name.clone()).collect();

        let implementors = implementors_of.get(&iface.id).cloned().unwrap_or_default();
        if implementors.is_empty() {
            continue;
        }

        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let max_overlap =
            ((methods.len() as f32) * params.isp_max_overlap_fraction).floor() as usize;

        let mut under_using: Vec<SolidSymbolRef> = Vec::new();
        let mut covered: HashSet<String> = HashSet::new();

        for impl_id in &implementors {
            let Some(impl_sym) = by_id.get(impl_id) else {
                continue;
            };
            let impl_methods: HashSet<String> = methods_of(impl_sym, all_symbols)
                .into_iter()
                .map(|m| m.name.clone())
                .collect();
            let overlap: HashSet<&String> = impl_methods.intersection(&method_names).collect();
            for m in &overlap {
                covered.insert((*m).clone());
            }
            if overlap.len() <= max_overlap {
                under_using.push(to_ref(impl_sym));
            }
        }

        if !implementors.is_empty() && under_using.len() * 2 >= implementors.len() {
            let mut unused: Vec<String> = method_names.difference(&covered).cloned().collect();
            unused.sort();
            let (unused_kept, unused_omitted) =
                truncate_with_count(unused, params.representative_count);
            let (impl_kept, impl_omitted) =
                truncate_with_count(under_using, params.representative_count);
            findings.push(SolidFinding::FatInterface {
                principle: SolidPrinciple::Isp,
                interface: to_ref(iface),
                method_count: methods.len(),
                unused_methods: unused_kept,
                unused_methods_omitted: unused_omitted,
                under_using_implementors: impl_kept,
                under_using_implementors_omitted: impl_omitted,
            });
        }
    }
    findings
}

#[allow(clippy::too_many_lines)]
fn detect_dip(
    candidates: &[&SymbolRecord],
    callees_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    uses_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    implements_of: &HashMap<SymbolId, Vec<SymbolId>>,
    by_id: &HashMap<SymbolId, SymbolRecord>,
    corpus_root: &str,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    let mut findings: Vec<SolidFinding> = Vec::new();
    for consumer in candidates {
        if consumer.kind != "function" && consumer.kind != "method" {
            continue;
        }
        let consumer_pkg = package_of(&consumer.file_path, corpus_root);
        let empty: HashSet<SymbolId> = HashSet::new();
        let edges: HashSet<&SymbolId> = callees_map
            .get(&consumer.id)
            .unwrap_or(&empty)
            .iter()
            .chain(uses_map.get(&consumer.id).unwrap_or(&empty).iter())
            .collect();

        for target_id in &edges {
            let Some(target) = by_id.get(target_id) else {
                continue;
            };
            // Skip same-package edges — DIP is about cross-layer dependencies.
            if package_of(&target.file_path, corpus_root) == consumer_pkg {
                continue;
            }
            // Skip the abstraction edges themselves.
            if params.interface_kinds.contains(&target.kind) {
                continue;
            }
            // Only fire when the concrete target implements something the
            // consumer could be using instead.
            let Some(trait_ids) = implements_of.get(target_id) else {
                continue;
            };
            let suggestion = trait_ids
                .iter()
                .find(|tid| !edges.contains(tid))
                .and_then(|tid| by_id.get(tid));
            let Some(suggested) = suggestion else {
                continue;
            };
            findings.push(SolidFinding::ConcreteDependency {
                principle: SolidPrinciple::Dip,
                consumer: to_ref(consumer),
                concrete_target: to_ref(target),
                suggested_abstraction: Some(to_ref(suggested)),
            });
        }
    }
    findings
}

/// Detect Fowler's Shotgun Surgery: the same `(name, kind)` symbol appears
/// across multiple files with mostly disjoint callee sets — a fan-out
/// dispatch family that probably wants a single abstraction.
///
/// Kept distinct from `detect_redundancy` (Type-4 clone) which requires
/// *high* Jaccard. Shotgun Surgery is the inverse: similar surface,
/// deliberately disjoint internals.
#[allow(clippy::too_many_lines)]
fn detect_shotgun_surgery(
    candidates: &[&SymbolRecord],
    callees_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    uses_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    corpus_root: &str,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    // Group candidates by (name, kind), restricted to kinds that can carry
    // dispatch — function and method. Structs/traits sharing a name is a
    // namespacing pattern, not a smell.
    let mut groups: HashMap<(String, String), Vec<&SymbolRecord>> = HashMap::new();
    for sym in candidates {
        if sym.kind != "function" && sym.kind != "method" {
            continue;
        }
        let key = (sym.name.clone(), sym.kind.clone());
        groups.entry(key).or_default().push(sym);
    }

    let mut findings: Vec<SolidFinding> = Vec::new();
    for ((name, kind), members) in groups {
        // Skip Rust-conventional / universal idiomatic method names. These
        // appear all over a typical codebase as trait conformance (Display,
        // Default, From, ...) or universal entry points; treating them as
        // shotgun surgery is pure noise.
        if params.shotgun_skip_conventional_names && is_conventional_method_name(&name) {
            continue;
        }
        // One symbol per file — collapse multi-impl-block duplication.
        let mut by_file: HashMap<String, &SymbolRecord> = HashMap::new();
        for sym in members {
            by_file
                .entry(sym.file_path.clone())
                .and_modify(|prev| {
                    if sym.line_start < prev.line_start {
                        *prev = sym;
                    }
                })
                .or_insert(sym);
        }
        if by_file.len() < params.shotgun_min_sites {
            continue;
        }
        // Sites must span ≥ shotgun_min_packages distinct packages
        // (workspace-relative: first path segment after the corpus root).
        // Single-crate fan-out is typically intentional polymorphism
        // (e.g. per-language trait impls all in `ministr-core/src/code/lang/`).
        let packages: HashSet<String> = by_file
            .values()
            .map(|s| package_of(&s.file_path, corpus_root))
            .filter(|p| !p.is_empty())
            .collect();
        if packages.len() < params.shotgun_min_packages {
            continue;
        }
        let sites: Vec<&SymbolRecord> = by_file.into_values().collect();

        // Average pairwise Jaccard over callees ∪ uses. Low = disjoint
        // internals = stronger Shotgun Surgery signal.
        let empty: HashSet<SymbolId> = HashSet::new();
        let callee_sets: Vec<HashSet<&SymbolId>> = sites
            .iter()
            .map(|s| {
                callees_map
                    .get(&s.id)
                    .unwrap_or(&empty)
                    .iter()
                    .chain(uses_map.get(&s.id).unwrap_or(&empty).iter())
                    .collect()
            })
            .collect();
        let mut sum_jac = 0.0f32;
        let mut pair_count: u32 = 0;
        for i in 0..sites.len() {
            for j in (i + 1)..sites.len() {
                let a = &callee_sets[i];
                let b = &callee_sets[j];
                let inter = a.intersection(b).count();
                let union = a.union(b).count();
                #[allow(clippy::cast_precision_loss)]
                let jac = if union == 0 {
                    0.0
                } else {
                    inter as f32 / union as f32
                };
                sum_jac += jac;
                pair_count += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let avg_jaccard = if pair_count == 0 {
            0.0
        } else {
            sum_jac / pair_count as f32
        };
        // High Jaccard = these are real Type-4 clones already; leave them
        // to `detect_redundancy` and stay quiet here.
        if avg_jaccard > params.shotgun_max_jaccard {
            continue;
        }

        // Stable ordering: by file path, then line.
        let mut ordered: Vec<&SymbolRecord> = sites;
        ordered.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then_with(|| a.line_start.cmp(&b.line_start))
        });
        let total = ordered.len();
        let refs: Vec<SolidSymbolRef> = ordered.iter().map(|s| to_ref(s)).collect();
        let (kept, omitted) = truncate_with_count(refs, params.representative_count);
        findings.push(SolidFinding::ShotgunSurgery {
            principle: SolidPrinciple::ShotgunSurgery,
            name,
            kind,
            sites: kept,
            sites_omitted: omitted,
            sites_total: total,
            avg_jaccard,
        });
    }
    // Sort by total site count (most-fan-out first) for stable output.
    findings.sort_by(|a, b| {
        let sa = match a {
            SolidFinding::ShotgunSurgery { sites_total, .. } => *sites_total,
            _ => 0,
        };
        let sb = match b {
            SolidFinding::ShotgunSurgery { sites_total, .. } => *sites_total,
            _ => 0,
        };
        sb.cmp(&sa)
    });
    findings
}

/// Detect architectural cyclic dependencies by running Tarjan's
/// strongly-connected-components algorithm over the package-level import
/// graph. Packages are derived from `package_prefix(file_path)` — the
/// first two path segments — which is the cheapest way to approximate
/// crate / workspace package identity across languages without requiring
/// a pre-built `PackageGraph`.
#[allow(clippy::too_many_lines)]
fn detect_cyclic_dependency(
    all_symbols: &[SymbolRecord],
    by_id: &HashMap<SymbolId, SymbolRecord>,
    _callees_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    uses_map: &HashMap<SymbolId, HashSet<SymbolId>>,
    corpus_root: &str,
    params: &SolidParams,
) -> Vec<SolidFinding> {
    // Index every symbol's package once (workspace-relative).
    let pkg_of: HashMap<&SymbolId, String> = all_symbols
        .iter()
        .map(|s| (&s.id, package_of(&s.file_path, corpus_root)))
        .collect();

    // Build an index of "which (name, kind) symbols does each package
    // own?" so we can detect ambiguous cross-package edges where the
    // source package already has a same-named twin. Such edges are
    // almost always indexer name-resolution noise rather than real
    // cross-crate dependencies — the source code could resolve the
    // reference locally, so the cross-crate binding doesn't reflect
    // intent.
    let mut symbol_in_pkg: HashSet<(String, String, String)> = HashSet::new();
    for sym in all_symbols {
        if let Some(pkg) = pkg_of.get(&sym.id)
            && !pkg.is_empty()
        {
            symbol_in_pkg.insert((pkg.clone(), sym.name.clone(), sym.kind.clone()));
        }
    }

    // Build the directed package graph from cross-package symbol edges.
    // Track every distinct edge per (from_pkg, to_pkg) so we can apply
    // the "≥ N edges per direction" threshold below.
    //
    // Deliberately ignore `Calls` edges here: method-call refs are
    // resolved by name in the indexer and can produce phantom cross-crate
    // edges when the target name (`status`, `new`, ...) is ambiguous.
    // `Uses` + `Imports` edges (type-position references and `use`
    // declarations) require an actual symbol in scope, so they reflect
    // real dependencies between packages.
    let empty: HashSet<SymbolId> = HashSet::new();
    let mut edge_counts: HashMap<(String, String), usize> = HashMap::new();
    let mut examples: HashMap<(String, String), (&SymbolRecord, &SymbolRecord)> = HashMap::new();
    for sym in all_symbols {
        // Sample / fixture / test paths are not part of the workspace
        // dependency graph — their job is to be sample data, not real
        // production code. Skip edges whose source lives in one.
        if params.cyclic_skip_test_paths && is_test_path(&sym.file_path) {
            continue;
        }
        // Only function / method sources can produce the kind of edge
        // that drives a real architectural cycle. Type declarations
        // (enum / struct / trait) only legitimately reference their
        // field / variant / supertrait types, which the compiler
        // already enforces to be acyclic across packages. In practice,
        // when an indexer attributes a cross-package `Uses` edge to an
        // enum at line N, it's almost always misattribution of a
        // function-body reference from a nearby symbol in the same
        // file.
        if sym.kind != "function" && sym.kind != "method" {
            continue;
        }
        let from_pkg = match pkg_of.get(&sym.id) {
            Some(p) => p.clone(),
            None => continue,
        };
        if from_pkg.is_empty() {
            continue;
        }
        let uses = uses_map.get(&sym.id).unwrap_or(&empty);
        for target_id in uses {
            let Some(target_sym) = by_id.get(target_id) else {
                continue;
            };
            if params.cyclic_skip_test_paths && is_test_path(&target_sym.file_path) {
                continue;
            }
            let to_pkg = match pkg_of.get(&target_sym.id) {
                Some(p) => p.clone(),
                None => continue,
            };
            if to_pkg.is_empty() || to_pkg == from_pkg {
                continue;
            }
            // Skip the edge if the source package owns its own symbol
            // with the same (name, kind). The reference could have
            // resolved locally, so the cross-package binding is almost
            // certainly an indexer name-resolution artefact — common
            // with deliberately-mirrored wire types (`SolidSymbolRef`
            // exists in both `ministr-api` and `ministr-core`, etc.).
            let twin_key = (
                from_pkg.clone(),
                target_sym.name.clone(),
                target_sym.kind.clone(),
            );
            if symbol_in_pkg.contains(&twin_key) {
                continue;
            }
            *edge_counts
                .entry((from_pkg.clone(), to_pkg.clone()))
                .or_default() += 1;
            examples
                .entry((from_pkg.clone(), to_pkg.clone()))
                .or_insert((sym, target_sym));
        }
    }

    // Apply the "≥ N edges per direction" threshold. A single edge
    // between two packages is usually a phantom from ambiguous symbol-
    // name resolution; real coupling shows up as multiple distinct
    // touch points.
    let mut adjacency: HashMap<String, HashSet<String>> = HashMap::new();
    for ((from, to), count) in &edge_counts {
        if *count >= params.cyclic_min_edges_per_direction {
            adjacency
                .entry(from.clone())
                .or_default()
                .insert(to.clone());
        }
    }

    let sccs = tarjan_scc(&adjacency);
    let mut findings: Vec<SolidFinding> = Vec::new();
    for scc in sccs {
        if scc.len() < 2 {
            continue;
        }
        // Collect every cross-package edge that lives inside the SCC.
        let scc_set: HashSet<&String> = scc.iter().collect();
        let mut inner_edges: Vec<SolidEdge> = Vec::new();
        for (from, to) in examples.keys() {
            if scc_set.contains(from) && scc_set.contains(to) {
                let pair = examples.get(&(from.clone(), to.clone())).expect("present");
                inner_edges.push(SolidEdge {
                    from: from.clone(),
                    to: to.clone(),
                    example_from: to_ref(pair.0),
                    example_to: to_ref(pair.1),
                });
            }
        }
        if inner_edges.is_empty() {
            continue;
        }
        // Stable: lexicographic on (from, to).
        inner_edges.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.to.cmp(&b.to)));
        let edge_count = inner_edges.len();
        let (kept_edges, omitted_edges) =
            truncate_with_count(inner_edges, params.representative_count);
        let mut packages = scc;
        packages.sort();
        findings.push(SolidFinding::CyclicDependency {
            principle: SolidPrinciple::CyclicDependency,
            packages,
            edge_count,
            example_edges: kept_edges,
            example_edges_omitted: omitted_edges,
        });
    }
    findings
}

/// Tarjan's strongly-connected-components algorithm over a `HashMap`
/// adjacency list. Returns one `Vec<String>` per SCC in reverse
/// post-order; trivial (size-1, no self-loop) SCCs are included so the
/// caller can filter as needed.
fn tarjan_scc(adj: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
    let nodes: Vec<&String> = adj.keys().collect();
    let node_idx: HashMap<&String, usize> =
        nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let n = nodes.len();
    let mut indices = vec![usize::MAX; n];
    let mut lowlinks = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut sccs: Vec<Vec<String>> = Vec::new();

    // Iterative DFS to dodge stack-overflow on huge graphs.
    for v in 0..n {
        if indices[v] != usize::MAX {
            continue;
        }
        let mut work_stack: Vec<(usize, std::vec::IntoIter<usize>)> = Vec::new();
        indices[v] = next_index;
        lowlinks[v] = next_index;
        next_index += 1;
        stack.push(v);
        on_stack[v] = true;
        let v_succs = collect_successors(nodes[v], adj, &node_idx);
        work_stack.push((v, v_succs.into_iter()));

        while let Some(&mut (parent, ref mut succ_iter)) = work_stack.last_mut() {
            if let Some(child) = succ_iter.next() {
                if indices[child] == usize::MAX {
                    indices[child] = next_index;
                    lowlinks[child] = next_index;
                    next_index += 1;
                    stack.push(child);
                    on_stack[child] = true;
                    let child_succs = collect_successors(nodes[child], adj, &node_idx);
                    work_stack.push((child, child_succs.into_iter()));
                } else if on_stack[child] {
                    lowlinks[parent] = lowlinks[parent].min(indices[child]);
                }
            } else {
                if lowlinks[parent] == indices[parent] {
                    let mut scc: Vec<String> = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        scc.push(nodes[w].clone());
                        if w == parent {
                            break;
                        }
                    }
                    sccs.push(scc);
                }
                work_stack.pop();
                if let Some(&mut (caller, _)) = work_stack.last_mut() {
                    lowlinks[caller] = lowlinks[caller].min(lowlinks[parent]);
                }
            }
        }
    }
    sccs
}

fn collect_successors(
    node: &String,
    adj: &HashMap<String, HashSet<String>>,
    node_idx: &HashMap<&String, usize>,
) -> Vec<usize> {
    adj.get(node)
        .into_iter()
        .flat_map(|set| set.iter())
        .filter_map(|n| node_idx.get(n).copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::embedding::Embedder;
    use crate::error::IndexError;
    use crate::index::HnswIndex;
    use crate::storage::{SqliteStorage, SymbolRecord, SymbolRefRecord};
    use crate::types::SymbolId;

    /// Deterministic byte-shingle embedder so test vectors are reproducible
    /// without loading a real model.
    struct ShingleEmbedder {
        dim: usize,
    }

    impl Embedder for ShingleEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b);
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

    /// Build a service over an in-memory SQLite + tiny HNSW for unit testing.
    fn make_service() -> (QueryService, Arc<ShingleEmbedder>) {
        let dim = 8;
        let embedder = Arc::new(ShingleEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();
        (
            QueryService::new(storage, embedder.clone(), index),
            embedder,
        )
    }

    /// Insert a symbol and its stub embedding in one shot.
    #[allow(clippy::too_many_arguments)]
    async fn put_symbol(
        storage: &SqliteStorage,
        embedder: &ShingleEmbedder,
        id: &str,
        name: &str,
        kind: &str,
        module_path: &str,
        file: &str,
        signature: &str,
        line_start: u32,
        line_end: u32,
    ) {
        let sym = SymbolRecord {
            id: SymbolId(id.into()),
            file_path: file.into(),
            name: name.into(),
            kind: kind.into(),
            visibility: "pub".into(),
            signature: signature.into(),
            doc_comment: None,
            module_path: module_path.into(),
            line_start,
            line_end,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[sym]).await.unwrap();
        let text = format!("{name} {signature}");
        let vec = embedder.embed(&[text.as_str()]).unwrap().pop().unwrap();
        let vid = VectorId::symbol_stub(id).as_str().to_string();
        storage.store_full_dim_vectors(&[(vid, vec)]).await.unwrap();
    }

    async fn put_ref(storage: &SqliteStorage, from: &str, to: &str, kind: RefKind) {
        storage
            .insert_symbol_refs(&[SymbolRefRecord {
                from_symbol_id: SymbolId(from.into()),
                to_symbol_id: SymbolId(to.into()),
                ref_kind: kind,
            }])
            .await
            .unwrap();
    }

    /// Regression: on real corpora that use absolute paths
    /// (`/Users/.../workspace/<crate>/src/...`), the corpus-root prefix
    /// must strip the leading workspace path so `package_of` returns the
    /// crate name. Without this, every file collapsed to one "package"
    /// and the cross-package filters silently killed every finding.
    #[test]
    fn corpus_root_prefix_strips_absolute_workspace_path() {
        let records = vec![
            SymbolRecord {
                id: SymbolId("a".into()),
                file_path: "/Users/x/workspace/crate-a/src/lib.rs".into(),
                name: "_".into(),
                kind: "function".into(),
                visibility: "pub".into(),
                signature: String::new(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
            SymbolRecord {
                id: SymbolId("b".into()),
                file_path: "/Users/x/workspace/crate-b/src/lib.rs".into(),
                name: "_".into(),
                kind: "function".into(),
                visibility: "pub".into(),
                signature: String::new(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
        ];
        let root = corpus_root_prefix(&records);
        assert_eq!(root, "/Users/x/workspace/");
        assert_eq!(
            package_of("/Users/x/workspace/crate-a/src/lib.rs", &root),
            "crate-a"
        );
        assert_eq!(
            package_of("/Users/x/workspace/crate-b/src/lib.rs", &root),
            "crate-b"
        );
    }

    /// Single-file corpus must not panic; the corpus root degenerates to
    /// the file's directory and the package is the file's basename.
    #[test]
    fn corpus_root_prefix_single_file() {
        let records = vec![SymbolRecord {
            id: SymbolId("a".into()),
            file_path: "src/only.rs".into(),
            name: "_".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: String::new(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 1,
            cyclomatic_complexity: None,
        }];
        let root = corpus_root_prefix(&records);
        // Whole path is the (single) common segment chain.
        assert_eq!(root, "src/only.rs/");
        // The root is longer than the path, so `strip_prefix` falls back
        // to the raw path and `package_of` returns its first segment.
        // This degenerate case never produces a meaningful cycle/shotgun
        // finding since one file can't span multiple packages — the
        // package value just exists for consistency.
        assert_eq!(package_of("src/only.rs", &root), "src");
    }

    #[tokio::test]
    async fn arity_bucket_counts_top_level_commas() {
        assert_eq!(arity_bucket("fn foo()"), 0);
        assert_eq!(arity_bucket("fn foo(x: i32)"), 1);
        assert_eq!(arity_bucket("fn foo(x: i32, y: i32)"), 2);
        assert_eq!(arity_bucket("fn foo(x: Vec<(i32, i32)>, y: i32)"), 2);
        assert_eq!(
            arity_bucket("fn foo(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32, g: i32)"),
            5,
            "bucket clamps at 5+"
        );
    }

    #[tokio::test]
    async fn redundancy_clusters_near_duplicate_functions() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Two near-duplicate handlers in different files calling the same
        // helper.
        put_symbol(
            storage,
            &embedder,
            "sym-a",
            "handle_a",
            "function",
            "mod_a",
            "src/a.rs",
            "fn handle_a(req: Request) -> Response",
            1,
            20,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b",
            "handle_a",
            "function",
            "mod_b",
            "src/b.rs",
            "fn handle_a(req: Request) -> Response",
            1,
            18,
        )
        .await;
        // A control symbol that is structurally and semantically different.
        put_symbol(
            storage,
            &embedder,
            "sym-c",
            "totally_unrelated_thing",
            "function",
            "mod_c",
            "src/c.rs",
            "fn totally_unrelated_thing(z: ZZZZZ) -> QQQQQ",
            1,
            6,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-helper",
            "helper",
            "function",
            "mod_h",
            "src/h.rs",
            "fn helper()",
            1,
            5,
        )
        .await;

        // Both duplicates call the same helper — Jaccard = 1.
        put_ref(storage, "sym-a", "sym-helper", RefKind::Calls).await;
        put_ref(storage, "sym-b", "sym-helper", RefKind::Calls).await;
        // Control has a disjoint callee set so Jaccard with the duplicates is 0.
        put_symbol(
            storage,
            &embedder,
            "sym-other-helper",
            "other_helper",
            "function",
            "mod_oh",
            "src/oh.rs",
            "fn other_helper()",
            1,
            5,
        )
        .await;
        put_ref(storage, "sym-c", "sym-other-helper", RefKind::Calls).await;

        // Lower thresholds because our toy embedder is coarse.
        let params = SolidParams {
            principles: vec![SolidPrinciple::DryOcp],
            similarity_threshold: 0.5,
            jaccard_threshold: 0.5,
            ..SolidParams::default()
        };

        let findings = service.detect_solid_violations(&params).await.unwrap();
        let cluster = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::Redundancy {
                    members,
                    cross_module,
                    ..
                } => Some((members, *cross_module)),
                _ => None,
            })
            .expect("expected at least one redundancy cluster");
        let ids: HashSet<&str> = cluster.0.iter().map(|m| m.symbol_id.as_str()).collect();
        assert!(ids.contains("sym-a"), "cluster should include sym-a");
        assert!(ids.contains("sym-b"), "cluster should include sym-b");
        assert!(
            !ids.contains("sym-c"),
            "control sym-c must not join the cluster"
        );
        assert!(cluster.1, "cluster spans 2 files → cross_module=true");
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn srp_detects_low_cohesion_container() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Container with two clearly separate responsibilities.
        put_symbol(
            storage,
            &embedder,
            "sym-impl",
            "Service",
            "impl",
            "svc",
            "src/svc.rs",
            "impl Service",
            1,
            100,
        )
        .await;
        // Auth methods (share auth helpers).
        put_symbol(
            storage,
            &embedder,
            "sym-login",
            "login",
            "function",
            "svc::Service",
            "src/svc.rs",
            "fn login(u: User)",
            10,
            20,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-logout",
            "logout",
            "function",
            "svc::Service",
            "src/svc.rs",
            "fn logout(u: User)",
            22,
            30,
        )
        .await;
        // Billing methods (share billing helpers).
        put_symbol(
            storage,
            &embedder,
            "sym-charge",
            "charge",
            "function",
            "svc::Service",
            "src/svc.rs",
            "fn charge(c: Customer)",
            32,
            45,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-refund",
            "refund",
            "function",
            "svc::Service",
            "src/svc.rs",
            "fn refund(c: Customer)",
            47,
            60,
        )
        .await;
        // Helpers.
        put_symbol(
            storage,
            &embedder,
            "sym-auth-h",
            "auth_helper",
            "function",
            "auth",
            "src/auth.rs",
            "fn auth_helper()",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-bill-h",
            "billing_helper",
            "function",
            "billing",
            "src/billing.rs",
            "fn billing_helper()",
            1,
            5,
        )
        .await;
        // Two completely disjoint callee sets.
        put_ref(storage, "sym-login", "sym-auth-h", RefKind::Calls).await;
        put_ref(storage, "sym-logout", "sym-auth-h", RefKind::Calls).await;
        put_ref(storage, "sym-charge", "sym-bill-h", RefKind::Calls).await;
        put_ref(storage, "sym-refund", "sym-bill-h", RefKind::Calls).await;

        // Boost the cosine threshold so only the callee-overlap edge fires.
        let params = SolidParams {
            principles: vec![SolidPrinciple::Srp],
            srp_cohesion_threshold: 0.999,
            ..SolidParams::default()
        };

        let findings = service.detect_solid_violations(&params).await.unwrap();
        let (container, components) = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::LowCohesion {
                    container,
                    components,
                    ..
                } => Some((container, components)),
                _ => None,
            })
            .expect("expected SRP low-cohesion finding");
        assert_eq!(container.symbol_id, "sym-impl");
        assert_eq!(
            components.len(),
            2,
            "expected exactly two cohesion components"
        );
        let sizes: Vec<usize> = components.iter().map(|c| c.size).collect();
        assert_eq!(sizes, vec![2, 2]);
        for c in components {
            assert_eq!(c.size, c.members.len() + c.members_omitted);
        }
    }

    /// A container with one tight cluster + several leaf singletons is a
    /// "god object" / large-class shape, not an SRP-style "two responsibilities
    /// to split." The detector must stay quiet — leaving that smell to other
    /// tools (`ministr_impact`, code review).
    #[tokio::test]
    async fn srp_quiet_when_only_one_cluster_plus_singletons() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        put_symbol(
            storage,
            &embedder,
            "sym-impl",
            "Big",
            "impl",
            "big",
            "src/big.rs",
            "impl Big",
            1,
            200,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-shared",
            "shared_helper",
            "function",
            "h",
            "src/h.rs",
            "fn shared_helper()",
            1,
            5,
        )
        .await;
        // Five methods that all share one helper — one big cluster.
        for name in &["a", "b", "c", "d", "e"] {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-{name}"),
                name,
                "function",
                "big::Big",
                "src/big.rs",
                &format!("fn {name}()"),
                10,
                15,
            )
            .await;
            put_ref(
                storage,
                &format!("sym-{name}"),
                "sym-shared",
                RefKind::Calls,
            )
            .await;
        }
        // Plus two unrelated singletons.
        for name in &["x", "y"] {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-{name}"),
                name,
                "function",
                "big::Big",
                "src/big.rs",
                &format!("fn {name}()"),
                20,
                25,
            )
            .await;
        }

        let params = SolidParams {
            principles: vec![SolidPrinciple::Srp],
            // > 1.0 disables the cosine-fallback edge entirely so the test
            // depends only on shared-callee structure.
            srp_cohesion_threshold: 2.0,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::LowCohesion { .. })),
            "one-cluster-plus-singletons must not fire SRP, got: {findings:?}"
        );
    }

    /// A container whose methods all have disjoint callees (every "component"
    /// is a singleton) is not an SRP violation — there's no cluster to
    /// extract. The detector must stay quiet on this shape.
    #[tokio::test]
    async fn srp_quiet_when_every_component_is_singleton() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        put_symbol(
            storage,
            &embedder,
            "sym-impl",
            "Loose",
            "impl",
            "loose",
            "src/loose.rs",
            "impl Loose",
            1,
            100,
        )
        .await;
        // Five methods, each calling a *different* helper — no shared callees.
        for (idx, name) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            let idx_u32 = u32::try_from(idx).unwrap();
            put_symbol(
                storage,
                &embedder,
                &format!("sym-loose-{name}"),
                name,
                "function",
                "loose::Loose",
                "src/loose.rs",
                &format!("fn {name}()"),
                10 + idx_u32 * 2,
                11 + idx_u32 * 2,
            )
            .await;
            put_symbol(
                storage,
                &embedder,
                &format!("sym-h-{name}"),
                &format!("helper_{name}"),
                "function",
                "helpers",
                "src/helpers.rs",
                &format!("fn helper_{name}()"),
                10 + idx_u32 * 2,
                11 + idx_u32 * 2,
            )
            .await;
            put_ref(
                storage,
                &format!("sym-loose-{name}"),
                &format!("sym-h-{name}"),
                RefKind::Calls,
            )
            .await;
        }

        let params = SolidParams {
            principles: vec![SolidPrinciple::Srp],
            // Force callee-overlap-only edges so the singleton structure survives.
            srp_cohesion_threshold: 0.999,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::LowCohesion { .. })),
            "should not emit SRP findings when every component is a singleton, got: {findings:?}"
        );
    }

    /// When the same Rust type has multiple `impl` blocks in one file, all of
    /// them resolve to the same method set. The detector must collapse them
    /// into a single finding, not emit one per `impl`.
    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn srp_dedupes_containers_that_share_method_set() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Two impl blocks for the same type, in the same file.
        put_symbol(
            storage,
            &embedder,
            "sym-impl-a",
            "Svc",
            "impl",
            "svc",
            "src/svc.rs",
            "impl Svc",
            1,
            50,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-impl-b",
            "Svc",
            "impl",
            "svc",
            "src/svc.rs",
            "impl Svc",
            120,
            200,
        )
        .await;
        // Auth methods (cluster on auth helper).
        for name in &["login", "logout"] {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-{name}"),
                name,
                "function",
                "svc::Svc",
                "src/svc.rs",
                &format!("fn {name}(u: User)"),
                10,
                20,
            )
            .await;
        }
        // Billing methods (cluster on billing helper).
        for name in &["charge", "refund"] {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-{name}"),
                name,
                "function",
                "svc::Svc",
                "src/svc.rs",
                &format!("fn {name}(c: Customer)"),
                30,
                40,
            )
            .await;
        }
        put_symbol(
            storage,
            &embedder,
            "sym-auth-h",
            "auth_helper",
            "function",
            "auth",
            "src/auth.rs",
            "fn auth_helper()",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-bill-h",
            "billing_helper",
            "function",
            "billing",
            "src/billing.rs",
            "fn billing_helper()",
            1,
            5,
        )
        .await;
        put_ref(storage, "sym-login", "sym-auth-h", RefKind::Calls).await;
        put_ref(storage, "sym-logout", "sym-auth-h", RefKind::Calls).await;
        put_ref(storage, "sym-charge", "sym-bill-h", RefKind::Calls).await;
        put_ref(storage, "sym-refund", "sym-bill-h", RefKind::Calls).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::Srp],
            srp_cohesion_threshold: 0.999,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        let srp: Vec<_> = findings
            .iter()
            .filter(|f| matches!(f, SolidFinding::LowCohesion { .. }))
            .collect();
        assert_eq!(
            srp.len(),
            1,
            "two impl blocks for the same type must produce one finding, got {srp:?}"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn isp_flags_fat_interface_with_under_using_implementor() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // A 6-method trait.
        put_symbol(
            storage,
            &embedder,
            "sym-trait",
            "Backend",
            "trait",
            "be",
            "src/be.rs",
            "trait Backend",
            1,
            40,
        )
        .await;
        for (idx, name) in [
            "method_a", "method_b", "method_c", "method_d", "method_e", "method_f",
        ]
        .iter()
        .enumerate()
        {
            let idx_u32 = u32::try_from(idx).unwrap();
            put_symbol(
                storage,
                &embedder,
                &format!("sym-tm-{idx}"),
                name,
                "function",
                "be::Backend",
                "src/be.rs",
                &format!("fn {name}()"),
                10 + idx_u32,
                11 + idx_u32,
            )
            .await;
        }
        // Under-using implementor: only one matching method.
        put_symbol(
            storage,
            &embedder,
            "sym-impl-lite",
            "LiteImpl",
            "impl",
            "be",
            "src/lite.rs",
            "impl Backend for LiteImpl",
            1,
            20,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-lite-m1",
            "method_a",
            "function",
            "be::LiteImpl",
            "src/lite.rs",
            "fn method_a()",
            5,
            6,
        )
        .await;
        // Heavy implementor: nope, also under-using (only 1) — drives the 50% rule.
        put_symbol(
            storage,
            &embedder,
            "sym-impl-heavy",
            "HeavyImpl",
            "impl",
            "be",
            "src/heavy.rs",
            "impl Backend for HeavyImpl",
            1,
            20,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-heavy-m1",
            "method_b",
            "function",
            "be::HeavyImpl",
            "src/heavy.rs",
            "fn method_b()",
            5,
            6,
        )
        .await;

        put_ref(storage, "sym-impl-lite", "sym-trait", RefKind::Implements).await;
        put_ref(storage, "sym-impl-heavy", "sym-trait", RefKind::Implements).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::Isp],
            ..SolidParams::default()
        };

        let findings = service.detect_solid_violations(&params).await.unwrap();
        let (iface, count, unused) = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::FatInterface {
                    interface,
                    method_count,
                    unused_methods,
                    ..
                } => Some((interface, *method_count, unused_methods)),
                _ => None,
            })
            .expect("expected ISP finding");
        assert_eq!(iface.symbol_id, "sym-trait");
        assert_eq!(count, 6);
        // Of 6 methods, only method_a and method_b are covered → 4 unused.
        assert_eq!(unused.len(), 4);
    }

    #[tokio::test]
    async fn dip_flags_concrete_cross_package_call_with_available_trait() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Consumer (high-level) — in package src/high.
        put_symbol(
            storage,
            &embedder,
            "sym-consumer",
            "do_work",
            "function",
            "high",
            "src/high/do.rs",
            "fn do_work()",
            1,
            30,
        )
        .await;
        // Trait (abstraction) — could be in either package; put it in low.
        put_symbol(
            storage,
            &embedder,
            "sym-trait",
            "Store",
            "trait",
            "low",
            "src/low/store.rs",
            "trait Store",
            1,
            10,
        )
        .await;
        // Concrete struct in a *different* package that implements Store.
        put_symbol(
            storage,
            &embedder,
            "sym-concrete",
            "DiskStore",
            "struct",
            "low",
            "src/low/disk.rs",
            "struct DiskStore",
            1,
            20,
        )
        .await;
        put_ref(storage, "sym-concrete", "sym-trait", RefKind::Implements).await;
        // Consumer uses the concrete, not the trait.
        put_ref(storage, "sym-consumer", "sym-concrete", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::Dip],
            ..SolidParams::default()
        };

        let findings = service.detect_solid_violations(&params).await.unwrap();
        let (consumer, target, suggestion) = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::ConcreteDependency {
                    consumer,
                    concrete_target,
                    suggested_abstraction,
                    ..
                } => Some((consumer, concrete_target, suggested_abstraction)),
                _ => None,
            })
            .expect("expected DIP finding");
        assert_eq!(consumer.symbol_id, "sym-consumer");
        assert_eq!(target.symbol_id, "sym-concrete");
        assert_eq!(
            suggestion.as_ref().map(|s| s.symbol_id.as_str()),
            Some("sym-trait")
        );
    }

    #[tokio::test]
    async fn dip_quiet_when_consumer_already_uses_trait() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        put_symbol(
            storage,
            &embedder,
            "sym-consumer",
            "do_work",
            "function",
            "high",
            "src/high/do.rs",
            "fn do_work()",
            1,
            30,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-trait",
            "Store",
            "trait",
            "low",
            "src/low/store.rs",
            "trait Store",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-concrete",
            "DiskStore",
            "struct",
            "low",
            "src/low/disk.rs",
            "struct DiskStore",
            1,
            20,
        )
        .await;
        put_ref(storage, "sym-concrete", "sym-trait", RefKind::Implements).await;
        put_ref(storage, "sym-consumer", "sym-concrete", RefKind::Uses).await;
        // Already abstracted via the trait → no DIP smell.
        put_ref(storage, "sym-consumer", "sym-trait", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::Dip],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::ConcreteDependency { .. })),
            "should not flag a consumer that already uses the trait"
        );
    }

    #[tokio::test]
    async fn empty_corpus_returns_no_findings() {
        let (service, _) = make_service();
        let findings = service
            .detect_solid_violations(&SolidParams::default())
            .await
            .unwrap();
        assert!(findings.is_empty());
    }

    /// The dispatch family pattern: same `(name, kind)` across 3+ files
    /// where each implementation calls into a *different* helper. This is
    /// exactly the case Type-4 clone detection (avg high Jaccard) rejects
    /// and ShotgunSurgery should catch.
    #[tokio::test]
    async fn shotgun_surgery_catches_parallel_dispatch_family() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Three "do_thing" functions in three different files, each
        // delegating to a different helper.
        for (i, file) in ["src/a.rs", "src/b.rs", "src/c.rs"].iter().enumerate() {
            let idx = u32::try_from(i).unwrap();
            put_symbol(
                storage,
                &embedder,
                &format!("sym-do-{i}"),
                "do_thing",
                "function",
                &format!("mod_{i}"),
                file,
                "fn do_thing()",
                10,
                25,
            )
            .await;
            put_symbol(
                storage,
                &embedder,
                &format!("sym-helper-{i}"),
                &format!("helper_{i}"),
                "function",
                "h",
                &format!("src/h{idx}.rs"),
                &format!("fn helper_{i}()"),
                1,
                5,
            )
            .await;
            put_ref(
                storage,
                &format!("sym-do-{i}"),
                &format!("sym-helper-{i}"),
                RefKind::Calls,
            )
            .await;
        }

        let params = SolidParams {
            principles: vec![SolidPrinciple::ShotgunSurgery],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        let (name, sites_total, avg_jaccard) = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::ShotgunSurgery {
                    name,
                    sites_total,
                    avg_jaccard,
                    ..
                } => Some((name, *sites_total, *avg_jaccard)),
                _ => None,
            })
            .expect("expected ShotgunSurgery finding");
        assert_eq!(name, "do_thing");
        assert_eq!(sites_total, 3);
        assert!(
            avg_jaccard < 0.01,
            "callee sets are fully disjoint → expected avg_jaccard ≈ 0, got {avg_jaccard}"
        );
    }

    /// Two packages that import each other form a 2-cycle.
    #[tokio::test]
    async fn cyclic_dependency_two_package_cycle() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        // Package A (src/a/...) and package B (src/b/...) with mutual
        // Uses edges across the boundary.
        put_symbol(
            storage,
            &embedder,
            "sym-a-mod",
            "thing",
            "function",
            "a",
            "src/a/lib.rs",
            "fn thing()",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b-mod",
            "stuff",
            "function",
            "b",
            "src/b/lib.rs",
            "fn stuff()",
            1,
            10,
        )
        .await;
        // Add a second symbol per crate so we have ≥ 2 distinct Uses
        // edges in each direction — the default threshold requires that
        // many to confirm a real cycle (single edges are usually
        // phantom name-resolution artefacts).
        put_symbol(
            storage,
            &embedder,
            "sym-a-extra",
            "extra_a",
            "function",
            "a",
            "src/a/extra.rs",
            "fn extra_a()",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b-extra",
            "extra_b",
            "function",
            "b",
            "src/b/extra.rs",
            "fn extra_b()",
            1,
            5,
        )
        .await;
        put_ref(storage, "sym-a-mod", "sym-b-mod", RefKind::Uses).await;
        put_ref(storage, "sym-a-extra", "sym-b-extra", RefKind::Uses).await;
        put_ref(storage, "sym-b-mod", "sym-a-mod", RefKind::Uses).await;
        put_ref(storage, "sym-b-extra", "sym-a-extra", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        let (packages, edge_count) = findings
            .iter()
            .find_map(|f| match f {
                SolidFinding::CyclicDependency {
                    packages,
                    edge_count,
                    ..
                } => Some((packages, *edge_count)),
                _ => None,
            })
            .expect("expected CyclicDependency finding");
        assert_eq!(packages.len(), 2);
        // Packages are workspace-relative: corpus root is "src/", so the
        // package of "src/a/lib.rs" is "a", and "src/b/lib.rs" is "b".
        assert!(packages.iter().any(|p| p == "a"));
        assert!(packages.iter().any(|p| p == "b"));
        // `edge_count` counts distinct (from_pkg → to_pkg) directional
        // edges inside the SCC — 2 packages, both directions ⇒ 2.
        assert_eq!(edge_count, 2, "two directional package edges expected");
    }

    /// Single Uses edge per direction is usually a phantom from
    /// ambiguous symbol-name resolution (e.g. two crates that both
    /// define a type called `SolidSymbolRef`). The default threshold of
    /// 2 edges per direction must suppress this shape.
    #[tokio::test]
    async fn cyclic_dependency_suppresses_single_edge_phantom_cycle() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        put_symbol(
            storage,
            &embedder,
            "sym-a",
            "a_thing",
            "function",
            "a",
            "src/a/lib.rs",
            "fn a_thing()",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b",
            "b_thing",
            "function",
            "b",
            "src/b/lib.rs",
            "fn b_thing()",
            1,
            10,
        )
        .await;
        // ONE Uses edge per direction — the kind of pattern phantom
        // name-resolution typically produces.
        put_ref(storage, "sym-a", "sym-b", RefKind::Uses).await;
        put_ref(storage, "sym-b", "sym-a", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "single-edge mutual Uses should not produce a cycle at default threshold, got: {findings:?}"
        );

        // Dialing the threshold down to 1 must re-enable the finding.
        let aggressive = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            cyclic_min_edges_per_direction: 1,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&aggressive).await.unwrap();
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "with threshold=1, single-edge cycle should fire"
        );
    }

    /// Indexers occasionally misattribute a function-body reference to
    /// a nearby type declaration (enum / struct / trait) at the same
    /// file. The cycle detector must only count edges sourced from
    /// function / method symbols so those misattributions can't drive
    /// phantom cycles.
    #[tokio::test]
    async fn cyclic_dependency_ignores_non_function_source_edges() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // crate-a holds an enum and crate-b holds a function; the indexer
        // (in the simulated scenario) has misattributed a Uses edge to
        // the enum.
        put_symbol(
            storage,
            &embedder,
            "sym-a-enum",
            "Result",
            "enum",
            "a",
            "src/a/lib.rs",
            "enum Result",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b-fn",
            "do_thing",
            "function",
            "b",
            "src/b/lib.rs",
            "fn do_thing()",
            1,
            5,
        )
        .await;
        // Phantom: enum "uses" a function in another crate.
        put_ref(storage, "sym-a-enum", "sym-b-fn", RefKind::Uses).await;
        // Legitimate reverse edge from a function.
        put_symbol(
            storage,
            &embedder,
            "sym-b-fn2",
            "another",
            "function",
            "b",
            "src/b/extra.rs",
            "fn another()",
            1,
            5,
        )
        .await;
        put_ref(storage, "sym-b-fn", "sym-a-enum", RefKind::Uses).await;
        put_ref(storage, "sym-b-fn2", "sym-a-enum", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            cyclic_min_edges_per_direction: 1,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "edge sourced from an enum must not contribute to a cycle, got: {findings:?}"
        );
    }

    /// When two packages each own a symbol with the same name + kind
    /// (e.g. wire-type duplication between `ministr-api` and
    /// `ministr-core`), the indexer's name-resolver can produce phantom
    /// cross-package edges. The cycle detector must skip them — the
    /// source could have resolved locally.
    #[tokio::test]
    async fn cyclic_dependency_skips_ambiguous_same_name_twins() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // Both crates own a `Foo` struct.
        put_symbol(
            storage,
            &embedder,
            "sym-a-foo",
            "Foo",
            "struct",
            "a",
            "src/a/foo.rs",
            "struct Foo",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b-foo",
            "Foo",
            "struct",
            "b",
            "src/b/foo.rs",
            "struct Foo",
            1,
            5,
        )
        .await;
        // A consumer in `a` that *should* be using `a::Foo` but the
        // indexer phantom-binds the use to `b::Foo`. Same scenario in
        // reverse for the other direction.
        put_symbol(
            storage,
            &embedder,
            "sym-a-user",
            "use_foo_a",
            "function",
            "a",
            "src/a/use.rs",
            "fn use_foo_a()",
            1,
            5,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b-user",
            "use_foo_b",
            "function",
            "b",
            "src/b/use.rs",
            "fn use_foo_b()",
            1,
            5,
        )
        .await;
        put_ref(storage, "sym-a-user", "sym-b-foo", RefKind::Uses).await;
        put_ref(storage, "sym-b-user", "sym-a-foo", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            // Threshold of 1 so any surviving edge would fire — the
            // same-name-twin filter is what must keep this silent.
            cyclic_min_edges_per_direction: 1,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "ambiguous same-name twin edges should be skipped, got: {findings:?}"
        );
    }

    /// Edges whose source or target file lives in a test/fixture path
    /// are sample data and must be excluded by default.
    #[tokio::test]
    async fn cyclic_dependency_skips_test_fixture_edges() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // Two production-path symbols, two fixture-path symbols.
        put_symbol(
            storage,
            &embedder,
            "sym-prod",
            "prod_thing",
            "function",
            "a",
            "src/a/lib.rs",
            "fn prod_thing()",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-fixture",
            "Serialize",
            "function",
            "b",
            "src/b/tests/fixtures/sample.cs",
            "fn Serialize()",
            1,
            5,
        )
        .await;
        // Plus give crate-a a second internal symbol so the corpus root
        // doesn't collapse the whole prefix.
        put_symbol(
            storage,
            &embedder,
            "sym-other",
            "other",
            "function",
            "a",
            "src/a/other.rs",
            "fn other()",
            1,
            5,
        )
        .await;
        // Two cross-package edges, but the target lives in a fixture
        // path so the cycle filter should drop them.
        put_ref(storage, "sym-prod", "sym-fixture", RefKind::Uses).await;
        put_ref(storage, "sym-other", "sym-fixture", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            cyclic_min_edges_per_direction: 1,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "fixture-targeting edges should be filtered, got: {findings:?}"
        );
    }

    /// Method-call refs (`RefKind::Calls`) are resolved by name in the
    /// indexer and frequently produce phantom cross-package edges when
    /// target names are ambiguous (`new`, `status`, ...). The cycle
    /// detector must ignore them; only `Uses` / `Imports` count.
    #[tokio::test]
    async fn cyclic_dependency_ignores_calls_only_cycles() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // Two crates with `Calls` edges in both directions — no `Uses` /
        // `Imports`. The architecture is acyclic per the package manifest
        // even though method names happen to match across crates.
        put_symbol(
            storage,
            &embedder,
            "sym-a",
            "a_thing",
            "function",
            "a",
            "src/a/lib.rs",
            "fn a_thing()",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-b",
            "b_thing",
            "function",
            "b",
            "src/b/lib.rs",
            "fn b_thing()",
            1,
            10,
        )
        .await;
        put_ref(storage, "sym-a", "sym-b", RefKind::Calls).await;
        put_ref(storage, "sym-b", "sym-a", RefKind::Calls).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "Calls-only mutual refs should not produce a cycle, got: {findings:?}"
        );
    }

    /// One-way cross-package dependencies (a clean layered architecture)
    /// must not produce a cycle finding.
    #[tokio::test]
    async fn cyclic_dependency_silent_on_layered_arch() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        put_symbol(
            storage,
            &embedder,
            "sym-high",
            "consumer",
            "function",
            "high",
            "src/high/lib.rs",
            "fn consumer()",
            1,
            10,
        )
        .await;
        put_symbol(
            storage,
            &embedder,
            "sym-low",
            "primitive",
            "function",
            "low",
            "src/low/lib.rs",
            "fn primitive()",
            1,
            10,
        )
        .await;
        // Only high → low; no reverse edge.
        put_ref(storage, "sym-high", "sym-low", RefKind::Uses).await;

        let params = SolidParams {
            principles: vec![SolidPrinciple::CyclicDependency],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::CyclicDependency { .. })),
            "one-way layered dep must not fire, got: {findings:?}"
        );
    }

    /// Conventional method names (`new`, `default`, `fmt`, ...) must be
    /// silently dropped — they're language idioms, not fan-out smells —
    /// but the filter must respect the opt-out flag.
    #[tokio::test]
    async fn shotgun_surgery_skips_conventional_names_by_default() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // Three independent types each with a `new()` constructor across
        // three packages — would otherwise trip Shotgun Surgery.
        for (i, file) in [
            "ministr-api/src/x.rs",
            "ministr-core/src/y.rs",
            "ministr-daemon/src/z.rs",
        ]
        .iter()
        .enumerate()
        {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-new-{i}"),
                "new",
                "function",
                &format!("m_{i}::T{i}"),
                file,
                "fn new() -> Self",
                10,
                25,
            )
            .await;
        }

        let default_params = SolidParams {
            principles: vec![SolidPrinciple::ShotgunSurgery],
            ..SolidParams::default()
        };
        let findings = service
            .detect_solid_violations(&default_params)
            .await
            .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::ShotgunSurgery { name, .. } if name == "new")),
            "conventional `new` must be filtered by default, got: {findings:?}"
        );

        // Opt-out → finding reappears.
        let opt_out = SolidParams {
            principles: vec![SolidPrinciple::ShotgunSurgery],
            shotgun_skip_conventional_names: false,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&opt_out).await.unwrap();
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SolidFinding::ShotgunSurgery { name, .. } if name == "new")),
            "with skip_conventional_names=false the `new` group should fire"
        );
    }

    /// A 3-site fan-out confined to a single package is usually intentional
    /// per-language polymorphism — the cross-package filter must keep it
    /// quiet, but a 3-site fan-out across multiple packages must fire.
    #[tokio::test]
    async fn shotgun_surgery_requires_cross_package_spread() {
        let (service, embedder) = make_service();
        let storage = service.storage();
        // One file in a *different* crate so the corpus root prefix
        // collapses to nothing and the three `lang/*.rs` files share the
        // same "ministr-core" package after stripping.
        put_symbol(
            storage,
            &embedder,
            "sym-external",
            "external",
            "function",
            "external",
            "ministr-api/src/lib.rs",
            "fn external()",
            1,
            5,
        )
        .await;
        // Same name, 3 sites, all in the same crate (ministr-core).
        for (i, file) in [
            "ministr-core/src/code/lang/a.rs",
            "ministr-core/src/code/lang/b.rs",
            "ministr-core/src/code/lang/c.rs",
        ]
        .iter()
        .enumerate()
        {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-extract-{i}"),
                "extract_lang_thing",
                "function",
                &format!("lang_{i}::T{i}"),
                file,
                "fn extract_lang_thing()",
                10,
                25,
            )
            .await;
        }

        let params = SolidParams {
            principles: vec![SolidPrinciple::ShotgunSurgery],
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::ShotgunSurgery { .. })),
            "single-package fan-out should be silent, got: {findings:?}"
        );
    }

    /// Three same-name functions that all call the *same* helper are a
    /// real Type-4 clone family — ShotgunSurgery must defer to the
    /// redundancy detector and stay quiet.
    #[tokio::test]
    async fn shotgun_surgery_silent_when_callees_align() {
        let (service, embedder) = make_service();
        let storage = service.storage();

        put_symbol(
            storage,
            &embedder,
            "sym-helper",
            "shared",
            "function",
            "h",
            "src/h.rs",
            "fn shared()",
            1,
            5,
        )
        .await;
        for (i, file) in ["src/a.rs", "src/b.rs", "src/c.rs"].iter().enumerate() {
            put_symbol(
                storage,
                &embedder,
                &format!("sym-do-{i}"),
                "do_thing",
                "function",
                &format!("mod_{i}"),
                file,
                "fn do_thing()",
                10,
                25,
            )
            .await;
            put_ref(
                storage,
                &format!("sym-do-{i}"),
                "sym-helper",
                RefKind::Calls,
            )
            .await;
        }

        let params = SolidParams {
            principles: vec![SolidPrinciple::ShotgunSurgery],
            shotgun_max_jaccard: 0.5,
            ..SolidParams::default()
        };
        let findings = service.detect_solid_violations(&params).await.unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f, SolidFinding::ShotgunSurgery { .. })),
            "high-Jaccard cluster must defer to redundancy, got: {findings:?}"
        );
    }
}
