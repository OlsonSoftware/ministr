//! The diagnostics "verify-stage" op for [`QueryService`] (FL5).
//!
//! Detects each local corpus root's toolchain(s) via the language-agnostic
//! registry in [`crate::code::diagnostics`], runs them, and normalises their
//! output to [`Diagnostic`]s — each cross-linked to the enclosing indexed
//! symbol (the FL1 range→symbol bridge). The registry + per-format parsers are
//! pure and unit-tested in [`crate::code::diagnostics`]; this layer adds the
//! three impure pieces: root discovery (from storage), bounded process
//! execution, and symbol mapping.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tracing::{debug, instrument, warn};

use crate::code::diagnostics::{self, Diagnostic};
use crate::storage::{Storage, SymbolFilter};
use crate::types::RootKind;

use super::{QueryError, QueryService};

/// Wall-clock cap for a single toolchain invocation. A toolchain that exceeds
/// this is abandoned (its diagnostics are simply omitted) rather than hanging
/// the op.
const TOOLCHAIN_TIMEOUT: Duration = Duration::from_secs(120);

impl QueryService {
    /// Run the project's own toolchain(s) and return structured diagnostics —
    /// the `textDocument/publishDiagnostics` analog (FL5, the agentic "verify"
    /// stage).
    ///
    /// Detects toolchains per local corpus root (`Cargo.toml` → cargo,
    /// `tsconfig.json` → tsc, `.eslintrc.json` → eslint, `pyproject.toml` →
    /// ruff, `go.mod` → go vet; SARIF-emitting tools plug in via the registry),
    /// runs each, parses its output, and cross-links every diagnostic's primary
    /// line to the enclosing symbol via the indexed symbol ranges (FL1). The
    /// result is severity-sorted (errors first) and bounded to `limit`.
    ///
    /// `languages` optionally restricts which toolchains run (by their
    /// [`diagnostics::Toolchain::language`]). When no toolchain is detected (or
    /// none is installed) the result is simply empty — never an error — so the
    /// op degrades gracefully on a project with no configured toolchain.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] only if listing corpus roots or symbols
    /// fails. Toolchain launch/parse failures are swallowed (a missing tool is
    /// not an error).
    #[instrument(skip(self))]
    pub async fn diagnostics(
        &self,
        languages: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<Diagnostic>, QueryError> {
        let roots = self.storage.list_corpus_roots().await?;
        let mut out: Vec<Diagnostic> = Vec::new();
        // file → [(line_start, line_end, symbol_id)], so range→symbol mapping
        // queries storage at most once per touched file.
        let mut sym_cache: HashMap<String, Vec<(u32, u32, String)>> = HashMap::new();

        'roots: for root in roots {
            // Diagnostics only make sense for on-disk local sources; web/git
            // roots (cloud-restored bundles) have no live toolchain.
            if !matches!(root.kind, RootKind::Local) {
                continue;
            }
            let root_path = Path::new(&root.path);
            if !root_path.is_dir() {
                continue;
            }
            for tc in diagnostics::detect_toolchains(root_path) {
                if let Some(filter) = languages
                    && !filter.iter().any(|l| l == tc.language)
                {
                    continue;
                }
                let Some(raw) = run_toolchain(tc, &root.path).await else {
                    continue;
                };
                let mut diags = diagnostics::parse_diagnostics(tc.format, tc.id, &raw, root_path);
                for d in &mut diags {
                    d.symbol_id = self
                        .symbol_for_position(&mut sym_cache, &d.file, d.line_start)
                        .await?;
                }
                out.append(&mut diags);
                if out.len() >= limit {
                    break 'roots;
                }
            }
        }

        // Errors first, then stable by file/line, and bound the set.
        out.sort_by(|a, b| {
            a.severity
                .rank()
                .cmp(&b.severity.rank())
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.line_start.cmp(&b.line_start))
        });
        out.truncate(limit);
        Ok(out)
    }

    /// Map a `file:line` to the id of the smallest indexed symbol enclosing it
    /// (the FL1 range→symbol cross-link). Caches the per-file symbol ranges so
    /// repeated diagnostics in one file cost a single storage query.
    async fn symbol_for_position(
        &self,
        cache: &mut HashMap<String, Vec<(u32, u32, String)>>,
        file: &str,
        line: u32,
    ) -> Result<Option<String>, QueryError> {
        if !cache.contains_key(file) {
            let filter = SymbolFilter {
                name: None,
                name_exact: None,
                kind: None,
                module: None,
                visibility: None,
                file_path: Some(file.to_string()),
            };
            let ranges = self
                .storage
                .list_symbols(&filter)
                .await?
                .into_iter()
                .map(|s| (s.line_start, s.line_end, s.id.0))
                .collect();
            cache.insert(file.to_string(), ranges);
        }
        // Innermost enclosing symbol wins (smallest line span containing `line`).
        let best = cache
            .get(file)
            .and_then(|ranges| {
                ranges
                    .iter()
                    .filter(|(start, end, _)| *start <= line && line <= *end)
                    .min_by_key(|(start, end, _)| end.saturating_sub(*start))
            })
            .map(|(_, _, id)| id.clone());
        Ok(best)
    }
}

/// Run one toolchain in `root`, returning its combined stdout+stderr, or `None`
/// if the program is missing / un-launchable / times out. Diagnostics degrade
/// gracefully: a missing toolchain yields no diagnostics, not an error.
async fn run_toolchain(tc: &diagnostics::Toolchain, root: &str) -> Option<String> {
    let program = tc.program.to_string();
    let args: Vec<String> = tc.args.iter().map(|a| (*a).to_string()).collect();
    let cwd = root.to_string();
    let id = tc.id;
    let join = tokio::task::spawn_blocking(move || {
        Command::new(&program)
            .args(&args)
            .current_dir(&cwd)
            .output()
    });
    match tokio::time::timeout(TOOLCHAIN_TIMEOUT, join).await {
        Ok(Ok(Ok(output))) => {
            // cargo/eslint/ruff/tsc write diagnostics to stdout; go vet/gcc to
            // stderr. Concatenate so one parser path covers both.
            let mut s = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                s.push('\n');
                s.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            Some(s)
        }
        Ok(Ok(Err(e))) => {
            debug!(toolchain = id, error = %e, "diagnostics toolchain not runnable; skipping");
            None
        }
        Ok(Err(e)) => {
            warn!(toolchain = id, error = %e, "diagnostics toolchain task panicked; skipping");
            None
        }
        Err(_) => {
            warn!(toolchain = id, "diagnostics toolchain timed out; skipping");
            None
        }
    }
}
