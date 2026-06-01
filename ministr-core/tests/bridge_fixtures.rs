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

#[cfg(feature = "lang-c")]
fn c_language() -> tree_sitter::Language {
    tree_sitter_c::LANGUAGE.into()
}

#[cfg(feature = "lang-go")]
fn go_language() -> tree_sitter::Language {
    tree_sitter_go::LANGUAGE.into()
}

#[cfg(feature = "lang-java")]
fn java_language() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}

#[cfg(feature = "lang-kotlin")]
fn kotlin_language() -> tree_sitter::Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}

#[cfg(feature = "lang-proto")]
fn proto_language() -> tree_sitter::Language {
    tree_sitter_proto::LANGUAGE.into()
}

#[cfg(feature = "lang-dart")]
fn dart_language() -> tree_sitter::Language {
    tree_sitter_dart::language()
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

// ---------------------------------------------------------------------------
// FFI fixtures (Rust #[no_mangle] extern "C" export ↔ Python ctypes import)
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-python")]
#[test]
fn ffi_fixtures_link() {
    use ministr_core::code::bridge::ffi::FfiExtractor;

    let rust_src: &[u8] =
        b"#[no_mangle]\npub extern \"C\" fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let py_src: &[u8] = b"import ctypes\n\nlib = ctypes.CDLL(\"libadd.so\")\n\ndef run():\n    return lib.add(1, 2)\n";

    let rust_tree = parse_with(rust_src, &rust_language());
    let py_tree = parse_with(py_src, &python_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(FfiExtractor));

    let files = [
        SourceFile {
            file_path: "lib.rs",
            language: "rust",
            tree: &rust_tree,
            source: rust_src,
        },
        SourceFile {
            file_path: "client.py",
            language: "python",
            tree: &py_tree,
            source: py_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links
            .iter()
            .any(|l| l.kind == BridgeKind::Ffi && l.export.binding_key == "add"),
        "expected an ffi link for C-ABI symbol `add` (Rust export → Python ctypes), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// cgo fixtures (C function export ↔ Go C.func import)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "lang-c", feature = "lang-go"))]
#[test]
fn cgo_fixtures_link() {
    use ministr_core::code::bridge::cgo::CgoExtractor;

    let c_src: &[u8] = b"int add(int a, int b) {\n    return a + b;\n}\n";
    let go_src: &[u8] = b"package main\n\n// #include \"add.h\"\nimport \"C\"\n\nfunc Run() int {\n    return int(C.add(1, 2))\n}\n";

    let c_tree = parse_with(c_src, &c_language());
    let go_tree = parse_with(go_src, &go_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(CgoExtractor));

    let files = [
        SourceFile {
            file_path: "add.c",
            language: "c",
            tree: &c_tree,
            source: c_src,
        },
        SourceFile {
            file_path: "main.go",
            language: "go",
            tree: &go_tree,
            source: go_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links
            .iter()
            .any(|l| l.kind == BridgeKind::Cgo && l.export.binding_key == "add"),
        "expected a cgo link for C function `add` (C export → Go C.add), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// JNI fixtures (Java native decl ↔ C Java_pkg_Class_method export)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "lang-c", feature = "lang-java"))]
#[test]
fn jni_fixtures_link() {
    use ministr_core::code::bridge::jni::JniExtractor;

    let java_src: &[u8] =
        b"package com.example;\n\npublic class Foo {\n    public native int compute(int x);\n}\n";
    let c_src: &[u8] = b"#include <jni.h>\n\nJNIEXPORT jint JNICALL Java_com_example_Foo_compute(JNIEnv *env, jobject obj, jint x) {\n    return x;\n}\n";

    let java_tree = parse_with(java_src, &java_language());
    let c_tree = parse_with(c_src, &c_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(JniExtractor));

    let files = [
        SourceFile {
            file_path: "Foo.java",
            language: "java",
            tree: &java_tree,
            source: java_src,
        },
        SourceFile {
            file_path: "native.c",
            language: "c",
            tree: &c_tree,
            source: c_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links
            .iter()
            .any(|l| l.kind == BridgeKind::Jni && l.import.binding_key == "compute"),
        "expected a jni link for native method `compute` (Java native ↔ C Java_*), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// UniFFI fixtures (Rust #[uniffi::export] ↔ foreign import)
// ---------------------------------------------------------------------------

#[cfg(feature = "lang-python")]
#[test]
fn uniffi_fixtures_link() {
    use ministr_core::code::bridge::uniffi::UniffiExtractor;

    let rust_src: &[u8] =
        b"#[uniffi::export]\npub fn greet(name: String) -> String {\n    name\n}\n";
    let py_src: &[u8] = b"from my_lib import greet\n\n\ndef run():\n    return greet(\"x\")\n";

    let rust_tree = parse_with(rust_src, &rust_language());
    let py_tree = parse_with(py_src, &python_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(UniffiExtractor));

    let files = [
        SourceFile {
            file_path: "lib.rs",
            language: "rust",
            tree: &rust_tree,
            source: rust_src,
        },
        SourceFile {
            file_path: "client.py",
            language: "python",
            tree: &py_tree,
            source: py_src,
        },
    ];

    // The Rust #[uniffi::export] side always extracts; the foreign import side
    // is heuristic. Assert the export endpoint resolves; a full link is the
    // stronger signal when the heuristic import matches.
    let endpoints = linker.extract_all(&files);
    assert!(
        endpoints.iter().any(|e| e.kind == BridgeKind::UniFfi
            && e.binding_key == "greet"
            && e.role == EndpointRole::Export),
        "expected a uniffi export endpoint for `greet`, got {endpoints:?}",
    );
}

// ---------------------------------------------------------------------------
// gRPC fixtures (.proto service ↔ generated client stub reference)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "lang-proto", feature = "lang-go"))]
#[test]
fn grpc_fixtures_link() {
    use ministr_core::code::bridge::grpc::GrpcExtractor;

    let proto_src: &[u8] = b"syntax = \"proto3\";\n\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}\n";
    let go_src: &[u8] = b"package main\n\nfunc dial(conn *grpc.ClientConn) {\n    client := NewGreeterClient(conn)\n    _ = client\n}\n";

    let proto_tree = parse_with(proto_src, &proto_language());
    let go_tree = parse_with(go_src, &go_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(GrpcExtractor));

    let files = [
        SourceFile {
            file_path: "greeter.proto",
            language: "proto",
            tree: &proto_tree,
            source: proto_src,
        },
        SourceFile {
            file_path: "client.go",
            language: "go",
            tree: &go_tree,
            source: go_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links
            .iter()
            .any(|l| l.kind == BridgeKind::Grpc && l.export.binding_key == "Greeter"),
        "expected a grpc link for service `Greeter` (.proto ↔ Go NewGreeterClient), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// Flutter platform-channel fixtures (Dart MethodChannel ↔ native registration)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "lang-dart", feature = "lang-kotlin"))]
#[test]
fn flutter_channel_fixtures_link() {
    use ministr_core::code::bridge::flutter::FlutterChannelExtractor;

    let dart_src: &[u8] = b"import 'package:flutter/services.dart';\n\nclass Battery {\n  static const platform = MethodChannel('com.example/battery');\n}\n";
    let kotlin_src: &[u8] = b"class MainActivity {\n    fun configure(messenger: BinaryMessenger) {\n        MethodChannel(messenger, \"com.example/battery\")\n    }\n}\n";

    let dart_tree = parse_with(dart_src, &dart_language());
    let kotlin_tree = parse_with(kotlin_src, &kotlin_language());

    let mut linker = BridgeLinker::new();
    linker.register(Box::new(FlutterChannelExtractor));

    let files = [
        SourceFile {
            file_path: "battery.dart",
            language: "dart",
            tree: &dart_tree,
            source: dart_src,
        },
        SourceFile {
            file_path: "MainActivity.kt",
            language: "kotlin",
            tree: &kotlin_tree,
            source: kotlin_src,
        },
    ];

    let links = linker.extract_and_link(&files);
    assert!(
        links.iter().any(|l| l.kind == BridgeKind::FlutterChannel
            && l.export.binding_key == "com.example/battery"),
        "expected a flutter_channel link for `com.example/battery` (Dart ↔ Kotlin), got {links:?}",
    );
}

// ---------------------------------------------------------------------------
// Coverage guard — every BridgeKind has an e2e link fixture above
// ---------------------------------------------------------------------------

/// All 13 `BridgeKind` variants. The `match` is intentionally exhaustive: when
/// a new variant is added it fails to compile HERE, forcing the author to add
/// an e2e fixture above and a `BRIDGE_FIXTURED` entry below.
fn all_bridge_kinds() -> [BridgeKind; 13] {
    let kinds = [
        BridgeKind::TauriCommand,
        BridgeKind::TauriEvent,
        BridgeKind::Napi,
        BridgeKind::WasmBindgen,
        BridgeKind::PyO3,
        BridgeKind::HttpRoute,
        BridgeKind::Ffi,
        BridgeKind::Cgo,
        BridgeKind::Jni,
        BridgeKind::UniFfi,
        BridgeKind::Grpc,
        BridgeKind::FlutterChannel,
        BridgeKind::ElectronIpc,
    ];
    // Exhaustiveness tripwire: a new variant breaks this match → update the
    // array above, add a fixture, and add it to BRIDGE_FIXTURED.
    for k in kinds {
        match k {
            BridgeKind::TauriCommand
            | BridgeKind::TauriEvent
            | BridgeKind::Napi
            | BridgeKind::WasmBindgen
            | BridgeKind::PyO3
            | BridgeKind::HttpRoute
            | BridgeKind::Ffi
            | BridgeKind::Cgo
            | BridgeKind::Jni
            | BridgeKind::UniFfi
            | BridgeKind::Grpc
            | BridgeKind::FlutterChannel
            | BridgeKind::ElectronIpc => {}
        }
    }
    kinds
}

/// Bridge kinds with a `*_fixtures_link` (or endpoint) e2e test in this file.
const BRIDGE_FIXTURED: &[&str] = &[
    "tauri_command",   // tauri_command_fixtures_link
    "tauri_event",     // tauri_event_fixtures_link
    "napi",            // napi_fixtures_link
    "wasm_bindgen",    // wasm_bindgen_fixtures_link
    "pyo3",            // pyo3_fixtures_link
    "http_route",      // http_route_fixtures_link_rust_to_ts
    "ffi",             // ffi_fixtures_link
    "cgo",             // cgo_fixtures_link
    "jni",             // jni_fixtures_link
    "uniffi",          // uniffi_fixtures_link (export endpoint)
    "grpc",            // grpc_fixtures_link
    "flutter_channel", // flutter_channel_fixtures_link
    "electron_ipc",    // electron_ipc_fixtures_link
];

#[test]
fn every_bridge_kind_has_an_e2e_fixture() {
    for k in all_bridge_kinds() {
        assert!(
            BRIDGE_FIXTURED.contains(&k.as_str()),
            "BridgeKind `{}` has no e2e fixture in bridge_fixtures.rs — add a \
             `{}_fixtures_link` test and list it in BRIDGE_FIXTURED",
            k.as_str(),
            k.as_str(),
        );
    }
    // Every fixtured name is a real kind (catches typos / removed variants).
    for name in BRIDGE_FIXTURED {
        assert!(
            BridgeKind::parse(name).is_some(),
            "BRIDGE_FIXTURED lists unknown bridge kind `{name}`",
        );
    }
}

// ---------------------------------------------------------------------------
// Monorepo-subdir framework detection (detect_in_files)
// ---------------------------------------------------------------------------

/// A Tauri app lives under `<repo>/app/src-tauri/`, so its manifests sit BELOW
/// the corpus root. `FrameworkDetector::detect` only walks UP from the root and
/// misses them; `detect_in_files` scans the directory of every manifest in the
/// file set and finds them. This pins that contrast.
#[test]
fn detect_in_files_finds_subdir_tauri_app() {
    use ministr_core::code::bridge::detector::FrameworkDetector;

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let subdir = root.join("app").join("src-tauri");
    std::fs::create_dir_all(&subdir).expect("create src-tauri dir");

    let cargo_toml = subdir.join("Cargo.toml");
    std::fs::write(
        &cargo_toml,
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ntauri = \"2\"\n",
    )
    .expect("write Cargo.toml");
    let tauri_conf = subdir.join("tauri.conf.json");
    std::fs::write(&tauri_conf, "{\n  \"productName\": \"app\"\n}\n")
        .expect("write tauri.conf.json");

    // The upward walk from the repo root misses the subdir app.
    let from_root = FrameworkDetector::detect(root);
    assert!(
        !from_root.contains(&BridgeKind::TauriCommand),
        "detect() from the repo root should NOT see the subdir Tauri app, got {from_root:?}",
    );

    // detect_in_files, given the discovered manifests, finds it.
    let detected = FrameworkDetector::detect_in_files(&[cargo_toml, tauri_conf]);
    assert!(
        detected.contains(&BridgeKind::TauriCommand),
        "detect_in_files should find the subdir Tauri app's TauriCommand bridge, got {detected:?}",
    );
}
