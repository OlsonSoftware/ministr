# Cross-language bridges

A text search dies at a language boundary: a Rust `#[tauri::command] fn
save_file` and the TypeScript `invoke("save_file")` that calls it share
nothing a grep can connect — and with napi-rs's snake_case→camelCase
transform, not even the name survives. Bridges are indexed, queryable links
across these boundaries, served by `ministr_bridge`.

## The binding kinds

| Kind | Boundary |
|---|---|
| `tauri_command` | `#[tauri::command]` ↔ `invoke("name")` |
| `tauri_event` | `emit` ↔ `listen` |
| `napi` | napi-rs `#[napi]` ↔ JS/TS (case-transformed names handled) |
| `wasm_bindgen` | `#[wasm_bindgen]` ↔ JS imports from the generated pkg |
| `pyo3` | `#[pyfunction]` / `#[pyclass]` / `#[pymethods]` ↔ Python |
| `http_route` | route definitions ↔ client fetch/request calls |
| `ffi` | `extern "C"` ↔ ctypes and friends |
| `cgo` | Go `C.func(...)` ↔ C definitions |
| `jni` | Java/Kotlin `native`/`external` ↔ `Java_*` exports |
| `uni_ffi` | `#[uniffi::export]` ↔ Swift/Kotlin/Python |
| `grpc` | `.proto` services ↔ generated client stubs |
| `flutter_channel` | Dart `MethodChannel("name")` ↔ native handlers |
| `electron_ipc` | `ipcRenderer.invoke/send` ↔ `ipcMain.handle/on` |

## How linking works

1. **Extraction** — one extractor per kind scans the files in its
   applicable languages, emitting export and import endpoints, each with a
   binding key and a confidence.
2. **Exact linking** — endpoints are grouped by kind and binding key;
   exports pair with imports.
3. **Case-transformed linking** — a second pass with case-normalized keys
   catches snake_case ↔ camelCase conventions, at a lower confidence.
4. **Semantic fallback** — still-unmatched endpoints are embedded and
   paired by name similarity above a threshold.

A stored link's confidence is the minimum of its two endpoints' — links
never claim more certainty than their weakest side.

## In practice

The server's own instructions tell agents to call `ministr_bridge` before
changing any IPC or FFI boundary, so every cross-language call site is on
the table before a signature changes underneath it. `ministr_references`
also surfaces bridge edges via `ref_kind: "bridge"`.
