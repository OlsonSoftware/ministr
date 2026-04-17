//! Project initialization and `.iris.toml` generation.
//!
//! Auto-detects project structure by scanning manifests (`Cargo.toml`,
//! `package.json`, `pyproject.toml`) and workspace layouts, then generates
//! a sensible `.iris.toml` configuration file with commented sections.
//!
//! # Examples
//!
//! ```no_run
//! use iris_core::init::write_config;
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
    /// Relative paths to source directories.
    pub source_paths: Vec<String>,
    /// Relative paths to documentation files/directories.
    pub doc_paths: Vec<String>,
    /// Suggested ignore patterns for `.iris.toml`.
    pub ignore_patterns: Vec<String>,
}

/// Primary language detected in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
    Java,
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
        if self.has_python {
            langs.push(Language::Python);
        }
        if self.has_go {
            langs.push(Language::Go);
        }
        if self.has_java {
            langs.push(Language::Java);
        }
        langs
    }
}

/// Errors that can occur during `iris init`.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The config file already exists and `--force` was not specified.
    #[error(".iris.toml already exists at {}", path.display())]
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

    let project_name = derive_project_name(root);
    let source_paths = detect_source_paths(root, &workspaces, has_rust, has_node, has_python);
    let doc_paths = detect_doc_paths(root);
    let ignore_patterns = default_ignore_patterns(has_rust, has_node, has_python);
    let project_type = classify_project_type(root, &workspaces, &bridges, has_rust, has_node);

    ProjectDetection {
        project_name,
        project_type,
        workspaces,
        bridges,
        has_rust,
        has_node,
        has_python,
        has_go,
        has_java,
        source_paths,
        doc_paths,
        ignore_patterns,
    }
}

/// Generate a commented TOML string from a [`ProjectDetection`].
///
/// The output includes inline comments explaining each section and is
/// suitable for writing directly to `.iris.toml`.
#[must_use]
#[allow(clippy::too_many_lines)] // template rendering — splitting would scatter the template
pub fn render_toml(detection: &ProjectDetection) -> String {
    let mut out = String::new();

    // Header comment
    let _ = writeln!(
        out,
        "# iris corpus configuration — generated by `iris init`"
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
        "# iris automatically ignores target/, node_modules/, __pycache__, .git/, etc."
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

/// Write `.iris.toml` to `root`, failing if it already exists (unless `force`).
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
        // .iris.toml already exists — skip it, but still write MCP configs.
        write_mcp_configs(root)?;
        return Ok(detection);
    }

    let toml_str = render_toml(&detection);
    std::fs::write(&config_path, toml_str)?;

    // Write MCP client configs (Claude Code + Copilot).
    write_mcp_configs(root)?;

    Ok(detection)
}

/// Write MCP client configuration files for Claude Code and GitHub Copilot.
///
/// Creates `.mcp.json` (Claude Code) and `.vscode/mcp.json` (Copilot) if
/// they don't already contain an iris entry. Existing files are merged
/// non-destructively — only the `iris` key is added.
///
/// # Errors
///
/// Returns [`InitError::Io`] on filesystem errors.
pub fn write_mcp_configs(root: &Path) -> Result<(), InitError> {
    // Claude Code: .mcp.json
    write_mcp_json(root, ".mcp.json")?;

    // GitHub Copilot / VS Code: .vscode/mcp.json
    let vscode_dir = root.join(".vscode");
    if !vscode_dir.exists() {
        std::fs::create_dir_all(&vscode_dir)?;
    }
    write_mcp_json(root, ".vscode/mcp.json")?;

    // Cursor: .cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    if !cursor_dir.exists() {
        std::fs::create_dir_all(&cursor_dir)?;
    }
    write_mcp_json(root, ".cursor/mcp.json")?;

    Ok(())
}

/// Write or merge an iris entry into an MCP JSON config file.
fn write_mcp_json(root: &Path, relative_path: &str) -> Result<(), InitError> {
    let path = root.join(relative_path);

    let iris_entry = serde_json::json!({
        "command": "iris",
        "args": ["serve", "--transport", "stdio"]
    });

    let mut config: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Always update the iris entry to ensure correct args.
    let servers = config.as_object_mut().and_then(|o| {
        o.entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
    });

    if let Some(servers) = servers {
        servers.insert("iris".to_string(), iris_entry);
        let json_str = serde_json::to_string_pretty(&config).unwrap_or_default();
        std::fs::write(&path, format!("{json_str}\n"))?;
    }

    Ok(())
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

/// Detect source directories based on project layout.
fn detect_source_paths(
    root: &Path,
    workspaces: &[WorkspaceInfo],
    has_rust: bool,
    has_node: bool,
    has_python: bool,
) -> Vec<String> {
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

    // If no workspace members contributed paths, check for standalone layouts
    if paths.is_empty() {
        if has_rust && root.join("src").is_dir() {
            paths.push("src".to_string());
        }
        if has_node
            && let Some(rel) = find_js_source_dir(root, root)
            && !paths.contains(&rel)
        {
            paths.push(rel);
        }
        if has_python
            && let Some(rel) = find_python_source_dir(root)
            && !paths.contains(&rel)
        {
            paths.push(rel);
        }
    }

    // Last resort: index everything
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

/// Build default ignore patterns based on detected languages.
fn default_ignore_patterns(has_rust: bool, has_node: bool, has_python: bool) -> Vec<String> {
    let mut patterns = Vec::new();

    if has_rust {
        patterns.push("*.snap".to_string());
    }
    if has_node {
        patterns.extend([
            "*.test.ts".to_string(),
            "*.spec.ts".to_string(),
            "*.test.js".to_string(),
            "*.spec.js".to_string(),
        ]);
    }
    if has_python {
        patterns.extend(["*_test.py".to_string(), "test_*.py".to_string()]);
    }

    patterns
}

/// Compute a relative path from `base` to `target`, returning `None` if
/// `target` is not under `base`.
fn relative_path(base: &Path, target: &Path) -> Option<String> {
    target
        .strip_prefix(base)
        .ok()
        .map(|rel| rel.to_string_lossy().to_string())
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

        // Should succeed (writes MCP configs) but not overwrite .iris.toml.
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
        let patterns = default_ignore_patterns(true, true, false);
        assert!(patterns.contains(&"*.snap".to_string()));
        assert!(patterns.contains(&"*.test.ts".to_string()));
        assert!(!patterns.iter().any(|p| p.contains("_test.py")));

        let py_patterns = default_ignore_patterns(false, false, true);
        assert!(py_patterns.contains(&"*_test.py".to_string()));
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
}
