//! Framework auto-detection for cross-language bridges.
//!
//! Scans project manifest files (`Cargo.toml`, `package.json`, `pyproject.toml`,
//! `tauri.conf.json`) to determine which bridge frameworks are present,
//! enabling targeted extractor activation.

use std::collections::BTreeSet;
use std::path::Path;

use super::BridgeKind;

/// Detects which cross-language bridge frameworks are present in a project.
///
/// Scans manifest files at the corpus root to identify dependencies that
/// indicate specific bridge mechanisms. The detected kinds can then be used
/// to selectively register only the relevant [`BridgeExtractor`]s.
///
/// [`BridgeExtractor`]: super::BridgeExtractor
///
/// # Examples
///
/// ```
/// use iris_core::code::bridge::detector::FrameworkDetector;
///
/// // On a directory with no manifest files, nothing is detected:
/// let detected = FrameworkDetector::detect(std::path::Path::new("/nonexistent"));
/// assert!(detected.is_empty());
/// ```
pub struct FrameworkDetector;

/// Cargo dependency markers that indicate a specific bridge kind.
const CARGO_MARKERS: &[(&str, &[BridgeKind])] = &[
    ("tauri", &[BridgeKind::TauriCommand, BridgeKind::TauriEvent]),
    ("napi", &[BridgeKind::Napi]),
    ("napi-derive", &[BridgeKind::Napi]),
    ("wasm-bindgen", &[BridgeKind::WasmBindgen]),
    ("pyo3", &[BridgeKind::PyO3]),
    ("actix-web", &[BridgeKind::HttpRoute]),
    ("axum", &[BridgeKind::HttpRoute]),
    ("rocket", &[BridgeKind::HttpRoute]),
];

/// `package.json` dependency markers.
const NPM_MARKERS: &[(&str, &[BridgeKind])] = &[
    (
        "@tauri-apps/api",
        &[BridgeKind::TauriCommand, BridgeKind::TauriEvent],
    ),
    ("express", &[BridgeKind::HttpRoute]),
    ("fastify", &[BridgeKind::HttpRoute]),
    ("@napi-rs/cli", &[BridgeKind::Napi]),
];

/// `pyproject.toml` dependency markers.
const PYTHON_MARKERS: &[(&str, &[BridgeKind])] = &[
    ("pyo3", &[BridgeKind::PyO3]),
    ("maturin", &[BridgeKind::PyO3]),
    ("fastapi", &[BridgeKind::HttpRoute]),
    ("flask", &[BridgeKind::HttpRoute]),
    ("django", &[BridgeKind::HttpRoute]),
];

impl FrameworkDetector {
    /// Detect bridge frameworks present at the given corpus root.
    ///
    /// Scans manifest files and returns a sorted, deduplicated list of
    /// detected [`BridgeKind`]s.
    #[must_use]
    pub fn detect(corpus_root: &Path) -> Vec<BridgeKind> {
        let mut kinds = BTreeSet::new();

        Self::scan_cargo_toml(corpus_root, &mut kinds);
        Self::scan_package_json(corpus_root, &mut kinds);
        Self::scan_pyproject_toml(corpus_root, &mut kinds);
        Self::scan_tauri_conf(corpus_root, &mut kinds);

        kinds.into_iter().collect()
    }

    /// Scan `Cargo.toml` for bridge-related dependencies.
    fn scan_cargo_toml(root: &Path, kinds: &mut BTreeSet<BridgeKind>) {
        let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) else {
            return;
        };

        // Simple string-matching against dependency keys.
        // We look for the dependency name appearing as a TOML key in
        // [dependencies], [dev-dependencies], or [build-dependencies].
        for &(marker, bridge_kinds) in CARGO_MARKERS {
            if content_has_cargo_dep(&content, marker) {
                kinds.extend(bridge_kinds);
            }
        }
    }

    /// Scan `package.json` for bridge-related dependencies.
    fn scan_package_json(root: &Path, kinds: &mut BTreeSet<BridgeKind>) {
        let Ok(content) = std::fs::read_to_string(root.join("package.json")) else {
            return;
        };

        for &(marker, bridge_kinds) in NPM_MARKERS {
            if content.contains(marker) {
                kinds.extend(bridge_kinds);
            }
        }
    }

    /// Scan `pyproject.toml` for bridge-related dependencies.
    fn scan_pyproject_toml(root: &Path, kinds: &mut BTreeSet<BridgeKind>) {
        let Ok(content) = std::fs::read_to_string(root.join("pyproject.toml")) else {
            return;
        };

        for &(marker, bridge_kinds) in PYTHON_MARKERS {
            if content.contains(marker) {
                kinds.extend(bridge_kinds);
            }
        }
    }

    /// Scan for `tauri.conf.json` — its mere presence indicates a Tauri project.
    fn scan_tauri_conf(root: &Path, kinds: &mut BTreeSet<BridgeKind>) {
        // Tauri v1: src-tauri/tauri.conf.json
        // Tauri v2: src-tauri/tauri.conf.json (same path)
        let candidates = [
            root.join("tauri.conf.json"),
            root.join("src-tauri").join("tauri.conf.json"),
        ];

        for path in &candidates {
            if path.exists() {
                kinds.insert(BridgeKind::TauriCommand);
                kinds.insert(BridgeKind::TauriEvent);
                return;
            }
        }
    }
}

/// Check if a `Cargo.toml` content string contains a dependency with the given name.
///
/// Uses simple heuristics: looks for the package name as a TOML key
/// (e.g. `tauri = "..."` or `tauri = { version = "..." }`).
fn content_has_cargo_dep(content: &str, dep_name: &str) -> bool {
    // Match patterns like:
    //   dep_name = "..."
    //   dep_name = { ... }
    //   dep_name.workspace = true
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(dep_name) {
            let rest = rest.trim_start();
            if rest.starts_with('=') || rest.starts_with('.') {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.is_empty());
    }

    #[test]
    fn detect_tauri_from_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
tauri = { version = "2.0", features = ["dialog"] }
serde = "1"
"#,
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::TauriCommand));
        assert!(kinds.contains(&BridgeKind::TauriEvent));
    }

    #[test]
    fn detect_napi_from_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            r#"
[dependencies]
napi = "2"
napi-derive = "2"
"#,
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Napi));
    }

    #[test]
    fn detect_wasm_bindgen_from_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\nwasm-bindgen = \"0.2\"\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::WasmBindgen));
    }

    #[test]
    fn detect_pyo3_from_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\npyo3 = { version = \"0.21\", features = [\"extension-module\"] }\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::PyO3));
    }

    #[test]
    fn detect_tauri_from_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"@tauri-apps/api": "^2.0.0"}}"#,
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::TauriCommand));
        assert!(kinds.contains(&BridgeKind::TauriEvent));
    }

    #[test]
    fn detect_pyo3_from_pyproject_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[build-system]\nrequires = [\"maturin>=1.0\"]\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::PyO3));
    }

    #[test]
    fn detect_tauri_from_conf_json() {
        let tmp = tempfile::tempdir().unwrap();
        let src_tauri = tmp.path().join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
        std::fs::write(src_tauri.join("tauri.conf.json"), "{}").unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::TauriCommand));
        assert!(kinds.contains(&BridgeKind::TauriEvent));
    }

    #[test]
    fn detect_http_route_from_axum() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\naxum = \"0.7\"\ntokio = \"1\"\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::HttpRoute));
    }

    #[test]
    fn detect_multiple_frameworks() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\ntauri = \"2\"\npyo3 = \"0.21\"\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::TauriCommand));
        assert!(kinds.contains(&BridgeKind::TauriEvent));
        assert!(kinds.contains(&BridgeKind::PyO3));
        assert!(!kinds.contains(&BridgeKind::Napi));
    }

    #[test]
    fn cargo_dep_detection_does_not_false_positive() {
        let tmp = tempfile::tempdir().unwrap();
        // "taurine" should NOT match "tauri"
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\ntaurine = \"1.0\"\n",
        )
        .unwrap();

        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(
            !kinds.contains(&BridgeKind::TauriCommand),
            "taurine should not match tauri"
        );
    }

    #[test]
    fn content_has_cargo_dep_variants() {
        // Key = value
        assert!(content_has_cargo_dep("tauri = \"2.0\"", "tauri"));
        // Key = { ... }
        assert!(content_has_cargo_dep(
            "tauri = { version = \"2\" }",
            "tauri"
        ));
        // Key.workspace = true
        assert!(content_has_cargo_dep("tauri.workspace = true", "tauri"));
        // Substring mismatch
        assert!(!content_has_cargo_dep("taurine = \"1\"", "tauri"));
        // No match
        assert!(!content_has_cargo_dep("serde = \"1\"", "tauri"));
    }
}
