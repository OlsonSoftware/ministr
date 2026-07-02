# ministr_bridge

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Cross-language bridge links (Tauri commands, NAPI exports, PyO3 functions, FFI, HTTP routes, etc.). Call before modifying any IPC or FFI boundary so you see every cross-language call site.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `bridge_kind` | string | no | Filter by bridge kind: 'tauri_command', 'tauri_event', 'napi', 'wasm_bindgen', 'pyo3', 'http_route', 'ffi', 'cgo', 'jni', 'uni_ffi', 'grpc', 'flutter_channel', 'electron_ipc' |
| `file_path` | string | no | Filter links where either endpoint is in this file path |
| `language` | string | no | Filter links involving this language (e.g. 'rust', 'typescript', 'javascript', 'python') |
| `project` | string | no | Optional linked-project label. Omit for primary corpus. |
| `query` | string | no | Search query to filter by binding key or symbol name (case-insensitive substring match) |

Annotations: read-only · idempotent.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
