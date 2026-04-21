//! Cross-package import graph for workspace-aware reference resolution.
//!
//! Maps crate/package names to workspace member directories and tracks
//! cross-package dependency edges derived from symbol references.
//!
//! # Examples
//!
//! ```no_run
//! use ministr_core::code::package_graph::PackageGraph;
//! use std::path::Path;
//!
//! let graph = PackageGraph::from_cargo_workspace(
//!     Path::new("/repo"),
//!     &[Path::new("/repo/ministr-core").to_path_buf()],
//! );
//! assert_eq!(graph.dir_prefix_for_crate("ministr_core"), Some("ministr-core"));
//! ```

use std::path::{Path, PathBuf};

/// Metadata for a single package/crate within a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    /// The package name as declared in `Cargo.toml` (e.g., `"ministr-core"`).
    pub name: String,
    /// The crate name used in Rust source (hyphens replaced with underscores, e.g., `"ministr_core"`).
    pub crate_name: String,
    /// Directory prefix relative to the workspace root (e.g., `"ministr-core"`).
    pub dir_prefix: String,
}

/// Cross-package import graph for a workspace.
///
/// Holds package metadata and provides lookups from crate names to directory
/// prefixes for cross-crate reference resolution.
#[derive(Debug, Clone, Default)]
pub struct PackageGraph {
    packages: Vec<PackageInfo>,
}

/// A directed edge in the cross-package import graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageEdge {
    /// The package that contains the import statement.
    pub from_package: String,
    /// The package being imported.
    pub to_package: String,
    /// Number of cross-package references.
    pub ref_count: usize,
}

impl PackageGraph {
    /// Build a `PackageGraph` from a Cargo workspace root and its member directories.
    ///
    /// Reads each member's `Cargo.toml` to extract the package name, then
    /// computes the crate name (hyphens → underscores) and directory prefix.
    #[must_use]
    pub fn from_cargo_workspace(root: &Path, members: &[PathBuf]) -> Self {
        let mut packages = Vec::new();

        for member_dir in members {
            let cargo_toml = member_dir.join("Cargo.toml");
            let Some(name) = read_cargo_package_name(&cargo_toml) else {
                continue;
            };

            let dir_prefix = member_dir
                .strip_prefix(root)
                .unwrap_or(member_dir.as_path())
                .to_string_lossy()
                .to_string();

            let crate_name = name.replace('-', "_");

            packages.push(PackageInfo {
                name,
                crate_name,
                dir_prefix,
            });
        }

        Self { packages }
    }

    /// Create an empty `PackageGraph`.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns `true` if the graph contains no packages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Look up the directory prefix for a crate name.
    ///
    /// The crate name uses underscores (e.g., `"ministr_core"`), as it appears
    /// in Rust `use` declarations.
    #[must_use]
    pub fn dir_prefix_for_crate(&self, crate_name: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|p| p.crate_name == crate_name)
            .map(|p| p.dir_prefix.as_str())
    }

    /// Determine which package owns a given file path.
    ///
    /// The file path should be relative to the workspace root
    /// (e.g., `"ministr-core/src/config.rs"`).
    #[must_use]
    pub fn package_for_file(&self, file_path: &str) -> Option<&str> {
        self.packages
            .iter()
            .find(|p| file_path.starts_with(&p.dir_prefix))
            .map(|p| p.name.as_str())
    }

    /// Add a package to the graph.
    ///
    /// If a package with the same crate name already exists, it is replaced.
    /// This is used to register cloned dependency packages so that
    /// cross-crate reference resolution can link back to consuming code.
    pub fn add_package(&mut self, info: PackageInfo) {
        self.packages.retain(|p| p.crate_name != info.crate_name);
        self.packages.push(info);
    }

    /// Build a `PackageGraph` entry from a cloned repository directory.
    ///
    /// Reads `Cargo.toml` files in the clone directory to discover package
    /// names. For workspaces, all member packages are added. For single
    /// crates, the single package is added.
    ///
    /// The `dir_prefix` is set to the clone directory path so that symbol
    /// file paths (which are relative to the clone dir) can be matched.
    #[must_use]
    pub fn from_cloned_repo(clone_dir: &Path) -> Self {
        let mut graph = Self::default();

        // Try workspace detection first.
        if let Some(ws) = crate::workspace::detect_workspace(clone_dir) {
            let ws_graph = Self::from_cargo_workspace(clone_dir, &ws.members);
            for pkg in ws_graph.packages {
                graph.packages.push(pkg);
            }
            return graph;
        }

        // Single crate: read Cargo.toml directly.
        let cargo_toml = clone_dir.join("Cargo.toml");
        if let Some(name) = read_cargo_package_name(&cargo_toml) {
            let crate_name = name.replace('-', "_");
            let dir_prefix = clone_dir.to_string_lossy().to_string();
            graph.packages.push(PackageInfo {
                name,
                crate_name,
                dir_prefix,
            });
        }

        graph
    }

    /// Return all packages in the graph.
    #[must_use]
    pub fn packages(&self) -> &[PackageInfo] {
        &self.packages
    }

    /// Compute cross-package edges from a set of (`from_file`, `to_file`) reference pairs.
    ///
    /// Groups references by their owning packages and counts edges.
    #[must_use]
    pub fn compute_edges(&self, ref_pairs: &[(&str, &str)]) -> Vec<PackageEdge> {
        use std::collections::HashMap;

        let mut edge_counts: HashMap<(String, String), usize> = HashMap::new();

        for (from_file, to_file) in ref_pairs {
            let from_pkg = self.package_for_file(from_file);
            let to_pkg = self.package_for_file(to_file);

            if let (Some(from), Some(to)) = (from_pkg, to_pkg)
                && from != to
            {
                *edge_counts
                    .entry((from.to_string(), to.to_string()))
                    .or_insert(0) += 1;
            }
        }

        let mut edges: Vec<PackageEdge> = edge_counts
            .into_iter()
            .map(|((from, to), count)| PackageEdge {
                from_package: from,
                to_package: to,
                ref_count: count,
            })
            .collect();

        edges.sort_by(|a, b| {
            a.from_package
                .cmp(&b.from_package)
                .then(a.to_package.cmp(&b.to_package))
        });

        edges
    }
}

/// Read the `[package] name` from a `Cargo.toml` file.
fn read_cargo_package_name(cargo_toml: &Path) -> Option<String> {
    let content = std::fs::read_to_string(cargo_toml).ok()?;
    let parsed: toml::Value = toml::from_str(&content).ok()?;
    parsed
        .get("package")?
        .get("name")?
        .as_str()
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_cargo_workspace() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create workspace members
        for (dir, name) in &[
            ("alpha-lib", "alpha-lib"),
            ("beta-core", "beta-core"),
            ("gamma", "gamma"),
        ] {
            let member_dir = root.join(dir);
            std::fs::create_dir_all(member_dir.join("src")).unwrap();
            std::fs::write(
                member_dir.join("Cargo.toml"),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            )
            .unwrap();
        }

        tmp
    }

    #[test]
    fn from_cargo_workspace_builds_package_list() {
        let tmp = setup_cargo_workspace();
        let root = tmp.path();

        let members = vec![
            root.join("alpha-lib"),
            root.join("beta-core"),
            root.join("gamma"),
        ];

        let graph = PackageGraph::from_cargo_workspace(root, &members);

        assert_eq!(graph.packages().len(), 3);
        assert!(!graph.is_empty());
    }

    #[test]
    fn dir_prefix_for_crate_maps_underscores_to_hyphens() {
        let tmp = setup_cargo_workspace();
        let root = tmp.path();

        let members = vec![
            root.join("alpha-lib"),
            root.join("beta-core"),
            root.join("gamma"),
        ];

        let graph = PackageGraph::from_cargo_workspace(root, &members);

        // Crate name uses underscores
        assert_eq!(graph.dir_prefix_for_crate("alpha_lib"), Some("alpha-lib"));
        assert_eq!(graph.dir_prefix_for_crate("beta_core"), Some("beta-core"));
        assert_eq!(graph.dir_prefix_for_crate("gamma"), Some("gamma"));
        assert_eq!(graph.dir_prefix_for_crate("unknown"), None);
    }

    #[test]
    fn package_for_file_identifies_owning_package() {
        let tmp = setup_cargo_workspace();
        let root = tmp.path();

        let members = vec![
            root.join("alpha-lib"),
            root.join("beta-core"),
            root.join("gamma"),
        ];

        let graph = PackageGraph::from_cargo_workspace(root, &members);

        assert_eq!(
            graph.package_for_file("alpha-lib/src/lib.rs"),
            Some("alpha-lib")
        );
        assert_eq!(
            graph.package_for_file("beta-core/src/config.rs"),
            Some("beta-core")
        );
        assert_eq!(graph.package_for_file("gamma/src/main.rs"), Some("gamma"));
        assert_eq!(graph.package_for_file("unknown/src/lib.rs"), None);
    }

    #[test]
    fn compute_edges_groups_cross_package_refs() {
        let tmp = setup_cargo_workspace();
        let root = tmp.path();

        let members = vec![
            root.join("alpha-lib"),
            root.join("beta-core"),
            root.join("gamma"),
        ];

        let graph = PackageGraph::from_cargo_workspace(root, &members);

        let ref_pairs = vec![
            ("gamma/src/main.rs", "alpha-lib/src/lib.rs"),
            ("gamma/src/main.rs", "beta-core/src/config.rs"),
            ("gamma/src/cli.rs", "alpha-lib/src/types.rs"),
            ("beta-core/src/lib.rs", "alpha-lib/src/lib.rs"),
            // Same-package ref — should be excluded
            ("alpha-lib/src/a.rs", "alpha-lib/src/b.rs"),
        ];

        let edges = graph.compute_edges(&ref_pairs);

        assert_eq!(edges.len(), 3);

        let gamma_to_alpha = edges
            .iter()
            .find(|e| e.from_package == "gamma" && e.to_package == "alpha-lib")
            .unwrap();
        assert_eq!(gamma_to_alpha.ref_count, 2);

        let gamma_to_beta = edges
            .iter()
            .find(|e| e.from_package == "gamma" && e.to_package == "beta-core")
            .unwrap();
        assert_eq!(gamma_to_beta.ref_count, 1);
    }

    #[test]
    fn empty_graph() {
        let graph = PackageGraph::empty();
        assert!(graph.is_empty());
        assert_eq!(graph.dir_prefix_for_crate("foo"), None);
        assert_eq!(graph.package_for_file("foo/src/lib.rs"), None);
    }

    #[test]
    fn add_package_registers_new_entry() {
        let mut graph = PackageGraph::empty();
        assert!(graph.is_empty());

        graph.add_package(PackageInfo {
            name: "serde".into(),
            crate_name: "serde".into(),
            dir_prefix: "/tmp/clones/serde-abc123".into(),
        });

        assert!(!graph.is_empty());
        assert_eq!(graph.packages().len(), 1);
        assert_eq!(
            graph.dir_prefix_for_crate("serde"),
            Some("/tmp/clones/serde-abc123")
        );
    }

    #[test]
    fn add_package_replaces_existing_crate() {
        let mut graph = PackageGraph::empty();
        graph.add_package(PackageInfo {
            name: "serde".into(),
            crate_name: "serde".into(),
            dir_prefix: "/old/path".into(),
        });
        graph.add_package(PackageInfo {
            name: "serde".into(),
            crate_name: "serde".into(),
            dir_prefix: "/new/path".into(),
        });

        assert_eq!(graph.packages().len(), 1);
        assert_eq!(graph.dir_prefix_for_crate("serde"), Some("/new/path"));
    }

    #[test]
    fn from_cloned_repo_detects_single_crate() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"my-dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();

        let graph = PackageGraph::from_cloned_repo(root);
        assert_eq!(graph.packages().len(), 1);
        assert!(graph.dir_prefix_for_crate("my_dep").is_some());
    }
}
