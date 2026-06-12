//! Project initialization and `.ministr.toml` generation.
//!
//! Auto-detects project structure by scanning manifests (`Cargo.toml`,
//! `package.json`, `pyproject.toml`) and workspace layouts, then generates
//! a sensible `.ministr.toml` configuration file with commented sections.
//!
//! # Examples
//!
//! ```no_run
//! use ministr_core::init::write_config;
//! use std::path::Path;
//!
//! let detection = write_config(Path::new("."), false).unwrap();
//! println!("Detected: {}", detection.project_name);
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::code::bridge::BridgeKind;
use crate::code::bridge::detector::FrameworkDetector;
use crate::config::CORPUS_CONFIG_FILENAME;
use crate::workspace::{WorkspaceInfo, WorkspaceKind, detect_all_workspaces};

/// Classified project type, inferred from manifests and directory structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectType {
    /// Multi-package workspace (Cargo workspace, npm workspaces, pnpm, etc.).
    Monorepo,
    /// Reusable library crate/package (no binaries, lib-only).
    Library,
    /// Command-line tool (binary with no web server indicators).
    Cli,
    /// Web application (frontend framework or fullstack with UI).
    WebApp,
    /// HTTP API service (server with routes, no frontend).
    Api,
    /// Could not determine a specific type.
    Unknown,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Monorepo => "monorepo",
            Self::Library => "library",
            Self::Cli => "cli",
            Self::WebApp => "web-app",
            Self::Api => "api",
            Self::Unknown => "unknown",
        })
    }
}

/// Summary of what was detected in a project directory.
#[derive(Debug, Clone)]
pub struct ProjectDetection {
    /// Human-readable project name (derived from directory name or manifest).
    pub project_name: String,
    /// Classified project type (monorepo, library, cli, web-app, api).
    pub project_type: ProjectType,
    /// Detected workspace layouts (Cargo, npm, pnpm, etc.).
    pub workspaces: Vec<WorkspaceInfo>,
    /// Detected cross-language bridge frameworks.
    pub bridges: Vec<BridgeKind>,
    /// Whether a Rust project was detected (`Cargo.toml` present).
    pub has_rust: bool,
    /// Whether a Node.js project was detected (`package.json` present).
    pub has_node: bool,
    /// Whether a Python project was detected (`pyproject.toml` or `setup.py`).
    pub has_python: bool,
    /// Whether a Go project was detected (`go.mod` present).
    pub has_go: bool,
    /// Whether a Java/Kotlin project was detected (`pom.xml` or `build.gradle`).
    pub has_java: bool,
    /// Whether a PHP project was detected (`composer.json` present).
    pub has_php: bool,
    /// Whether a Ruby project was detected (`Gemfile` or `*.gemspec`).
    pub has_ruby: bool,
    /// Whether a C# project was detected (`*.csproj` or `*.sln`).
    pub has_csharp: bool,
    /// Whether a Kotlin project was detected (`*.gradle.kts`).
    pub has_kotlin: bool,
    /// Whether a Swift package was detected (`Package.swift`).
    pub has_swift: bool,
    /// Whether a Scala project was detected (`build.sbt`).
    pub has_scala: bool,
    /// Whether a C/C++ project was detected (`CMakeLists.txt`).
    pub has_cpp: bool,
    /// Whether an Elixir project was detected (`mix.exs`).
    pub has_elixir: bool,
    /// Whether a JavaScript (non-TypeScript) project was detected
    /// (`package.json` present, no `tsconfig.json`).
    pub has_javascript: bool,
    /// Relative paths to source directories.
    pub source_paths: Vec<String>,
    /// Relative paths to documentation files/directories.
    pub doc_paths: Vec<String>,
    /// Suggested ignore patterns for `.ministr.toml`.
    pub ignore_patterns: Vec<String>,
}

/// Primary language detected in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    Php,
    Ruby,
    Csharp,
    Kotlin,
    Swift,
    Scala,
    Cpp,
    Elixir,
}

impl ProjectDetection {
    /// Return the list of primary languages detected in this project.
    #[must_use]
    pub fn detected_languages(&self) -> Vec<Language> {
        let mut langs = Vec::new();
        if self.has_rust {
            langs.push(Language::Rust);
        }
        if self.has_node {
            langs.push(Language::TypeScript);
        }
        // A Node project with no tsconfig.json also gets JS guidance.
        if self.has_javascript {
            langs.push(Language::JavaScript);
        }
        if self.has_python {
            langs.push(Language::Python);
        }
        if self.has_go {
            langs.push(Language::Go);
        }
        if self.has_java {
            langs.push(Language::Java);
        }
        if self.has_php {
            langs.push(Language::Php);
        }
        if self.has_ruby {
            langs.push(Language::Ruby);
        }
        if self.has_csharp {
            langs.push(Language::Csharp);
        }
        if self.has_kotlin {
            langs.push(Language::Kotlin);
        }
        if self.has_swift {
            langs.push(Language::Swift);
        }
        if self.has_scala {
            langs.push(Language::Scala);
        }
        if self.has_cpp {
            langs.push(Language::Cpp);
        }
        if self.has_elixir {
            langs.push(Language::Elixir);
        }
        langs
    }
}

/// Errors that can occur during `ministr init`.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The config file already exists and `--force` was not specified.
    #[error(".ministr.toml already exists at {}", path.display())]
    AlreadyExists {
        /// Path to the existing config file.
        path: PathBuf,
    },

    /// Filesystem I/O error.
    #[error("I/O error: {source}")]
    Io {
        /// The underlying I/O error.
        #[from]
        source: std::io::Error,
    },
}

/// Whether the top level of `root` contains a file with any of the given
/// extensions (case-sensitive, no leading dot).
fn dir_has_extension(root: &Path, exts: &[&str]) -> bool {
    std::fs::read_dir(root).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.contains(&x))
        })
    })
}

/// Detect project structure at `root` and build a [`ProjectDetection`].
///
/// Scans for manifests, workspace layouts, bridge frameworks, source
/// directories, and documentation files. All returned paths are relative
/// to `root`.
#[must_use]
pub fn detect_project(root: &Path) -> ProjectDetection {
    let workspaces = detect_all_workspaces(root);
    let bridges = FrameworkDetector::detect(root);

    let has_rust = root.join("Cargo.toml").exists();
    let has_node = root.join("package.json").exists();
    let has_python = root.join("pyproject.toml").exists() || root.join("setup.py").exists();
    let has_go = root.join("go.mod").exists();
    let has_java = root.join("pom.xml").exists()
        || root.join("build.gradle").exists()
        || root.join("build.gradle.kts").exists();
    let has_php = root.join("composer.json").exists();
    let has_ruby = root.join("Gemfile").exists()
        || std::fs::read_dir(root).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x == "gemspec")
            })
        });

    let has_csharp = dir_has_extension(root, &["csproj", "sln"]);
    let has_kotlin =
        root.join("build.gradle.kts").exists() || root.join("settings.gradle.kts").exists();
    let has_swift = root.join("Package.swift").exists();
    let has_scala = root.join("build.sbt").exists();
    let has_cpp = root.join("CMakeLists.txt").exists();
    let has_elixir = root.join("mix.exs").exists();
    // Node project with no TypeScript config → treat as JavaScript.
    let has_javascript = has_node && !root.join("tsconfig.json").exists();

    let project_name = derive_project_name(root);
    let doc_paths = detect_doc_paths(root);
    let mut project_type = classify_project_type(root, &workspaces, &bridges, has_rust, has_node);

    // Build the detection with empty path lists first, then derive
    // source/ignore paths from the full picture (every detected
    // language, the project type, the workspace layout) rather than the
    // old rust/node/python-only trio.
    let mut detection = ProjectDetection {
        project_name,
        project_type,
        workspaces,
        bridges,
        has_rust,
        has_node,
        has_python,
        has_go,
        has_java,
        has_php,
        has_ruby,
        has_csharp,
        has_kotlin,
        has_swift,
        has_scala,
        has_cpp,
        has_elixir,
        has_javascript,
        source_paths: Vec::new(),
        doc_paths,
        ignore_patterns: Vec::new(),
    };

    // Smarter polyglot classification: a repo mixing ≥2 independent
    // language ecosystems at the root (no formal workspace file) is
    // effectively a monorepo for indexing purposes.
    if matches!(project_type, ProjectType::Unknown) && ecosystem_count(&detection) >= 2 {
        project_type = ProjectType::Monorepo;
        detection.project_type = project_type;
    }

    detection.source_paths = detect_source_paths(root, &detection);
    detection.ignore_patterns = default_ignore_patterns(root, &detection);
    detection
}

/// Number of independent language ecosystems detected at the root. Used
/// to recognise informal polyglot monorepos (multiple stacks side by
/// side without a Cargo/npm/pnpm/Nx workspace manifest).
fn ecosystem_count(d: &ProjectDetection) -> usize {
    [
        d.has_rust,
        d.has_node,
        d.has_python,
        d.has_go,
        d.has_java,
        d.has_php,
        d.has_ruby,
        d.has_csharp,
        d.has_swift,
        d.has_scala,
        d.has_cpp,
        d.has_elixir,
    ]
    .into_iter()
    .filter(|&b| b)
    .count()
}

/// Generate a commented TOML string from a [`ProjectDetection`].
///
/// The output includes inline comments explaining each section and is
/// suitable for writing directly to `.ministr.toml`.
#[must_use]
#[allow(clippy::too_many_lines)] // template rendering — splitting would scatter the template
pub fn render_toml(detection: &ProjectDetection) -> String {
    let mut out = String::new();

    // Header comment
    let _ = writeln!(
        out,
        "# ministr corpus configuration — generated by `ministr init`"
    );

    // Detection summary
    let mut summary_parts = Vec::new();
    for ws in &detection.workspaces {
        summary_parts.push(format!(
            "{} workspace ({} members)",
            ws.kind,
            ws.members.len()
        ));
    }
    if detection.has_rust
        && !detection
            .workspaces
            .iter()
            .any(|w| w.kind == WorkspaceKind::Cargo)
    {
        summary_parts.push("Rust crate".to_string());
    }
    if detection.has_node
        && !detection.workspaces.iter().any(|w| {
            matches!(
                w.kind,
                WorkspaceKind::Npm | WorkspaceKind::Yarn | WorkspaceKind::Pnpm
            )
        })
    {
        summary_parts.push("Node.js project".to_string());
    }
    if detection.has_python {
        summary_parts.push("Python project".to_string());
    }
    if !detection.bridges.is_empty() {
        let bridge_names: Vec<_> = detection
            .bridges
            .iter()
            .map(|b| b.as_str().to_string())
            .collect();
        summary_parts.push(format!("bridges: {}", bridge_names.join(", ")));
    }
    if summary_parts.is_empty() {
        summary_parts.push("generic project".to_string());
    }

    let _ = writeln!(out, "# Detected: {}", summary_parts.join(", "));
    let _ = writeln!(out, "#");
    let _ = writeln!(
        out,
        "# ministr automatically ignores target/, node_modules/, __pycache__, .git/, etc."
    );
    let _ = writeln!(
        out,
        "# Add extra ignore patterns below for project-specific exclusions."
    );
    let _ = writeln!(out);

    // [corpus] section
    let _ = writeln!(out, "[corpus]");

    // paths
    let _ = writeln!(out, "paths = [");
    if !detection.source_paths.is_empty() {
        let _ = writeln!(out, "    # Source code");
        for p in &detection.source_paths {
            let _ = writeln!(out, "    \"{p}\",");
        }
    }
    if !detection.doc_paths.is_empty() {
        if !detection.source_paths.is_empty() {
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "    # Documentation");
        for p in &detection.doc_paths {
            let _ = writeln!(out, "    \"{p}\",");
        }
    }
    let _ = writeln!(out, "]");

    // ignore
    if !detection.ignore_patterns.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "ignore = [");
        for p in &detection.ignore_patterns {
            let _ = writeln!(out, "    \"{p}\",");
        }
        let _ = writeln!(out, "]");
    }

    // sparse_weight — per-corpus-type hybrid retrieval default (rq4c/W5).
    // VISIBLE default, never silent: code projects get the measured 0.6 in
    // the generated file where the user reviews it before the first index;
    // docs-only projects get the keep-it-off guidance as a comment. The
    // measurement (deterministic eval, 2026-06): code nDCG@5 +23.6% at 0.6
    // (the knee), while docs precision regresses at EVERY weight — there is
    // no good global default, so the value is per-corpus by construction.
    let _ = writeln!(out);
    if ecosystem_count(detection) > 0 {
        let _ = writeln!(
            out,
            "# Hybrid retrieval: fuse keyword (sparse) and semantic (dense) search."
        );
        let _ = writeln!(
            out,
            "# 0.6 is the measured sweet spot for code corpora (exact identifiers"
        );
        let _ = writeln!(
            out,
            "# rank first). First index downloads a small sparse model (~100 MB)"
        );
        let _ = writeln!(
            out,
            "# and ingest does extra inference. Delete the line (or set 0) to"
        );
        let _ = writeln!(out, "# stay dense-only.");
        let _ = writeln!(out, "sparse_weight = 0.6");
    } else {
        let _ = writeln!(
            out,
            "# Hybrid retrieval (sparse_weight) is OFF: on documentation/prose"
        );
        let _ = writeln!(
            out,
            "# corpora keyword fusion measurably hurts precision. For corpora"
        );
        let _ = writeln!(out, "# that are mostly code, set:");
        let _ = writeln!(out, "# sparse_weight = 0.6");
    }

    // [agent] section — custom rules injected into generated agent configs
    let _ = writeln!(out);
    let _ = writeln!(out, "# [agent]");
    let _ = writeln!(
        out,
        "# Custom rules appended to all generated agent instruction files."
    );
    let _ = writeln!(
        out,
        "# Each entry is a line appended to .claude/rules/, .cursor/rules/, etc."
    );
    let _ = writeln!(out, "# rules = [");
    let _ = writeln!(out, "#     \"Always use snake_case for function names.\",");
    let _ = writeln!(
        out,
        "#     \"Prefer Result<T, E> over panic in library code.\","
    );
    let _ = writeln!(out, "# ]");

    out
}

/// Write `.ministr.toml` to `root`, failing if it already exists (unless `force`).
///
/// Returns the [`ProjectDetection`] so the caller can display what was found.
///
/// # Errors
///
/// Returns [`InitError::AlreadyExists`] if the file exists and `force` is false.
/// Returns [`InitError::Io`] on filesystem errors.
pub fn write_config(root: &Path, force: bool) -> Result<ProjectDetection, InitError> {
    let config_path = root.join(CORPUS_CONFIG_FILENAME);
    let detection = detect_project(root);

    if config_path.exists() && !force {
        // .ministr.toml already exists — skip it, but still write MCP configs.
        write_mcp_configs(root)?;
        return Ok(detection);
    }

    let toml_str = render_toml(&detection);
    std::fs::write(&config_path, toml_str)?;

    // Write MCP client configs (Claude Code + Copilot).
    write_mcp_configs(root)?;

    Ok(detection)
}

/// Identifies a supported MCP client.
///
/// Used by [`write_mcp_config`] (per-client write) and the Tauri MCP
/// wizard surface to select which client's config file to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum McpClientId {
    /// Anthropic Claude Code CLI. Reads `.mcp.json` in the project root.
    ClaudeCode,
    /// Cursor editor. Reads `.cursor/mcp.json` in the project root.
    Cursor,
    /// VS Code GitHub Copilot. Reads `.vscode/mcp.json` in the project root.
    VsCode,
    /// OpenAI Codex CLI. Reads `~/.codex/config.toml` (user-level, not
    /// per-project — see [`write_codex_mcp`] for the path resolution
    /// rationale).
    Codex,
}

impl McpClientId {
    /// Stable identifier suitable for IPC / config files.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Cursor => "cursor",
            Self::VsCode => "vscode",
            Self::Codex => "codex",
        }
    }

    /// Parse from a wire-format identifier produced by [`Self::as_str`].
    ///
    /// Named `parse` rather than `from_str` to avoid clashing with
    /// `std::str::FromStr::from_str`. Implementing FromStr would force a
    /// concrete error type on every caller, which is overkill for a
    /// closed enum like this.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude_code" => Some(Self::ClaudeCode),
            "cursor" => Some(Self::Cursor),
            "vscode" => Some(Self::VsCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    /// Human-readable label for display in UI.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
            Self::VsCode => "GitHub Copilot (VS Code)",
            Self::Codex => "Codex",
        }
    }
}

/// Write the MCP config for a single client.
///
/// Returns the absolute path of the file that was written so the caller
/// (typically the Tauri wizard) can show the user *exactly* which file
/// changed.
///
/// `root` is the project root. It is ignored for [`McpClientId::Codex`],
/// which writes a user-global file.
///
/// # Errors
///
/// Returns [`InitError::Io`] on filesystem errors.
pub fn write_mcp_config(client: McpClientId, root: &Path) -> Result<PathBuf, InitError> {
    match client {
        McpClientId::ClaudeCode => write_claude_mcp(root),
        McpClientId::Cursor => write_cursor_mcp(root),
        McpClientId::VsCode => write_vscode_mcp(root),
        McpClientId::Codex => write_codex_mcp(),
    }
}

/// Write MCP client configuration files for every project-scoped client
/// (Claude Code, VS Code Copilot, Cursor).
///
/// This is the bulk path used by `ministr init` so the user gets every
/// project file written in one shot. The interactive wizard prefers
/// [`write_mcp_config`] so it can target a single client at a time.
///
/// Codex is **not** included here because it's user-global, not
/// per-project — `ministr init` shouldn't reach into `~/.codex/`
/// without explicit consent.
///
/// # Errors
///
/// Returns [`InitError::Io`] on filesystem errors.
pub fn write_mcp_configs(root: &Path) -> Result<(), InitError> {
    write_claude_mcp(root)?;
    write_vscode_mcp(root)?;
    write_cursor_mcp(root)?;
    Ok(())
}

/// Write `.mcp.json` (Claude Code) under `root`.
fn write_claude_mcp(root: &Path) -> Result<PathBuf, InitError> {
    write_mcp_json_relative(root, ".mcp.json")
}

/// Write `.cursor/mcp.json` (Cursor) under `root`.
fn write_cursor_mcp(root: &Path) -> Result<PathBuf, InitError> {
    let cursor_dir = root.join(".cursor");
    if !cursor_dir.exists() {
        std::fs::create_dir_all(&cursor_dir)?;
    }
    write_mcp_json_relative(root, ".cursor/mcp.json")
}

/// Write `.vscode/mcp.json` (VS Code / GitHub Copilot) under `root`.
fn write_vscode_mcp(root: &Path) -> Result<PathBuf, InitError> {
    let vscode_dir = root.join(".vscode");
    if !vscode_dir.exists() {
        std::fs::create_dir_all(&vscode_dir)?;
    }
    write_mcp_json_relative(root, ".vscode/mcp.json")
}

/// Write or merge an ministr entry into a per-project MCP JSON config file.
fn write_mcp_json_relative(root: &Path, relative_path: &str) -> Result<PathBuf, InitError> {
    let path = root.join(relative_path);

    let ministr_entry = serde_json::json!({
        "command": "ministr",
        "args": ["serve", "--transport", "stdio"]
    });

    let mut config: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Always update the ministr entry to ensure correct args.
    let servers = config.as_object_mut().and_then(|o| {
        o.entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
    });

    if let Some(servers) = servers {
        servers.insert("ministr".to_string(), ministr_entry);
        let json_str = serde_json::to_string_pretty(&config).unwrap_or_default();
        std::fs::write(&path, format!("{json_str}\n"))?;
    }

    Ok(path)
}

/// Write or merge a `[mcp_servers.ministr]` entry into the user-global
/// Codex CLI config at `~/.codex/config.toml`.
///
/// The Codex CLI's MCP support is configured via TOML (not JSON) and
/// lives under the user's home directory rather than per-project — this
/// matches the standard OpenAI Codex CLI layout.
///
/// We do a simple text patch rather than a full TOML round-trip: if the
/// file exists and already has a `[mcp_servers.ministr]` section, we
/// rewrite that block; otherwise we append. This keeps existing user
/// edits in other sections intact even when our parser would round-trip
/// poorly (Codex's config doc strings, comments, ordering all matter to
/// users editing this file by hand).
fn write_codex_mcp() -> Result<PathBuf, InitError> {
    let home = home_dir().ok_or_else(|| InitError::Io {
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not resolve home directory for Codex config",
        ),
    })?;
    let dir = home.join(".codex");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    let path = dir.join("config.toml");

    let block = "\n[mcp_servers.ministr]\ncommand = \"ministr\"\nargs = [\"serve\", \"--transport\", \"stdio\"]\n";

    let mut existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    if let Some(start) = existing.find("[mcp_servers.ministr]") {
        // Find end of this section: the next `[` at the start of a line,
        // or end-of-file. Strip + reappend.
        let after_header = start + "[mcp_servers.ministr]".len();
        let rest = &existing[after_header..];
        let next_section = rest
            .match_indices('\n')
            .find_map(|(i, _)| {
                let line_start = after_header + i + 1;
                let line = existing[line_start..]
                    .split_once('\n')
                    .map_or_else(|| &existing[line_start..], |(line, _)| line);
                if line.trim_start().starts_with('[') {
                    Some(line_start)
                } else {
                    None
                }
            })
            .unwrap_or(existing.len());
        existing.replace_range(start..next_section, block.trim_start());
    } else {
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(block);
    }

    std::fs::write(&path, existing)?;
    Ok(path)
}

/// Cross-platform home-directory lookup. We prefer `HOME` (Unix) and
/// `USERPROFILE` (Windows) directly to avoid pulling in the `dirs` crate
/// for one call site.
fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.is_empty()
    {
        return Some(PathBuf::from(profile));
    }
    None
}

/// Derive the project name from the directory name or a manifest.
fn derive_project_name(root: &Path) -> String {
    // Try Cargo.toml [package] name first
    if let Some(name) = cargo_package_name(root) {
        return name;
    }
    // Try package.json name
    if let Some(name) = npm_package_name(root) {
        return name;
    }
    // Fall back to directory name
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string()
}

/// Extract `[package] name` from `Cargo.toml` if it's a single crate (not just a workspace root).
fn cargo_package_name(root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;
    parsed
        .get("package")?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Extract `name` from `package.json`.
fn npm_package_name(root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("package.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed.get("name")?.as_str().map(String::from)
}

/// Conventional source roots per detected language. Additive only — a
/// directory is contributed only when it actually exists, and the `.`
/// fallback still applies when nothing matched, so a misdetection can
/// never hide real code. Over-inclusion is harmless because the global
/// ignore rules already prune vendored/build trees.
fn conventional_source_dirs(d: &ProjectDetection) -> Vec<&'static str> {
    let mut dirs: Vec<&'static str> = Vec::new();
    let mut add = |xs: &[&'static str]| {
        for x in xs {
            if !dirs.contains(x) {
                dirs.push(x);
            }
        }
    };
    if d.has_rust {
        add(&["src", "crates"]);
    }
    if d.has_go {
        add(&["cmd", "internal", "pkg"]);
    }
    if d.has_java || d.has_kotlin || d.has_scala {
        add(&["src/main/java", "src/main/kotlin", "src/main/scala", "src"]);
    }
    if d.has_csharp {
        add(&["src", "Source"]);
    }
    if d.has_cpp {
        add(&["src", "source", "Source", "lib", "include"]);
    }
    if d.has_swift {
        add(&["Sources", "src"]);
    }
    if d.has_elixir {
        add(&["lib"]);
    }
    if d.has_php {
        add(&["src", "app"]);
    }
    if d.has_ruby {
        add(&["lib", "app"]);
    }
    // Dart/Flutter live under lib/ (has_node==false there; keyed off
    // the dart_tool/pubspec via has_* is not tracked, so include when
    // a Flutter bridge is present).
    if d.bridges.contains(&BridgeKind::FlutterChannel) {
        add(&["lib"]);
    }
    dirs
}

/// Detect source directories based on project layout.
fn detect_source_paths(root: &Path, d: &ProjectDetection) -> Vec<String> {
    let workspaces = &d.workspaces;
    let mut paths = Vec::new();

    // Workspace members first
    for ws in workspaces {
        match ws.kind {
            WorkspaceKind::Cargo => {
                for member in &ws.members {
                    if let Some(rel) = relative_path(root, &member.join("src"))
                        && member.join("src").is_dir()
                    {
                        paths.push(rel);
                    }
                    // Tauri / Electron pattern: Cargo member at `app/src-tauri`
                    // may have a co-located JS/TS frontend at `app/src`.
                    // Also check the member dir itself for a package.json.
                    for check_dir in [member.as_path(), member.parent().unwrap_or(member)] {
                        if check_dir.join("package.json").exists()
                            && let Some(rel) = find_js_source_dir(root, check_dir)
                            && !paths.contains(&rel)
                        {
                            paths.push(rel);
                        }
                    }
                }
            }
            WorkspaceKind::Npm
            | WorkspaceKind::Yarn
            | WorkspaceKind::Pnpm
            | WorkspaceKind::Turborepo
            | WorkspaceKind::Nx => {
                for member in &ws.members {
                    if let Some(rel) = find_js_source_dir(root, member) {
                        paths.push(rel);
                    }
                }
            }
        }
    }

    // If no workspace members contributed paths, check for standalone
    // layouts. JS/TS and Python get precise subdir resolution; every
    // other detected language contributes its conventional roots.
    if paths.is_empty() {
        if d.has_node
            && let Some(rel) = find_js_source_dir(root, root)
            && !paths.contains(&rel)
        {
            paths.push(rel);
        }
        if d.has_python
            && let Some(rel) = find_python_source_dir(root)
            && !paths.contains(&rel)
        {
            paths.push(rel);
        }
        for dir in conventional_source_dirs(d) {
            let candidate = dir.to_string();
            if root.join(dir).is_dir() && !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    // Last resort: index everything (also the safety net whenever the
    // detected roots might be incomplete — a single missed source dir
    // is worse than indexing a bit extra, since global ignore rules
    // already strip the noise).
    if paths.is_empty() {
        paths.push(".".to_string());
    }

    paths
}

/// Find the JS/TS source directory for a package at `pkg_dir`.
fn find_js_source_dir(root: &Path, pkg_dir: &Path) -> Option<String> {
    for dir_name in &["src", "lib", "app"] {
        let candidate = pkg_dir.join(dir_name);
        if candidate.is_dir() {
            return relative_path(root, &candidate);
        }
    }
    None
}

/// Find the Python source directory at `root`.
fn find_python_source_dir(root: &Path) -> Option<String> {
    // PEP 517 src layout
    if root.join("src").is_dir() {
        return Some("src".to_string());
    }
    // Try to find a package dir matching the project name from pyproject.toml
    if let Some(name) = pyproject_name(root) {
        let pkg_dir = name.replace('-', "_");
        if root.join(&pkg_dir).is_dir() {
            return Some(pkg_dir);
        }
    }
    None
}

/// Extract `[project] name` from `pyproject.toml`.
fn pyproject_name(root: &Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;
    parsed
        .get("project")?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Detect documentation files and directories at `root`.
fn detect_doc_paths(root: &Path) -> Vec<String> {
    let candidates = ["README.md", "CHANGELOG.md", "DESIGN.md", "docs", "doc"];
    candidates
        .iter()
        .filter(|p| root.join(p).exists())
        .map(|p| (*p).to_string())
        .collect()
}

/// Build default ignore patterns from the full detection.
///
/// Two kinds of pattern:
/// 1. Per-language test/snapshot globs (unchanged intent, now keyed off
///    every detected language).
/// 2. Project-type-gated build-output *directories*. These names
///    (`bin/`, `obj/`, `Library/`, `Binaries/`, …) are too generic to
///    sit in the global `ALWAYS_IGNORE_DIRS` (they collide with real
///    authored dirs in unrelated projects), but once we know the
///    project is Unity / Unreal / .NET / Xcode they are unambiguous
///    build output. Written into `.ministr.toml` so they apply only to
///    this scoped corpus.
fn default_ignore_patterns(root: &Path, d: &ProjectDetection) -> Vec<String> {
    let mut patterns = Vec::new();

    if d.has_rust {
        patterns.push("*.snap".to_string());
    }
    if d.has_node {
        patterns.extend([
            "*.test.ts".to_string(),
            "*.spec.ts".to_string(),
            "*.test.js".to_string(),
            "*.spec.js".to_string(),
        ]);
    }
    if d.has_python {
        patterns.extend(["*_test.py".to_string(), "test_*.py".to_string()]);
    }
    if d.has_go {
        patterns.push("*_test.go".to_string());
    }

    // .NET: bin/ and obj/ are build output but far too generic to
    // ignore globally.
    if d.has_csharp {
        patterns.extend(["bin/".to_string(), "obj/".to_string()]);
    }

    // Unity: ProjectSettings/ + Assets/ (or any *.unity scene) is the
    // unambiguous signature; Library/Temp/Obj/Logs are pure caches.
    let is_unity = root.join("ProjectSettings").is_dir()
        && (root.join("Assets").is_dir() || dir_has_extension(root, &["unity"]));
    if is_unity {
        patterns.extend([
            "Library/".to_string(),
            "Temp/".to_string(),
            "Obj/".to_string(),
            "Logs/".to_string(),
            "MemoryCaptures/".to_string(),
        ]);
    }

    // Unreal Engine: *.uproject/*.uplugin → Binaries/Intermediate/
    // Saved/DerivedDataCache are all regenerated build artifacts.
    if crate::ingestion::is_unreal_corpus(root) {
        patterns.extend([
            "Binaries/".to_string(),
            "Intermediate/".to_string(),
            "Saved/".to_string(),
            "DerivedDataCache/".to_string(),
        ]);
    }

    // Xcode/Swift: .build (SwiftPM) is build output; DerivedData is
    // already global.
    if d.has_swift {
        patterns.push(".build/".to_string());
    }

    patterns
}

/// Compute a relative path from `base` to `target`, returning `None` if
/// `target` is not under `base`.
///
/// Separators are always normalized to `/` so the emitted paths land in
/// `.ministr.toml` as portable cross-platform strings — a config
/// committed on Windows should round-trip cleanly on macOS / Linux.
/// Windows filesystem APIs accept `/` as a separator, so nothing
/// downstream needs to reverse this normalization.
fn relative_path(base: &Path, target: &Path) -> Option<String> {
    target.strip_prefix(base).ok().map(|rel| {
        let s = rel.to_string_lossy();
        if std::path::MAIN_SEPARATOR == '/' {
            s.into_owned()
        } else {
            s.replace(std::path::MAIN_SEPARATOR, "/")
        }
    })
}

/// Classify the project type from detected signals.
///
/// Priority order (first match wins):
/// 1. **Monorepo** — any workspace with ≥2 members
/// 2. **WebApp** — Tauri bridge, or frontend framework indicators
/// 3. **Api** — HTTP route bridges, or server framework dependencies
/// 4. **Cli** — Rust binary with clap/structopt, or Node bin field
/// 5. **Library** — Rust lib-only crate, or npm package without bin
/// 6. **Unknown** — fallback
fn classify_project_type(
    root: &Path,
    workspaces: &[WorkspaceInfo],
    bridges: &[BridgeKind],
    has_rust: bool,
    has_node: bool,
) -> ProjectType {
    // Monorepo: any workspace with 2+ members.
    if workspaces.iter().any(|w| w.members.len() >= 2) {
        return ProjectType::Monorepo;
    }

    // WebApp: Tauri/Wasm bridges, or frontend indicators.
    if bridges
        .iter()
        .any(|b| matches!(b, BridgeKind::TauriCommand | BridgeKind::WasmBindgen))
    {
        return ProjectType::WebApp;
    }
    if has_node && has_frontend_framework(root) {
        return ProjectType::WebApp;
    }

    // Api: HTTP route bridges, or server framework deps.
    if bridges.iter().any(|b| matches!(b, BridgeKind::HttpRoute)) {
        return ProjectType::Api;
    }
    if has_rust && has_server_dependency(root) {
        return ProjectType::Api;
    }

    // Cli: binary crate with CLI deps, or Node bin.
    if has_rust && is_rust_cli(root) {
        return ProjectType::Cli;
    }
    if has_node && has_node_bin(root) {
        return ProjectType::Cli;
    }

    // Library: lib-only Rust crate, or npm without bin.
    if has_rust && is_rust_lib_only(root) {
        return ProjectType::Library;
    }

    ProjectType::Unknown
}

/// Check if `package.json` references a frontend framework.
fn has_frontend_framework(root: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(root.join("package.json")) else {
        return false;
    };
    // Quick substring checks — avoids pulling in a JSON parser for a heuristic.
    [
        "\"react\"",
        "\"vue\"",
        "\"svelte\"",
        "\"next\"",
        "\"nuxt\"",
        "\"angular\"",
        "\"astro\"",
    ]
    .iter()
    .any(|fw| content.contains(fw))
}

/// Check if `Cargo.toml` has server-framework dependencies.
fn has_server_dependency(root: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return false;
    };
    [
        "actix-web",
        "axum",
        "rocket",
        "warp",
        "tide",
        "poem",
        "salvo",
    ]
    .iter()
    .any(|dep| content.contains(dep))
}

/// Check if the Rust project is a CLI (has binary targets + CLI deps).
fn is_rust_cli(root: &Path) -> bool {
    let has_main = root.join("src/main.rs").exists();
    if !has_main {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return false;
    };
    ["clap", "structopt", "argh", "bpaf"]
        .iter()
        .any(|dep| content.contains(dep))
}

/// Check if the Rust project is lib-only (no binary targets).
fn is_rust_lib_only(root: &Path) -> bool {
    let has_lib = root.join("src/lib.rs").exists();
    let has_main = root.join("src/main.rs").exists();
    has_lib && !has_main
}

/// Check if `package.json` has a `"bin"` field.
fn has_node_bin(root: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(root.join("package.json")) else {
        return false;
    };
    content.contains("\"bin\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn generated_config_ships_the_code_corpus_sparse_default_visibly() {
        // W5: a detected CODE project gets the measured hybrid default as an
        // ACTIVE, visible line in the generated file (the user reviews it
        // before the first index); a project with no code gets guidance as a
        // comment only — never a silent behavior change.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();

        let toml = render_toml(&detect_project(root));
        assert!(
            toml.contains("\nsparse_weight = 0.6\n"),
            "code project: active sparse_weight line, got:\n{toml}"
        );

        let empty = TempDir::new().unwrap();
        fs::write(empty.path().join("README.md"), "# docs only\n").unwrap();
        let toml = render_toml(&detect_project(empty.path()));
        assert!(
            !toml.contains("\nsparse_weight = 0.6\n"),
            "docs-only project must NOT enable sparse, got:\n{toml}"
        );
        assert!(
            toml.contains("# sparse_weight = 0.6"),
            "docs-only project still carries the commented guidance, got:\n{toml}"
        );
    }

    #[test]
    fn test_detect_cargo_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crate-a", "crate-b"]
"#,
        )
        .unwrap();

        fs::create_dir_all(root.join("crate-a/src")).unwrap();
        fs::create_dir_all(root.join("crate-b/src")).unwrap();

        let detection = detect_project(root);

        assert!(detection.has_rust);
        assert_eq!(detection.workspaces.len(), 1);
        assert_eq!(detection.workspaces[0].kind, WorkspaceKind::Cargo);
        assert!(detection.source_paths.contains(&"crate-a/src".to_string()));
        assert!(detection.source_paths.contains(&"crate-b/src".to_string()));
    }

    #[test]
    fn test_detect_single_rust_crate() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"
[package]
name = "my-crate"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();

        let detection = detect_project(root);

        assert!(detection.has_rust);
        assert_eq!(detection.project_name, "my-crate");
        assert!(detection.workspaces.is_empty());
        assert_eq!(detection.source_paths, vec!["src"]);
    }

    #[test]
    fn test_detect_node_monorepo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("package.json"),
            r#"{"name": "my-monorepo", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("packages/app-a/src")).unwrap();
        fs::create_dir_all(root.join("packages/app-b/lib")).unwrap();

        let detection = detect_project(root);

        assert!(detection.has_node);
        assert!(!detection.workspaces.is_empty());
        assert!(
            detection
                .source_paths
                .contains(&"packages/app-a/src".to_string())
        );
        assert!(
            detection
                .source_paths
                .contains(&"packages/app-b/lib".to_string())
        );
    }

    #[test]
    fn test_detect_python_project() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("pyproject.toml"),
            r#"
[project]
name = "my-lib"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();

        let detection = detect_project(root);

        assert!(detection.has_python);
        assert_eq!(detection.source_paths, vec!["src"]);
    }

    #[test]
    fn test_detect_polyglot() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"
[package]
name = "polyglot"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::write(root.join("package.json"), r#"{"name": "polyglot"}"#).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();

        let detection = detect_project(root);

        assert!(detection.has_rust);
        assert!(detection.has_node);
    }

    #[test]
    fn test_detect_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let detection = detect_project(tmp.path());

        assert!(!detection.has_rust);
        assert!(!detection.has_node);
        assert!(!detection.has_python);
        assert_eq!(detection.source_paths, vec!["."]);
    }

    #[test]
    fn test_render_toml_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"
[package]
name = "roundtrip"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("README.md"), "# Hello").unwrap();

        let detection = detect_project(root);
        let toml_str = render_toml(&detection);

        // Parse back as RepoConfig — should not error
        let parsed: crate::config::RepoConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.corpus.paths.contains(&"src".to_string()));
        assert!(parsed.corpus.paths.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_skips_overwrite_but_writes_mcp() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join(CORPUS_CONFIG_FILENAME), "existing").unwrap();

        // Should succeed (writes MCP configs) but not overwrite .ministr.toml.
        let result = write_config(root, false);
        assert!(result.is_ok());
        assert_eq!(
            fs::read_to_string(root.join(CORPUS_CONFIG_FILENAME)).unwrap(),
            "existing"
        );
        // MCP configs should exist.
        assert!(root.join(".mcp.json").exists());
        assert!(root.join(".vscode/mcp.json").exists());
        assert!(root.join(".cursor/mcp.json").exists());
    }

    #[test]
    fn test_force_overwrites() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join(CORPUS_CONFIG_FILENAME), "existing").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "test"
version = "0.1.0"
"#,
        )
        .unwrap();

        let result = write_config(root, true);
        assert!(result.is_ok());

        let content = fs::read_to_string(root.join(CORPUS_CONFIG_FILENAME)).unwrap();
        assert!(content.contains("[corpus]"));
    }

    #[test]
    fn test_doc_paths_detected() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join("README.md"), "# Hello").unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();

        let detection = detect_project(root);

        assert!(detection.doc_paths.contains(&"README.md".to_string()));
        assert!(detection.doc_paths.contains(&"docs".to_string()));
    }

    #[test]
    fn test_ignore_patterns_per_language() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        fs::write(root.join("package.json"), r#"{"name":"x"}"#).unwrap();
        let p = detect_project(root).ignore_patterns;
        assert!(p.contains(&"*.snap".to_string()));
        assert!(p.contains(&"*.test.ts".to_string()));
        assert!(!p.iter().any(|x| x.contains("_test.py")));

        let tmp2 = TempDir::new().unwrap();
        fs::write(
            tmp2.path().join("pyproject.toml"),
            "[project]\nname=\"x\"\n",
        )
        .unwrap();
        let py = detect_project(tmp2.path()).ignore_patterns;
        assert!(py.contains(&"*_test.py".to_string()));
    }

    #[test]
    fn dotnet_project_gates_bin_obj_ignores() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("App.csproj"), "<Project/>").unwrap();
        let p = detect_project(tmp.path()).ignore_patterns;
        assert!(p.contains(&"bin/".to_string()));
        assert!(p.contains(&"obj/".to_string()));
    }

    #[test]
    fn unity_project_gates_library_temp_ignores() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("ProjectSettings")).unwrap();
        fs::create_dir_all(root.join("Assets")).unwrap();
        let p = detect_project(root).ignore_patterns;
        assert!(p.contains(&"Library/".to_string()), "got {p:?}");
        assert!(p.contains(&"Temp/".to_string()));
    }

    #[test]
    fn unreal_project_gates_binaries_intermediate_ignores() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("MyGame.uproject"), "{}").unwrap();
        let p = detect_project(tmp.path()).ignore_patterns;
        assert!(p.contains(&"Binaries/".to_string()), "got {p:?}");
        assert!(p.contains(&"Intermediate/".to_string()));
        assert!(p.contains(&"Saved/".to_string()));
    }

    #[test]
    fn polyglot_root_classified_as_monorepo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Three independent ecosystems, no workspace manifest.
        fs::write(root.join("go.mod"), "module x\n").unwrap();
        fs::write(root.join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        fs::write(root.join("composer.json"), r#"{"name":"x/y"}"#).unwrap();
        assert_eq!(detect_project(root).project_type, ProjectType::Monorepo);
    }

    // -----------------------------------------------------------------------
    // Project type classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn classify_monorepo_from_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("a/src")).unwrap();
        fs::create_dir_all(root.join("b/src")).unwrap();

        let detection = detect_project(root);
        assert_eq!(detection.project_type, ProjectType::Monorepo);
    }

    #[test]
    fn classify_library_from_lib_only() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(root.join("Cargo.toml"), "[package]\nname = \"mylib\"\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn hello() {}").unwrap();

        let detection = detect_project(root);
        assert_eq!(detection.project_type, ProjectType::Library);
    }

    #[test]
    fn classify_cli_from_clap_dep() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"mytool\"\n\n[dependencies]\nclap = \"4\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let detection = detect_project(root);
        assert_eq!(detection.project_type, ProjectType::Cli);
    }

    #[test]
    fn classify_api_from_axum_dep() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"myapi\"\n\n[dependencies]\naxum = \"0.7\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let detection = detect_project(root);
        assert_eq!(detection.project_type, ProjectType::Api);
    }

    #[test]
    fn classify_webapp_from_react() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        fs::write(
            root.join("package.json"),
            r#"{"name": "myapp", "dependencies": {"react": "^18"}}"#,
        )
        .unwrap();

        let detection = detect_project(root);
        assert_eq!(detection.project_type, ProjectType::WebApp);
    }

    #[test]
    fn classify_unknown_for_empty() {
        let tmp = TempDir::new().unwrap();
        let detection = detect_project(tmp.path());
        assert_eq!(detection.project_type, ProjectType::Unknown);
    }

    #[test]
    fn detect_tauri_colocated_frontend() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Cargo workspace with a Tauri member at app/src-tauri
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app/src-tauri\"]\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("app/src-tauri/src")).unwrap();
        fs::write(
            root.join("app/src-tauri/Cargo.toml"),
            "[package]\nname = \"my-app\"\n",
        )
        .unwrap();

        // Frontend lives at app/src with a package.json at app/
        fs::create_dir_all(root.join("app/src")).unwrap();
        fs::write(root.join("app/package.json"), r#"{"name": "my-app"}"#).unwrap();

        let detection = detect_project(root);

        assert!(
            detection
                .source_paths
                .contains(&"app/src-tauri/src".to_string()),
            "should find Rust source: {:?}",
            detection.source_paths
        );
        assert!(
            detection.source_paths.contains(&"app/src".to_string()),
            "should find co-located frontend: {:?}",
            detection.source_paths
        );
    }

    #[test]
    fn detects_expanded_languages() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("App.csproj"), "<Project/>").unwrap();
        fs::write(root.join("build.gradle.kts"), "").unwrap();
        fs::write(root.join("Package.swift"), "// swift-tools-version:5.9").unwrap();
        fs::write(root.join("build.sbt"), "").unwrap();
        fs::write(
            root.join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.20)",
        )
        .unwrap();
        fs::write(root.join("mix.exs"), "defmodule M do\nend").unwrap();

        let d = detect_project(root);
        assert!(d.has_csharp);
        assert!(d.has_kotlin);
        assert!(d.has_swift);
        assert!(d.has_scala);
        assert!(d.has_cpp);
        assert!(d.has_elixir);

        let langs = d.detected_languages();
        for l in [
            Language::Csharp,
            Language::Kotlin,
            Language::Swift,
            Language::Scala,
            Language::Cpp,
            Language::Elixir,
        ] {
            assert!(langs.contains(&l), "missing {l:?} in {langs:?}");
        }
    }

    #[test]
    fn node_without_tsconfig_is_javascript() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        let d = detect_project(tmp.path());
        assert!(d.has_javascript);
        assert!(d.detected_languages().contains(&Language::JavaScript));
    }
}
