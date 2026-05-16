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
/// use ministr_core::code::bridge::detector::FrameworkDetector;
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
    // FFI: native interop — function loaders, JNI, and C-bindings generators.
    ("libloading", &[BridgeKind::Ffi]),
    ("libffi", &[BridgeKind::Ffi]),
    ("jni", &[BridgeKind::Ffi, BridgeKind::Jni]),
    ("bindgen", &[BridgeKind::Ffi]),
    ("cbindgen", &[BridgeKind::Ffi]),
    // UniFFI (Rust ↔ Swift/Kotlin/Python mobile bindings).
    ("uniffi", &[BridgeKind::UniFfi]),
    // gRPC (Rust tonic/prost).
    ("tonic", &[BridgeKind::Grpc]),
    ("prost", &[BridgeKind::Grpc]),
    ("grpcio", &[BridgeKind::Grpc]),
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
    // FFI: Node/Deno C-call libraries.
    ("ffi-napi", &[BridgeKind::Ffi]),
    ("koffi", &[BridgeKind::Ffi]),
    ("node-ffi", &[BridgeKind::Ffi]),
    ("@grpc/grpc-js", &[BridgeKind::Grpc]),
    ("@grpc/proto-loader", &[BridgeKind::Grpc]),
    // Electron — quoted to avoid matching substrings like
    // `electron-builder` only; the dependency key is `"electron"`.
    ("\"electron\"", &[BridgeKind::ElectronIpc]),
];

/// `pyproject.toml` dependency markers.
const PYTHON_MARKERS: &[(&str, &[BridgeKind])] = &[
    ("pyo3", &[BridgeKind::PyO3]),
    ("maturin", &[BridgeKind::PyO3]),
    ("fastapi", &[BridgeKind::HttpRoute]),
    ("flask", &[BridgeKind::HttpRoute]),
    ("django", &[BridgeKind::HttpRoute]),
    // FFI: cffi is the only manifest-visible signal.  Note that ctypes is in
    // the stdlib and won't appear here; bare ctypes-only projects are picked
    // up by the C/C++ source-presence fallback in `detect()` instead.
    ("cffi", &[BridgeKind::Ffi]),
    ("grpcio", &[BridgeKind::Grpc]),
    ("grpcio-tools", &[BridgeKind::Grpc]),
];

impl FrameworkDetector {
    /// Detect bridge frameworks present at or above the given directory.
    ///
    /// Walks up from `start_dir` checking each directory for manifest files
    /// (`Cargo.toml`, `package.json`, `pyproject.toml`, `tauri.conf.json`).
    /// Stops at a `.git` boundary or filesystem root. Returns a sorted,
    /// deduplicated list of detected [`BridgeKind`]s.
    #[must_use]
    pub fn detect(start_dir: &Path) -> Vec<BridgeKind> {
        let mut kinds = BTreeSet::new();
        let mut dir = start_dir.to_path_buf();
        let mut go_mod_seen = false;

        loop {
            Self::scan_cargo_toml(&dir, &mut kinds);
            Self::scan_package_json(&dir, &mut kinds);
            Self::scan_pyproject_toml(&dir, &mut kinds);
            Self::scan_tauri_conf(&dir, &mut kinds);
            Self::scan_pubspec(&dir, &mut kinds);
            if dir.join("go.mod").exists() {
                go_mod_seen = true;
            }

            // Stop at the ministr project root or VCS boundary.
            if dir.join(".ministr.toml").exists() || dir.join(".git").exists() {
                break;
            }
            if !dir.pop() {
                break;
            }
        }

        // Filesystem fallback for FFI: a project containing C/C++ source files
        // is almost certainly going to interop with someone via C ABI even when
        // there's no manifest signal (kernel modules, embedded firmware, lone
        // single-file libraries). This is the only place detection becomes
        // filesystem-driven rather than manifest-driven.
        if Self::has_c_or_cpp_sources(start_dir) {
            kinds.insert(BridgeKind::Ffi);
            // A Go module that also ships C sources is almost certainly
            // using cgo (the only first-class Go↔C mechanism).
            if go_mod_seen {
                kinds.insert(BridgeKind::Cgo);
            }
        }

        // `.proto` files present → gRPC is in play (generated stubs
        // are matched name-only, so this is the activation signal).
        if Self::has_ext(start_dir, &["proto"]) {
            kinds.insert(BridgeKind::Grpc);
        }

        kinds.into_iter().collect()
    }

    /// Whether `dir` contains any C/C++ source files at the top level.
    ///
    /// Cheap one-level glob — does not recurse. Adequate because most
    /// C/C++ projects place at least one `.c`/`.cpp`/`.h` at or near the
    /// corpus root, and recursing the whole tree would dominate detection
    /// time on large projects.
    fn has_c_or_cpp_sources(dir: &Path) -> bool {
        Self::has_ext(dir, &["c", "cpp", "cc", "cxx", "h", "hpp", "hh", "hxx"])
    }

    /// Whether `dir` contains any top-level file with one of `exts`.
    /// Cheap one-level scan (no recursion) — same rationale as the
    /// C/C++ fallback.
    fn has_ext(dir: &Path, exts: &[&str]) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str())
                && exts.contains(&ext)
            {
                return true;
            }
        }
        false
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

    /// Scan for `pubspec.yaml` — its presence indicates a Flutter/Dart
    /// project, which (when it has native platform code) uses platform
    /// channels. The `flutter:` key narrows it to Flutter specifically.
    fn scan_pubspec(root: &Path, kinds: &mut BTreeSet<BridgeKind>) {
        let Ok(content) = std::fs::read_to_string(root.join("pubspec.yaml")) else {
            return;
        };
        if content.contains("flutter:") || content.contains("sdk: flutter") {
            kinds.insert(BridgeKind::FlutterChannel);
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
    fn detect_ffi_from_cargo_libloading() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\nlibloading = \"0.8\"\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Ffi));
    }

    #[test]
    fn detect_ffi_from_cargo_jni() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\njni = \"0.21\"\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Ffi));
    }

    #[test]
    fn detect_ffi_from_npm_ffi_napi() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"dependencies": {"ffi-napi": "^4.0.3"}}"#,
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Ffi));
    }

    #[test]
    fn detect_ffi_from_python_cffi() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyproject.toml"),
            "[project]\ndependencies = [\"cffi >= 1.16\"]\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Ffi));
    }

    #[test]
    fn detect_ffi_from_bare_c_directory() {
        let tmp = tempfile::tempdir().unwrap();
        // No manifests at all — just a single .c file.
        std::fs::write(tmp.path().join("hello.c"), "int main(void) { return 0; }\n").unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(
            kinds.contains(&BridgeKind::Ffi),
            "bare C source should trigger FFI detection, got: {kinds:?}"
        );
    }

    #[test]
    fn detect_ffi_from_bare_cpp_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("greet.hpp"),
            "void greet(const char *name);\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(
            kinds.contains(&BridgeKind::Ffi),
            "bare C++ header should trigger FFI detection, got: {kinds:?}"
        );
    }

    #[test]
    fn detect_cgo_from_go_mod_plus_c_source() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("go.mod"),
            "module example.com/m\n\ngo 1.22\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("bridge.c"), "int work(void){return 0;}\n").unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Cgo), "got {kinds:?}");
    }

    #[test]
    fn no_cgo_without_c_sources() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("go.mod"), "module example.com/m\n").unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(!kinds.contains(&BridgeKind::Cgo), "got {kinds:?}");
    }

    #[test]
    fn detect_grpc_from_proto_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("svc.proto"), "service S {}\n").unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Grpc), "got {kinds:?}");
    }

    #[test]
    fn detect_uniffi_and_grpc_from_cargo() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\nuniffi = \"0.28\"\ntonic = \"0.12\"\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::UniFfi), "got {kinds:?}");
        assert!(kinds.contains(&BridgeKind::Grpc), "got {kinds:?}");
    }

    #[test]
    fn detect_jni_kind_from_cargo() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\njni = \"0.21\"\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::Jni), "got {kinds:?}");
        assert!(kinds.contains(&BridgeKind::Ffi), "got {kinds:?}");
    }

    #[test]
    fn detect_flutter_from_pubspec() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pubspec.yaml"),
            "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::FlutterChannel), "got {kinds:?}");
    }

    #[test]
    fn detect_electron_from_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            "{\n  \"devDependencies\": { \"electron\": \"^30.0.0\" }\n}\n",
        )
        .unwrap();
        let kinds = FrameworkDetector::detect(tmp.path());
        assert!(kinds.contains(&BridgeKind::ElectronIpc), "got {kinds:?}");
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
