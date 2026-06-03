//! Language-agnostic diagnostics — the FL5 "verify" surface.
//!
//! An autonomous agent's verify loop needs compile/lint errors as *structured
//! data*, not log-scraping. This module runs the project's own toolchain(s)
//! (`cargo check`, `tsc`, `eslint`, `ruff`, `go vet`, …) and normalises every
//! tool's output to one [`Diagnostic`] shape — the
//! `textDocument/publishDiagnostics` analog.
//!
//! It is deliberately **not** Rust-specific. The design is a
//! [`Toolchain`]-adapter registry: each adapter declares how to *detect*
//! itself (a manifest file), how to *invoke* it (program + machine-readable
//! args), and which *format* its output is in. A per-format parser maps that
//! output to [`Diagnostic`]. Adding a language is a single [`builtin_toolchains`]
//! entry, plus one parser if the format is new — and any tool that can emit
//! [SARIF](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html)
//! (GCC, MSVC, ruff, clang-tidy, and a growing list) plugs in for free.
//!
//! This module is the *pure* layer: it runs no processes and touches no
//! storage. [`QueryService::diagnostics`](crate::service) orchestrates
//! execution + range→symbol mapping (FL1 occurrence index) on top of it, so
//! the parsers here are unit-tested with fixture strings and need no toolchain
//! installed.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Severity of a [`Diagnostic`], normalised across toolchains.
///
/// Mirrors the LSP `DiagnosticSeverity` ladder so a consumer can reason about
/// errors/warnings uniformly regardless of which compiler or linter produced
/// the finding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    /// A hard error — the build/typecheck failed here.
    Error,
    /// A warning — compiles, but flagged.
    Warning,
    /// Informational note.
    Info,
    /// A hint / suggestion.
    Hint,
}

impl DiagnosticSeverity {
    /// Sort key so errors float to the top of a bounded result set.
    #[must_use]
    pub fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Hint => 3,
        }
    }
}

/// One normalised compiler/linter diagnostic.
///
/// Ranges are 1-based lines / 1-based columns, matching every toolchain's
/// human-facing convention (and the stored `SymbolRecord` lines the FL1
/// mapping resolves against). `symbol_id` is filled in by
/// [`QueryService::diagnostics`](crate::service) — the FL1 range→symbol
/// cross-link — and is `None` when no indexed symbol encloses the primary
/// line (e.g. a diagnostic in a generated or unindexed file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Diagnostic {
    /// Absolute path to the file the diagnostic is anchored in.
    pub file: String,
    /// 1-based start line.
    pub line_start: u32,
    /// 1-based start column.
    pub col_start: u32,
    /// 1-based end line (== `line_start` when the tool gives no end).
    pub line_end: u32,
    /// 1-based end column (== `col_start` when the tool gives no end).
    pub col_end: u32,
    /// Normalised severity.
    pub severity: DiagnosticSeverity,
    /// Machine-readable rule/error code (e.g. `E0599`, `TS2345`,
    /// `no-unused-vars`, `F401`). `None` when the tool emits none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable message (single primary line; never the raw build log).
    pub message: String,
    /// The toolchain / tool that produced this diagnostic (`cargo`, `tsc`,
    /// `eslint`, `ruff`, `go vet`, or a SARIF `tool.driver.name`).
    pub source: String,
    /// FL1 cross-link: the id of the indexed symbol enclosing the primary
    /// line, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
}

/// The machine-readable output format a [`Toolchain`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticFormat {
    /// `cargo check --message-format=json` — newline-delimited rustc JSON.
    CargoJson,
    /// `eslint --format json` — a JSON array of `{filePath, messages[]}`.
    EslintJson,
    /// `ruff check --output-format json` — a JSON array of violations.
    RuffJson,
    /// `tsc --pretty false` — `path(line,col): error TSxxxx: message` lines.
    TscText,
    /// gcc / clang / `go vet`-style `path:line:col: severity: message` lines
    /// (severity optional — `go vet` omits it).
    GccText,
    /// SARIF 2.1.0 (OASIS) — the universal static-analysis JSON interchange
    /// that GCC, MSVC, ruff, clang-tidy and many others can emit.
    Sarif,
}

/// A toolchain adapter: how to detect, invoke, and parse one diagnostics tool.
///
/// Adapters are static data — adding a language is one entry in
/// [`builtin_toolchains`].
#[derive(Debug, Clone, Copy)]
pub struct Toolchain {
    /// Stable id, surfaced as [`Diagnostic::source`] (e.g. `cargo`).
    pub id: &'static str,
    /// Primary language this toolchain checks (`rust`, `typescript`, …).
    pub language: &'static str,
    /// Manifest filename whose presence at a corpus root selects this tool.
    pub manifest: &'static str,
    /// Executable to run.
    pub program: &'static str,
    /// Arguments that make it emit machine-readable diagnostics.
    pub args: &'static [&'static str],
    /// Output format to parse.
    pub format: DiagnosticFormat,
}

/// The built-in toolchain adapters.
///
/// One per major ecosystem ministr indexes; the registry is the single place
/// to add more. JS/TS deliberately ships two adapters (type errors via `tsc`
/// and lint findings via `eslint`) because they surface different classes of
/// problem.
#[must_use]
pub fn builtin_toolchains() -> &'static [Toolchain] {
    &[
        Toolchain {
            id: "cargo",
            language: "rust",
            manifest: "Cargo.toml",
            program: "cargo",
            args: &["check", "--message-format=json", "--quiet"],
            format: DiagnosticFormat::CargoJson,
        },
        Toolchain {
            id: "tsc",
            language: "typescript",
            manifest: "tsconfig.json",
            program: "npx",
            args: &["--no-install", "tsc", "--noEmit", "--pretty", "false"],
            format: DiagnosticFormat::TscText,
        },
        Toolchain {
            id: "eslint",
            language: "javascript",
            manifest: ".eslintrc.json",
            program: "npx",
            args: &["--no-install", "eslint", ".", "--format", "json"],
            format: DiagnosticFormat::EslintJson,
        },
        Toolchain {
            id: "ruff",
            language: "python",
            manifest: "pyproject.toml",
            program: "ruff",
            args: &["check", "--output-format", "json", "."],
            format: DiagnosticFormat::RuffJson,
        },
        Toolchain {
            id: "go vet",
            language: "go",
            manifest: "go.mod",
            program: "go",
            args: &["vet", "./..."],
            format: DiagnosticFormat::GccText,
        },
    ]
}

/// Filter the registry to the toolchains whose manifest is in `present`.
///
/// Pure (no filesystem) so detection logic is unit-testable; [`detect_toolchains`]
/// is the filesystem-backed wrapper.
#[must_use]
pub fn toolchains_for_manifests<S: std::hash::BuildHasher>(
    present: &HashSet<&str, S>,
) -> Vec<&'static Toolchain> {
    builtin_toolchains()
        .iter()
        .filter(|tc| present.contains(tc.manifest))
        .collect()
}

/// Detect which built-in toolchains apply at `root` by manifest presence.
#[must_use]
pub fn detect_toolchains(root: &Path) -> Vec<&'static Toolchain> {
    let present: HashSet<&str> = builtin_toolchains()
        .iter()
        .map(|tc| tc.manifest)
        .filter(|m| root.join(m).is_file())
        .collect();
    toolchains_for_manifests(&present)
}

/// Resolve a tool-reported path to an absolute path under `root`.
fn absolutize(root: &Path, file: &str) -> String {
    let file = file.strip_prefix("file://").unwrap_or(file);
    let p = Path::new(file);
    if p.is_absolute() {
        file.to_string()
    } else {
        root.join(p).to_string_lossy().into_owned()
    }
}

/// Read an optional JSON number field as a `u32`, clamping a missing or
/// out-of-range value to `default` (line/column numbers never legitimately
/// exceed `u32`).
fn u32_or(v: Option<&serde_json::Value>, default: u32) -> u32 {
    v.and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(default)
}

/// Parse one toolchain's raw output into normalised [`Diagnostic`]s.
///
/// `source` is the toolchain id; `root` is its working directory (used to
/// absolutise relative paths). Always returns a (possibly empty) vec — a parse
/// miss yields no diagnostics rather than an error, so a noisy or partial tool
/// run degrades gracefully.
#[must_use]
pub fn parse_diagnostics(
    format: DiagnosticFormat,
    source: &str,
    output: &str,
    root: &Path,
) -> Vec<Diagnostic> {
    match format {
        DiagnosticFormat::CargoJson => parse_cargo_json(source, output, root),
        DiagnosticFormat::EslintJson => parse_eslint_json(source, output, root),
        DiagnosticFormat::RuffJson => parse_ruff_json(source, output, root),
        DiagnosticFormat::TscText => parse_tsc_text(source, output, root),
        DiagnosticFormat::GccText => parse_gcc_text(source, output, root),
        DiagnosticFormat::Sarif => parse_sarif(output, root),
    }
}

fn rustc_level(level: &str) -> Option<DiagnosticSeverity> {
    match level {
        "error" => Some(DiagnosticSeverity::Error),
        "warning" => Some(DiagnosticSeverity::Warning),
        "note" => Some(DiagnosticSeverity::Info),
        "help" => Some(DiagnosticSeverity::Hint),
        // "failure-note" and friends carry no span — skip.
        _ => None,
    }
}

/// `cargo check --message-format=json`: newline-delimited rustc JSON. We keep
/// `reason == "compiler-message"` entries with a primary span under `root`.
fn parse_cargo_json(source: &str, output: &str, root: &Path) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(severity) = msg
            .get("level")
            .and_then(|l| l.as_str())
            .and_then(rustc_level)
        else {
            continue;
        };
        let spans = msg.get("spans").and_then(|s| s.as_array());
        let Some(spans) = spans else { continue };
        // Prefer the primary span; fall back to the first span.
        let span = spans
            .iter()
            .find(|s| s.get("is_primary").and_then(serde_json::Value::as_bool) == Some(true))
            .or_else(|| spans.first());
        let Some(span) = span else { continue };
        let file = match span.get("file_name").and_then(|f| f.as_str()) {
            Some(f) => absolutize(root, f),
            None => continue,
        };
        // Skip diagnostics in dependencies (outside this root).
        if !Path::new(&file).starts_with(root) {
            continue;
        }
        let line_start = u32_or(span.get("line_start"), 0);
        let line_end = u32_or(span.get("line_end"), line_start);
        let col_start = u32_or(span.get("column_start"), 1);
        let col_end = u32_or(span.get("column_end"), col_start);
        let code = msg
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .map(String::from);
        let message = msg
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        out.push(Diagnostic {
            file,
            line_start,
            col_start,
            line_end,
            col_end,
            severity,
            code,
            message,
            source: source.to_string(),
            symbol_id: None,
        });
    }
    out
}

/// `eslint --format json`: `[{filePath, messages: [{line, column, endLine,
/// endColumn, severity (1=warn|2=error), ruleId, message}]}]`.
fn parse_eslint_json(source: &str, output: &str, root: &Path) -> Vec<Diagnostic> {
    let Ok(files) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };
    let Some(files) = files.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in files {
        let Some(path) = f.get("filePath").and_then(|p| p.as_str()) else {
            continue;
        };
        let file = absolutize(root, path);
        let Some(messages) = f.get("messages").and_then(|m| m.as_array()) else {
            continue;
        };
        for m in messages {
            let severity = match m.get("severity").and_then(serde_json::Value::as_u64) {
                Some(2) => DiagnosticSeverity::Error,
                _ => DiagnosticSeverity::Warning,
            };
            let line_start = u32_or(m.get("line"), 1);
            let col_start = u32_or(m.get("column"), 1);
            let line_end = u32_or(m.get("endLine"), line_start);
            let col_end = u32_or(m.get("endColumn"), col_start);
            let code = m.get("ruleId").and_then(|r| r.as_str()).map(String::from);
            let message = m
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or_default()
                .to_string();
            out.push(Diagnostic {
                file: file.clone(),
                line_start,
                col_start,
                line_end,
                col_end,
                severity,
                code,
                message,
                source: source.to_string(),
                symbol_id: None,
            });
        }
    }
    out
}

/// `ruff check --output-format json`: `[{code, message, filename,
/// location:{row,column}, end_location:{row,column}}]`.
fn parse_ruff_json(source: &str, output: &str, root: &Path) -> Vec<Diagnostic> {
    let Ok(items) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };
    let Some(items) = items.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in items {
        let Some(path) = it.get("filename").and_then(|f| f.as_str()) else {
            continue;
        };
        let file = absolutize(root, path);
        let loc = it.get("location");
        let end = it.get("end_location");
        let line_start = u32_or(loc.and_then(|l| l.get("row")), 1);
        let col_start = u32_or(loc.and_then(|l| l.get("column")), 1);
        let line_end = u32_or(end.and_then(|l| l.get("row")), line_start);
        let col_end = u32_or(end.and_then(|l| l.get("column")), col_start);
        let code = it.get("code").and_then(|c| c.as_str()).map(String::from);
        // Ruff syntax errors (E999) are hard errors; lints are warnings.
        let severity = match code.as_deref() {
            Some(c) if c.starts_with("E9") => DiagnosticSeverity::Error,
            _ => DiagnosticSeverity::Warning,
        };
        let message = it
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        out.push(Diagnostic {
            file,
            line_start,
            col_start,
            line_end,
            col_end,
            severity,
            code,
            message,
            source: source.to_string(),
            symbol_id: None,
        });
    }
    out
}

/// `tsc --pretty false`: `path(line,col): error TSxxxx: message`.
fn parse_tsc_text(source: &str, output: &str, root: &Path) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        // Split `file(l,c)` from `: <severity> <code>: <message>`.
        let Some(paren) = line.find('(') else {
            continue;
        };
        let Some(close_rel) = line[paren + 1..].find(')') else {
            continue;
        };
        let close = paren + 1 + close_rel;
        let file_part = &line[..paren];
        let coords = &line[paren + 1..close];
        let mut nums = coords.split(',');
        let (Some(l), Some(c)) = (nums.next(), nums.next()) else {
            continue;
        };
        let (Ok(line_start), Ok(col_start)) = (l.trim().parse::<u32>(), c.trim().parse::<u32>())
        else {
            continue;
        };
        // Remainder: `: error TS2345: message`.
        let rest = line[close + 1..].trim_start_matches([':', ' ']);
        let severity = if rest.starts_with("error") {
            DiagnosticSeverity::Error
        } else if rest.starts_with("warning") {
            DiagnosticSeverity::Warning
        } else {
            continue;
        };
        // `error TS2345: message` -> code = TS2345, message after the colon.
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let head = rest[..colon].trim(); // "error TS2345"
        let message = rest[colon + 1..].trim().to_string();
        let code = head.split_whitespace().nth(1).map(String::from);
        out.push(Diagnostic {
            file: absolutize(root, file_part),
            line_start,
            col_start,
            line_end: line_start,
            col_end: col_start,
            severity,
            code,
            message,
            source: source.to_string(),
            symbol_id: None,
        });
    }
    out
}

/// gcc / clang / `go vet`: `path:line:col: severity: message` (severity and
/// column both optional). Paths are assumed colon-free (the Unix case).
fn parse_gcc_text(source: &str, output: &str, root: &Path) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        // The first ": " separates `path:line[:col]` from the message; the
        // colons inside the location are not followed by a space.
        let Some(sep) = line.find(": ") else { continue };
        let (loc, mut message) = (&line[..sep], line[sep + 2..].to_string());
        // loc = path:line[:col]  (rsplit so a colon-free path survives)
        let mut parts = loc.rsplitn(3, ':');
        let last = parts.next();
        let mid = parts.next();
        let first = parts.next();
        let (line_start, col_start) = match (first, mid, last) {
            // path:line:col
            (Some(_path), Some(l), Some(c)) => {
                let (Ok(l), Ok(c)) = (l.parse::<u32>(), c.parse::<u32>()) else {
                    continue;
                };
                (l, c)
            }
            // path:line
            (None, Some(_path), Some(l)) => {
                let Ok(l) = l.parse::<u32>() else { continue };
                (l, 1)
            }
            _ => continue,
        };
        // Path is everything before the parsed line/col.
        let path_len = loc.len()
            - 1
            - last.map_or(0, str::len)
            - if first.is_some() {
                1 + mid.map_or(0, str::len)
            } else {
                0
            };
        let file_part = &loc[..path_len];
        // Strip an optional leading severity word from the message.
        let severity = if let Some(rest) = message.strip_prefix("error: ") {
            message = rest.to_string();
            DiagnosticSeverity::Error
        } else if let Some(rest) = message.strip_prefix("warning: ") {
            message = rest.to_string();
            DiagnosticSeverity::Warning
        } else if let Some(rest) = message.strip_prefix("note: ") {
            message = rest.to_string();
            DiagnosticSeverity::Info
        } else {
            // `go vet` emits no severity word; its findings are errors.
            DiagnosticSeverity::Error
        };
        out.push(Diagnostic {
            file: absolutize(root, file_part),
            line_start,
            col_start,
            line_end: line_start,
            col_end: col_start,
            severity,
            code: None,
            message,
            source: source.to_string(),
            symbol_id: None,
        });
    }
    out
}

fn sarif_level(level: &str) -> DiagnosticSeverity {
    match level {
        "error" => DiagnosticSeverity::Error,
        "note" => DiagnosticSeverity::Info,
        "none" => DiagnosticSeverity::Hint,
        // "warning" is the SARIF default.
        _ => DiagnosticSeverity::Warning,
    }
}

/// SARIF 2.1.0: `{runs: [{tool:{driver:{name}}, results: [{ruleId, level,
/// message:{text}, locations: [{physicalLocation:{artifactLocation:{uri},
/// region:{startLine, startColumn, endLine, endColumn}}}]}]}]}`.
///
/// The universal path: any tool that emits SARIF is consumed here with no new
/// adapter, and `source` is taken from the SARIF run's own `tool.driver.name`.
fn parse_sarif(output: &str, root: &Path) -> Vec<Diagnostic> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };
    let Some(runs) = doc.get("runs").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for run in runs {
        let tool = run
            .get("tool")
            .and_then(|t| t.get("driver"))
            .and_then(|d| d.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("sarif")
            .to_string();
        let Some(results) = run.get("results").and_then(|r| r.as_array()) else {
            continue;
        };
        for r in results {
            let severity =
                sarif_level(r.get("level").and_then(|l| l.as_str()).unwrap_or("warning"));
            let code = r.get("ruleId").and_then(|c| c.as_str()).map(String::from);
            let message = r
                .get("message")
                .and_then(|m| m.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string();
            let phys = r
                .get("locations")
                .and_then(|l| l.as_array())
                .and_then(|l| l.first())
                .and_then(|l| l.get("physicalLocation"));
            let Some(phys) = phys else { continue };
            let uri = phys
                .get("artifactLocation")
                .and_then(|a| a.get("uri"))
                .and_then(|u| u.as_str());
            let Some(uri) = uri else { continue };
            let region = phys.get("region");
            let line_start = u32_or(region.and_then(|r| r.get("startLine")), 1);
            let col_start = u32_or(region.and_then(|r| r.get("startColumn")), 1);
            let line_end = u32_or(region.and_then(|r| r.get("endLine")), line_start);
            let col_end = u32_or(region.and_then(|r| r.get("endColumn")), col_start);
            out.push(Diagnostic {
                file: absolutize(root, uri),
                line_start,
                col_start,
                line_end,
                col_end,
                severity,
                code,
                message,
                source: tool.clone(),
                symbol_id: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new("/proj")
    }

    #[test]
    fn detect_filters_registry_by_manifest() {
        let rust: HashSet<&str> = ["Cargo.toml"].into_iter().collect();
        let ids: Vec<&str> = toolchains_for_manifests(&rust)
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["cargo"]);

        let ts: HashSet<&str> = ["tsconfig.json", ".eslintrc.json"].into_iter().collect();
        let mut ids: Vec<&str> = toolchains_for_manifests(&ts).iter().map(|t| t.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["eslint", "tsc"]);

        assert!(toolchains_for_manifests(&HashSet::new()).is_empty());
    }

    #[test]
    fn cargo_json_extracts_primary_span() {
        // A real-shaped rustc compiler-message line + a build-finished line.
        let out = r#"{"reason":"compiler-message","message":{"level":"error","code":{"code":"E0599"},"message":"no method named `foo`","spans":[{"file_name":"src/lib.rs","line_start":42,"line_end":42,"column_start":9,"column_end":12,"is_primary":true}]}}
{"reason":"build-finished","success":false}"#;
        let diags = parse_cargo_json("cargo", out, root());
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.file, "/proj/src/lib.rs");
        assert_eq!(d.line_start, 42);
        assert_eq!(d.col_start, 9);
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.code.as_deref(), Some("E0599"));
        assert_eq!(d.source, "cargo");
    }

    #[test]
    fn cargo_json_skips_dependency_spans() {
        let out = r#"{"reason":"compiler-message","message":{"level":"warning","code":null,"message":"unused","spans":[{"file_name":"/home/u/.cargo/registry/src/dep/lib.rs","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}"#;
        assert!(parse_cargo_json("cargo", out, root()).is_empty());
    }

    #[test]
    fn eslint_json_maps_severity_and_rule() {
        let out = r#"[{"filePath":"/proj/src/a.js","messages":[{"ruleId":"no-unused-vars","severity":2,"message":"'x' is defined but never used","line":3,"column":7,"endLine":3,"endColumn":8}]}]"#;
        let diags = parse_eslint_json("eslint", out, root());
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.file, "/proj/src/a.js");
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.code.as_deref(), Some("no-unused-vars"));
        assert_eq!(d.line_start, 3);
    }

    #[test]
    fn ruff_json_relative_path_and_syntax_error() {
        let out = r#"[{"code":"F401","message":"`os` imported but unused","filename":"app/main.py","location":{"row":1,"column":8},"end_location":{"row":1,"column":10}},{"code":"E999","message":"SyntaxError","filename":"app/bad.py","location":{"row":2,"column":1},"end_location":{"row":2,"column":2}}]"#;
        let diags = parse_ruff_json("ruff", out, root());
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].file, "/proj/app/main.py");
        assert_eq!(diags[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diags[0].code.as_deref(), Some("F401"));
        assert_eq!(diags[1].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn tsc_text_parses_line_format() {
        let out = "src/x.ts(12,5): error TS2345: Argument of type 'string' is not assignable.\nsrc/y.ts(1,1): warning TS6133: 'a' is declared but never read.";
        let diags = parse_tsc_text("tsc", out, root());
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].file, "/proj/src/x.ts");
        assert_eq!(diags[0].line_start, 12);
        assert_eq!(diags[0].col_start, 5);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[0].code.as_deref(), Some("TS2345"));
        assert_eq!(diags[1].severity, DiagnosticSeverity::Warning);
        assert_eq!(diags[1].code.as_deref(), Some("TS6133"));
    }

    #[test]
    fn gcc_text_handles_go_vet_and_clang() {
        // go vet: no severity word, relative path, line:col.
        // clang: severity word present.
        let out = "pkg/m.go:10:6: result of fmt.Sprintf call not used\nsrc/a.c:3:5: error: use of undeclared identifier 'foo'";
        let diags = parse_gcc_text("go vet", out, root());
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].file, "/proj/pkg/m.go");
        assert_eq!(diags[0].line_start, 10);
        assert_eq!(diags[0].col_start, 6);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error); // go vet -> error
        assert_eq!(diags[0].message, "result of fmt.Sprintf call not used");
        assert_eq!(diags[1].file, "/proj/src/a.c");
        assert_eq!(diags[1].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[1].message, "use of undeclared identifier 'foo'");
    }

    #[test]
    fn sarif_universal_parser_uses_driver_name() {
        let out = r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"clang-tidy"}},"results":[{"ruleId":"bugprone-foo","level":"error","message":{"text":"possible bug"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/z.cpp"},"region":{"startLine":7,"startColumn":2,"endLine":7,"endColumn":9}}}]}]}]}"#;
        let diags = parse_sarif(out, root());
        assert_eq!(diags.len(), 1);
        let d = &diags[0];
        assert_eq!(d.source, "clang-tidy"); // SARIF self-describes the tool
        assert_eq!(d.file, "/proj/src/z.cpp");
        assert_eq!(d.line_start, 7);
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.code.as_deref(), Some("bugprone-foo"));
    }

    #[test]
    fn severity_rank_orders_errors_first() {
        let mut sevs = [
            DiagnosticSeverity::Hint,
            DiagnosticSeverity::Error,
            DiagnosticSeverity::Warning,
            DiagnosticSeverity::Info,
        ];
        sevs.sort_by_key(|s| s.rank());
        assert_eq!(sevs[0], DiagnosticSeverity::Error);
        assert_eq!(sevs[3], DiagnosticSeverity::Hint);
    }
}
