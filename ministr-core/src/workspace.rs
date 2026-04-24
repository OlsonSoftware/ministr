//! Workspace detection for monorepo and multi-package project layouts.
//!
//! Detects Cargo workspaces, npm/yarn workspaces, pnpm workspaces,
//! Turborepo, and Nx monorepo layouts from a project root directory.
//! Returns structured [`WorkspaceInfo`] with resolved member directories.
//!
//! # Examples
//!
//! ```no_run
//! use ministr_core::workspace::detect_workspace;
//! use std::path::Path;
//!
//! if let Some(ws) = detect_workspace(Path::new("/path/to/project")) {
//!     println!("Found {:?} workspace with {} members", ws.kind, ws.members.len());
//! }
//! ```

use std::path::{Path, PathBuf};

/// The kind of workspace or monorepo layout detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceKind {
    /// Rust/Cargo workspace (`Cargo.toml` with `[workspace]` section).
    Cargo,
    /// npm workspace (`package.json` with `"workspaces"` field).
    Npm,
    /// Yarn workspace (`package.json` with `"workspaces"` field).
    ///
    /// Distinguished from npm by the presence of `yarn.lock`.
    Yarn,
    /// pnpm workspace (`pnpm-workspace.yaml`).
    Pnpm,
    /// Turborepo monorepo (`turbo.json` at root).
    ///
    /// Member discovery delegates to the underlying package manager workspace.
    Turborepo,
    /// Nx monorepo (`nx.json` at root).
    Nx,
}

impl std::fmt::Display for WorkspaceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cargo => write!(f, "Cargo"),
            Self::Npm => write!(f, "npm"),
            Self::Yarn => write!(f, "Yarn"),
            Self::Pnpm => write!(f, "pnpm"),
            Self::Turborepo => write!(f, "Turborepo"),
            Self::Nx => write!(f, "Nx"),
        }
    }
}

/// Information about a detected workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceInfo {
    /// The type of workspace detected.
    pub kind: WorkspaceKind,
    /// The root directory of the workspace.
    pub root: PathBuf,
    /// Resolved member directories (expanded from glob patterns).
    pub members: Vec<PathBuf>,
}

/// Detect the primary workspace layout at `root`.
///
/// Returns the first detected workspace type, checked in priority order:
/// Cargo → pnpm → npm/Yarn → Turborepo → Nx.
///
/// For projects with multiple overlapping layouts (e.g., pnpm + Turborepo),
/// use [`detect_all_workspaces`] instead.
#[must_use]
pub fn detect_workspace(root: &Path) -> Option<WorkspaceInfo> {
    detect_all_workspaces(root).into_iter().next()
}

/// Detect all workspace layouts present at `root`.
///
/// A project can have multiple overlapping workspace types — for example,
/// a pnpm workspace that also uses Turborepo for task orchestration.
/// This function returns all detected layouts.
#[must_use]
pub fn detect_all_workspaces(root: &Path) -> Vec<WorkspaceInfo> {
    let mut workspaces = Vec::new();

    if let Some(ws) = detect_cargo(root) {
        workspaces.push(ws);
    }

    if let Some(ws) = detect_pnpm(root) {
        workspaces.push(ws);
    }

    // npm/yarn detection (mutually exclusive — yarn takes priority if yarn.lock exists)
    if let Some(ws) = detect_npm_yarn(root) {
        workspaces.push(ws);
    }

    if let Some(ws) = detect_turborepo(root) {
        workspaces.push(ws);
    }

    if let Some(ws) = detect_nx(root) {
        workspaces.push(ws);
    }

    workspaces
}

/// Detect a Cargo workspace from `Cargo.toml` with a `[workspace]` section.
fn detect_cargo(root: &Path) -> Option<WorkspaceInfo> {
    let cargo_toml = root.join("Cargo.toml");
    let content = std::fs::read_to_string(&cargo_toml).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;

    let workspace = parsed.get("workspace")?;
    let members = workspace.get("members")?.as_array()?;

    let patterns: Vec<&str> = members.iter().filter_map(toml::Value::as_str).collect();

    let resolved = expand_glob_patterns(root, &patterns);

    Some(WorkspaceInfo {
        kind: WorkspaceKind::Cargo,
        root: root.to_path_buf(),
        members: resolved,
    })
}

/// Detect a pnpm workspace from `pnpm-workspace.yaml`.
///
/// Parses the simple YAML format without a full YAML parser dependency:
/// ```yaml
/// packages:
///   - 'packages/*'
///   - 'apps/*'
/// ```
fn detect_pnpm(root: &Path) -> Option<WorkspaceInfo> {
    let yaml_path = root.join("pnpm-workspace.yaml");
    let content = std::fs::read_to_string(&yaml_path).ok()?;

    let patterns = parse_pnpm_workspace_yaml(&content);
    if patterns.is_empty() {
        return None;
    }

    let pattern_refs: Vec<&str> = patterns.iter().map(String::as_str).collect();
    let resolved = expand_glob_patterns(root, &pattern_refs);

    Some(WorkspaceInfo {
        kind: WorkspaceKind::Pnpm,
        root: root.to_path_buf(),
        members: resolved,
    })
}

/// Parse the `packages` list from a `pnpm-workspace.yaml` string.
///
/// This is a minimal parser that handles the standard format without
/// requiring a full YAML dependency.
fn parse_pnpm_workspace_yaml(content: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "packages:" {
            in_packages = true;
            continue;
        }

        if in_packages {
            if let Some(item) = trimmed.strip_prefix("- ") {
                // Strip surrounding quotes (single or double)
                let item = item.trim();
                let item = item
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .or_else(|| item.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
                    .unwrap_or(item);
                patterns.push(item.to_owned());
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // We've left the packages list (hit another top-level key)
                break;
            }
        }
    }

    patterns
}

/// Detect npm or Yarn workspace from `package.json` with a `"workspaces"` field.
///
/// Yarn is distinguished from npm by the presence of `yarn.lock`.
fn detect_npm_yarn(root: &Path) -> Option<WorkspaceInfo> {
    let pkg_json = root.join("package.json");
    let content = std::fs::read_to_string(&pkg_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    let workspaces = parsed.get("workspaces")?;

    // workspaces can be an array or an object with a "packages" key (Yarn classic)
    let patterns: Vec<&str> = match workspaces {
        serde_json::Value::Array(arr) => arr.iter().filter_map(serde_json::Value::as_str).collect(),
        serde_json::Value::Object(obj) => obj
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .map(|arr| arr.iter().filter_map(serde_json::Value::as_str).collect())
            .unwrap_or_default(),
        _ => return None,
    };

    if patterns.is_empty() {
        return None;
    }

    let resolved = expand_glob_patterns(root, &patterns);
    let kind = if root.join("yarn.lock").exists() {
        WorkspaceKind::Yarn
    } else {
        WorkspaceKind::Npm
    };

    Some(WorkspaceInfo {
        kind,
        root: root.to_path_buf(),
        members: resolved,
    })
}

/// Detect Turborepo from `turbo.json` at root.
///
/// Turborepo delegates member discovery to the underlying package manager
/// workspace, so members are resolved from npm/pnpm/yarn workspace config.
fn detect_turborepo(root: &Path) -> Option<WorkspaceInfo> {
    let turbo_json = root.join("turbo.json");
    if !turbo_json.exists() {
        return None;
    }

    // Members come from the underlying package manager workspace
    let members = resolve_js_workspace_members(root);

    Some(WorkspaceInfo {
        kind: WorkspaceKind::Turborepo,
        root: root.to_path_buf(),
        members,
    })
}

/// Detect Nx monorepo from `nx.json` at root.
///
/// Members are resolved from `workspace.json` (if present) or by scanning
/// for directories containing `project.json`.
fn detect_nx(root: &Path) -> Option<WorkspaceInfo> {
    let nx_json = root.join("nx.json");
    if !nx_json.exists() {
        return None;
    }

    let members = resolve_nx_members(root);

    Some(WorkspaceInfo {
        kind: WorkspaceKind::Nx,
        root: root.to_path_buf(),
        members,
    })
}

/// Resolve Nx workspace members from `workspace.json` or `project.json` files.
fn resolve_nx_members(root: &Path) -> Vec<PathBuf> {
    // Try workspace.json first
    if let Some(members) = resolve_nx_from_workspace_json(root) {
        return members;
    }

    // Fall back to scanning for project.json files
    resolve_nx_from_project_json_scan(root)
}

/// Parse `workspace.json` to find Nx project directories.
fn resolve_nx_from_workspace_json(root: &Path) -> Option<Vec<PathBuf>> {
    let ws_json = root.join("workspace.json");
    let content = std::fs::read_to_string(&ws_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;

    let projects = parsed.get("projects")?;

    let mut members = Vec::new();
    match projects {
        serde_json::Value::Object(map) => {
            for value in map.values() {
                // Value can be a string (path) or an object with a "root" field
                let dir = match value {
                    serde_json::Value::String(s) => Some(s.as_str()),
                    serde_json::Value::Object(obj) => {
                        obj.get("root").and_then(serde_json::Value::as_str)
                    }
                    _ => None,
                };
                if let Some(dir) = dir {
                    let path = root.join(dir);
                    if path.is_dir() {
                        members.push(path);
                    }
                }
            }
        }
        _ => return None,
    }

    members.sort();
    members.dedup();
    Some(members)
}

/// Scan for directories containing `project.json` (Nx convention).
fn resolve_nx_from_project_json_scan(root: &Path) -> Vec<PathBuf> {
    let mut members = Vec::new();

    // Search common Nx directory patterns
    for dir_name in &["apps", "libs", "packages"] {
        let dir = root.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("project.json").exists() {
                    members.push(path);
                }
            }
        }
    }

    members.sort();
    members
}

/// Resolve JavaScript workspace members from pnpm, npm, or yarn configuration.
fn resolve_js_workspace_members(root: &Path) -> Vec<PathBuf> {
    // Try pnpm first
    if let Some(ws) = detect_pnpm(root) {
        return ws.members;
    }

    // Then npm/yarn
    if let Some(ws) = detect_npm_yarn(root) {
        return ws.members;
    }

    Vec::new()
}

/// Expand glob patterns relative to a root directory into resolved directory paths.
///
/// Each pattern is joined with `root` and expanded. Only existing directories
/// are included in the result.
fn expand_glob_patterns(root: &Path, patterns: &[&str]) -> Vec<PathBuf> {
    let mut resolved = Vec::new();

    for pattern in patterns {
        // Skip negation patterns (e.g., "!packages/internal-*")
        if pattern.starts_with('!') {
            continue;
        }

        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        if let Ok(entries) = glob::glob(&pattern_str) {
            for entry in entries.flatten() {
                if entry.is_dir() {
                    resolved.push(entry);
                }
            }
        } else {
            // If the pattern is not a glob, treat it as a literal path
            let literal = root.join(pattern);
            if literal.is_dir() {
                resolved.push(literal);
            }
        }
    }

    resolved.sort();
    resolved.dedup();
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a directory structure under `root`.
    fn create_dirs(root: &Path, dirs: &[&str]) {
        for dir in dirs {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    // ── Cargo workspace tests ────────────────────────────────────────

    #[test]
    fn detect_cargo_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(
            root,
            &["ministr-core/src", "ministr-mcp/src", "ministr-cli/src"],
        );

        std::fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["ministr-core", "ministr-mcp", "ministr-cli"]
resolver = "2"
"#,
        )
        .unwrap();

        // Each member needs a Cargo.toml for it to be a valid directory
        for member in &["ministr-core", "ministr-mcp", "ministr-cli"] {
            std::fs::write(
                root.join(member).join("Cargo.toml"),
                format!("[package]\nname = \"{member}\""),
            )
            .unwrap();
        }

        let ws = detect_cargo(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Cargo);
        assert_eq!(ws.members.len(), 3);
        assert!(ws.members.contains(&root.join("ministr-core")));
        assert!(ws.members.contains(&root.join("ministr-mcp")));
        assert!(ws.members.contains(&root.join("ministr-cli")));
    }

    #[test]
    fn detect_cargo_workspace_with_globs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["crates/foo/src", "crates/bar/src"]);

        std::fs::write(
            root.join("Cargo.toml"),
            r#"
[workspace]
members = ["crates/*"]
"#,
        )
        .unwrap();

        let ws = detect_cargo(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Cargo);
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn no_cargo_workspace_without_workspace_section() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"solo\"\nversion = \"0.1.0\"",
        )
        .unwrap();

        assert!(detect_cargo(root).is_none());
    }

    // ── pnpm workspace tests ────────────────────────────────────────

    #[test]
    fn detect_pnpm_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/ui", "packages/api", "apps/web"]);

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'packages/*'\n  - 'apps/*'\n",
        )
        .unwrap();

        let ws = detect_pnpm(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Pnpm);
        assert_eq!(ws.members.len(), 3);
    }

    #[test]
    fn parse_pnpm_yaml_unquoted() {
        let yaml = "packages:\n  - packages/*\n  - apps/*\n";
        let patterns = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(patterns, vec!["packages/*", "apps/*"]);
    }

    #[test]
    fn parse_pnpm_yaml_double_quoted() {
        let yaml = "packages:\n  - \"packages/*\"\n  - \"apps/*\"\n";
        let patterns = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(patterns, vec!["packages/*", "apps/*"]);
    }

    #[test]
    fn parse_pnpm_yaml_stops_at_next_key() {
        let yaml = "packages:\n  - packages/*\ncatalog:\n  react: ^18\n";
        let patterns = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(patterns, vec!["packages/*"]);
    }

    // ── npm/yarn workspace tests ────────────────────────────────────

    #[test]
    fn detect_npm_workspace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/core", "packages/cli"]);

        std::fs::write(
            root.join("package.json"),
            r#"{"name": "mono", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();

        let ws = detect_npm_yarn(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Npm);
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn detect_yarn_workspace_with_lock() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/web"]);

        std::fs::write(
            root.join("package.json"),
            r#"{"name": "mono", "workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        std::fs::write(root.join("yarn.lock"), "").unwrap();

        let ws = detect_npm_yarn(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Yarn);
    }

    #[test]
    fn detect_yarn_classic_workspaces_object() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/lib"]);

        std::fs::write(
            root.join("package.json"),
            r#"{"name": "mono", "workspaces": {"packages": ["packages/*"]}}"#,
        )
        .unwrap();
        std::fs::write(root.join("yarn.lock"), "").unwrap();

        let ws = detect_npm_yarn(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Yarn);
        assert_eq!(ws.members.len(), 1);
    }

    #[test]
    fn no_npm_workspace_without_workspaces_field() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(
            root.join("package.json"),
            r#"{"name": "solo", "version": "1.0.0"}"#,
        )
        .unwrap();

        assert!(detect_npm_yarn(root).is_none());
    }

    // ── Turborepo tests ─────────────────────────────────────────────

    #[test]
    fn detect_turborepo_with_pnpm() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/ui", "apps/web"]);

        std::fs::write(root.join("turbo.json"), r#"{"pipeline": {}}"#).unwrap();
        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'packages/*'\n  - 'apps/*'\n",
        )
        .unwrap();

        let ws = detect_turborepo(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Turborepo);
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn no_turborepo_without_turbo_json() {
        let tmp = TempDir::new().unwrap();
        assert!(detect_turborepo(tmp.path()).is_none());
    }

    // ── Nx tests ────────────────────────────────────────────────────

    #[test]
    fn detect_nx_with_workspace_json() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["apps/frontend", "libs/shared"]);

        std::fs::write(root.join("nx.json"), "{}").unwrap();
        std::fs::write(
            root.join("workspace.json"),
            r#"{"projects": {"frontend": "apps/frontend", "shared": "libs/shared"}}"#,
        )
        .unwrap();

        let ws = detect_nx(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Nx);
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn detect_nx_with_project_json_scan() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["apps/web", "libs/ui"]);

        std::fs::write(root.join("nx.json"), "{}").unwrap();
        std::fs::write(root.join("apps/web/project.json"), "{}").unwrap();
        std::fs::write(root.join("libs/ui/project.json"), "{}").unwrap();

        let ws = detect_nx(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Nx);
        assert_eq!(ws.members.len(), 2);
    }

    #[test]
    fn no_nx_without_nx_json() {
        let tmp = TempDir::new().unwrap();
        assert!(detect_nx(tmp.path()).is_none());
    }

    // ── detect_all_workspaces tests ─────────────────────────────────

    #[test]
    fn detect_overlapping_pnpm_and_turborepo() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/ui"]);

        std::fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - 'packages/*'\n",
        )
        .unwrap();
        std::fs::write(root.join("turbo.json"), "{}").unwrap();

        let all = detect_all_workspaces(root);
        let kinds: Vec<WorkspaceKind> = all.iter().map(|ws| ws.kind).collect();
        assert!(kinds.contains(&WorkspaceKind::Pnpm));
        assert!(kinds.contains(&WorkspaceKind::Turborepo));
    }

    #[test]
    fn detect_workspace_returns_first() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["crates/core"]);

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        let ws = detect_workspace(root).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Cargo);
    }

    #[test]
    fn detect_workspace_returns_none_for_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(detect_workspace(tmp.path()).is_none());
    }

    // ── expand_glob_patterns tests ──────────────────────────────────

    #[test]
    fn expand_glob_skips_negation_patterns() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["packages/core", "packages/internal-x"]);

        let resolved = expand_glob_patterns(root, &["packages/*", "!packages/internal-*"]);
        // Negation patterns are skipped, so both dirs appear
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn expand_glob_literal_path() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        create_dirs(root, &["my-app"]);

        let resolved = expand_glob_patterns(root, &["my-app"]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], root.join("my-app"));
    }

    // ── WorkspaceKind Display tests ─────────────────────────────────

    #[test]
    fn workspace_kind_display() {
        assert_eq!(WorkspaceKind::Cargo.to_string(), "Cargo");
        assert_eq!(WorkspaceKind::Npm.to_string(), "npm");
        assert_eq!(WorkspaceKind::Yarn.to_string(), "Yarn");
        assert_eq!(WorkspaceKind::Pnpm.to_string(), "pnpm");
        assert_eq!(WorkspaceKind::Turborepo.to_string(), "Turborepo");
        assert_eq!(WorkspaceKind::Nx.to_string(), "Nx");
    }
}
