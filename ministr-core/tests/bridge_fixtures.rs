//! End-to-end bridge fixture tests.
//!
//! Each test loads a pair of fixture files (server + client), parses them with
//! tree-sitter, runs through the `BridgeLinker`, and asserts on the produced
//! links.

use ministr_core::code::bridge::linker::{BridgeLinker, SourceFile};
use ministr_core::code::bridge::{BridgeKind, EndpointRole};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_with(source: &[u8], language: &tree_sitter::Language) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(language).unwrap();
    parser.parse(source, None).unwrap()
}

fn read_fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/bridge/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
}

fn rust_language() -> tree_sitter::Language {
    tree_sitter_rust::LANGUAGE.into()
}

#[cfg(feature = "lang-javascript")]
fn js_language() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}

#[cfg(feature = "lang-typescript")]
fn ts_language() -> tree_sitter::Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}

#[cfg(feature = "lang-python")]
fn python_language() -> tree_sitter::Language {
    tree_sitter_python::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// Tauri fixtures
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-typescript")]
#[test]
fn tauri_command_fixtures_link() {
    use ministr_core::code::bridge::tauri::TauriCommandExtractor;

    let rust_src = read_fixture("tauri/commands.rs");
    let ts_src = read_fixture("tauri/app.ts");

    let rust_tree = parse_with(&rust_src, &rust_language());
    let ts_tree = parse_with(&ts_src, &ts_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(TauriCommandExtractor));

    let files = [
        SourceFile {
            file_path: "tauri/commands.rs",
            language: "rust",
            tree: &rust_tree,
            source: &rust_src,
        },
        SourceFile {
            file_path: "tauri/app.ts",
            language: "typescript",
            tree: &ts_tree,
            source: &ts_src,
        },
    ];

    let links = linker.extract_and_link(&files);

    assert!(
        links.len() >= 3,
        "expected at least 3 tauri command links, got {}",
        links.len()
    );
    for link in &links {
        assert_eq!(link.kind, BridgeKind::TauriCommand);
        assert_eq!(link.export.role, EndpointRole::Export);
        assert_eq!(link.import.role, EndpointRole::Import);
        assert_eq!(link.export.language, "rust");
        assert_eq!(link.import.language, "typescript");
    }

    let binding_keys: Vec<&str> = links
        .iter()
        .map(|l| l.export.binding_key.as_str())
        .collect();
    assert!(binding_keys.contains(&"greet"), "missing greet link");
    assert!(
        binding_keys.contains(&"get_settings"),
        "missing get_settings link"
    );
    assert!(
        binding_keys.contains(&"save_file"),
        "missing save_file link"
    );
}

// ---------------------------------------------------------------------------
// NAPI fixtures
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-javascript")]
#[test]
fn napi_fixtures_link() {
    use ministr_core::code::bridge::napi::NapiExtractor;

    let rust_src = read_fixture("napi/lib.rs");
    let js_src = read_fixture("napi/index.js");

    let rust_tree = parse_with(&rust_src, &rust_language());
    let js_tree = parse_with(&js_src, &js_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(NapiExtractor));

    let files = [
        SourceFile {
            file_path: "napi/lib.rs",
            language: "rust",
            tree: &rust_tree,
            source: &rust_src,
        },
        SourceFile {
            file_path: "napi/index.js",
            language: "javascript",
            tree: &js_tree,
            source: &js_src,
        },
    ];

    let links = linker.extract_and_link(&files);

    // Expect links for: add, get_version/getVersion, Calculator
    assert!(
        links.len() >= 2,
        "expected at least 2 napi links, got {}",
        links.len()
    );
    for link in &links {
        assert_eq!(link.kind, BridgeKind::Napi);
        assert_eq!(link.export.language, "rust");
        assert_eq!(link.import.language, "javascript");
    }
}

// ---------------------------------------------------------------------------
// wasm-bindgen fixtures
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-javascript")]
#[test]
fn wasm_bindgen_fixtures_link() {
    use ministr_core::code::bridge::wasm_bindgen::WasmBindgenExtractor;

    let rust_src = read_fixture("wasm_bindgen/lib.rs");
    let js_src = read_fixture("wasm_bindgen/app.js");

    let rust_tree = parse_with(&rust_src, &rust_language());
    let js_tree = parse_with(&js_src, &js_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(WasmBindgenExtractor));

    let files = [
        SourceFile {
            file_path: "wasm_bindgen/lib.rs",
            language: "rust",
            tree: &rust_tree,
            source: &rust_src,
        },
        SourceFile {
            file_path: "wasm_bindgen/app.js",
            language: "javascript",
            tree: &js_tree,
            source: &js_src,
        },
    ];

    let links = linker.extract_and_link(&files);

    assert!(
        links.len() >= 3,
        "expected at least 3 wasm-bindgen links, got {}",
        links.len()
    );
    for link in &links {
        assert_eq!(link.kind, BridgeKind::WasmBindgen);
        assert_eq!(link.export.language, "rust");
        assert_eq!(link.import.language, "javascript");
    }

    let binding_keys: Vec<&str> = links
        .iter()
        .map(|l| l.export.binding_key.as_str())
        .collect();
    assert!(binding_keys.contains(&"greet"), "missing greet link");
    assert!(
        binding_keys.contains(&"fibonacci"),
        "missing fibonacci link"
    );
    assert!(binding_keys.contains(&"Counter"), "missing Counter link");
}

// ---------------------------------------------------------------------------
// PyO3 fixtures
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-python")]
#[test]
fn pyo3_fixtures_link() {
    use ministr_core::code::bridge::pyo3::PyO3Extractor;

    let rust_src = read_fixture("pyo3/lib.rs");
    let py_src = read_fixture("pyo3/main.py");

    let rust_tree = parse_with(&rust_src, &rust_language());
    let py_tree = parse_with(&py_src, &python_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(PyO3Extractor));

    let files = [
        SourceFile {
            file_path: "pyo3/lib.rs",
            language: "rust",
            tree: &rust_tree,
            source: &rust_src,
        },
        SourceFile {
            file_path: "pyo3/main.py",
            language: "python",
            tree: &py_tree,
            source: &py_src,
        },
    ];

    let links = linker.extract_and_link(&files);

    // Expect links for: hello, Config, is_debug
    assert!(
        links.len() >= 2,
        "expected at least 2 pyo3 links, got {}",
        links.len()
    );
    for link in &links {
        assert_eq!(link.kind, BridgeKind::PyO3);
        assert_eq!(link.export.language, "rust");
        assert_eq!(link.import.language, "python");
    }
}

// ---------------------------------------------------------------------------
// HTTP route fixtures
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-typescript")]
#[test]
fn http_route_fixtures_link_rust_to_ts() {
    use ministr_core::code::bridge::http_route::HttpRouteExtractor;

    let rust_src = read_fixture("http_route/server.rs");
    let ts_src = read_fixture("http_route/client.ts");

    let rust_tree = parse_with(&rust_src, &rust_language());
    let ts_tree = parse_with(&ts_src, &ts_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(HttpRouteExtractor));

    let files = [
        SourceFile {
            file_path: "http_route/server.rs",
            language: "rust",
            tree: &rust_tree,
            source: &rust_src,
        },
        SourceFile {
            file_path: "http_route/client.ts",
            language: "typescript",
            tree: &ts_tree,
            source: &ts_src,
        },
    ];

    let links = linker.extract_and_link(&files);

    // Expect links for: GET /api/users, POST /api/users
    // DELETE /api/users/{id} vs DELETE /api/users/42 won't match exactly
    assert!(
        links.len() >= 2,
        "expected at least 2 http route links, got {}",
        links.len()
    );
    for link in &links {
        assert_eq!(link.kind, BridgeKind::HttpRoute);
    }

    let binding_keys: Vec<&str> = links
        .iter()
        .map(|l| l.export.binding_key.as_str())
        .collect();
    assert!(
        binding_keys.contains(&"GET /api/users"),
        "missing GET /api/users link"
    );
    assert!(
        binding_keys.contains(&"POST /api/users"),
        "missing POST /api/users link"
    );
}

// ---------------------------------------------------------------------------
// Tauri event fixtures (Rust emit ↔ TS listen)
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-typescript")]
#[test]
fn tauri_event_fixtures_link() {
    use ministr_core::code::bridge::tauri::TauriEventExtractor;

    let rust_src: &[u8] =
        b"fn setup(app: &tauri::App) {\n    app.emit(\"progress\", 50).unwrap();\n}\n";
    let ts_src: &[u8] = b"import { listen } from '@tauri-apps/api/event';\n\nexport function watch() {\n    listen(\"progress\", (e) => { console.log(e); });\n}\n";

    let rust_tree = parse_with(rust_src, &rust_language());
    let ts_tree = parse_with(ts_src, &ts_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(TauriEventExtractor));

    let files = [
        SourceFile {
            file_path: "events.rs",
            language: "rust",
            tree: &rust_tree,
            source: rust_src,
        },
        SourceFile {
            file_path: "watch.ts",
            language: "typescript",
            tree: &ts_tree,
            source: ts_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links.iter().any(|l| l.kind == BridgeKind::TauriEvent
            && l.export.binding_key == "progress"
            && l.export.role == EndpointRole::Export
            && l.import.role == EndpointRole::Import),
        "expected a tauri_event link for `progress` (Rust emit → TS listen), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// Electron IPC fixtures (main ipcMain.handle ↔ renderer ipcRenderer.invoke)
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-javascript")]
#[test]
fn electron_ipc_fixtures_link() {
    use ministr_core::code::bridge::electron::ElectronIpcExtractor;

    let main_src: &[u8] = b"const { ipcMain } = require('electron');\nipcMain.handle('get-config', async () => { return {}; });\n";
    let renderer_src: &[u8] = b"const { ipcRenderer } = require('electron');\nasync function load() {\n    return await ipcRenderer.invoke('get-config');\n}\n";

    let main_tree = parse_with(main_src, &js_language());
    let renderer_tree = parse_with(renderer_src, &js_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(ElectronIpcExtractor));

    let files = [
        SourceFile {
            file_path: "main.js",
            language: "javascript",
            tree: &main_tree,
            source: main_src,
        },
        SourceFile {
            file_path: "renderer.js",
            language: "javascript",
            tree: &renderer_tree,
            source: renderer_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links
            .iter()
            .any(|l| l.kind == BridgeKind::ElectronIpc && l.export.binding_key == "get-config"),
        "expected an electron_ipc link for channel `get-config`, got {links:?}",
    );
}

#[cfg(feature = "lang-python")]
#[test]
fn http_route_fixtures_python_server_exports() {
    use ministr_core::code::bridge::http_route::HttpRouteExtractor;

    let py_src = read_fixture("http_route/server.py");
    let py_tree = parse_with(&py_src, &python_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(HttpRouteExtractor));

    let files = [SourceFile {
        file_path: "http_route/server.py",
        language: "python",
        tree: &py_tree,
        source: &py_src,
    }];

    let endpoints = linker.extract_all(&files);

    assert!(
        endpoints.len() >= 3,
        "expected at least 3 python route exports, got {}",
        endpoints.len()
    );
    for ep in &endpoints {
        assert_eq!(ep.role, EndpointRole::Export);
        assert_eq!(ep.kind, BridgeKind::HttpRoute);
    }
}
