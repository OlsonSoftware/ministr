---
description: Query cross-language bridge links (Tauri, napi-rs, PyO3, wasm-bindgen, JNI, UniFFI, cgo, Electron, Flutter, gRPC, HTTP routes, FFI) using ministr_bridge. Use BEFORE changing any IPC/FFI/HTTP boundary.
---

Find cross-language bridges for: $ARGUMENTS

Use the `ministr_bridge` MCP tool. If the user named a bridge kind (`tauri_command`, `pyo3`, `napi`, `wasm_bindgen`, `jni`, `unifii`, `cgo`, `electron_ipc`, `flutter_channel`, `grpc`, `http_route`, `ffi`), pass it as `kind`. Otherwise pass the input as a free-text `query`.

Bridge results show both the export (definition side) and the import (call site) of every cross-language call. This is the only tool that can connect, e.g., a Rust `#[tauri::command]` to the TypeScript `invoke("…")` that calls it.
