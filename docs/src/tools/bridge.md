<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#compass-tool"/></svg>
</div>

# iris_bridge

Query cross-language binding links between exported symbols and their consumers across language boundaries.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `query` | string | no | Search bridge links by symbol name or binding key |
| `kind` | string | no | Filter by bridge kind: `tauri`, `napi`, `pyo3`, `wasm_bindgen`, `http_route` |
| `top_k` | integer | no | Maximum number of results (default: 20) |

## Response

```json
{
  "bridges": [
    {
      "kind": "tauri",
      "binding_key": "open_file",
      "confidence": 1.0,
      "export": {
        "symbol_id": "sym-src-tauri/src/commands.rs::commands::open_file",
        "file_path": "src-tauri/src/commands.rs",
        "line": 42
      },
      "imports": [
        {
          "file_path": "src/App.tsx",
          "line": 87,
          "context": "invoke('open_file', { path: selectedPath })"
        }
      ]
    }
  ],
  "budget_status": { ... }
}
```

### Response Fields

| Field | Description |
|---|---|
| `bridges[].kind` | Binding framework (`tauri`, `napi`, `pyo3`, `wasm_bindgen`, `http_route`) |
| `bridges[].binding_key` | The binding identifier (command name, export name, route path) |
| `bridges[].confidence` | Link confidence score (1.0 = exact match, <1.0 = semantic fallback) |
| `bridges[].export` | The producing side (e.g., `#[tauri::command]` function) |
| `bridges[].imports[]` | Consumer sites (e.g., `invoke()` calls) |

### Supported Frameworks

| Framework | Export Side | Import Side |
|---|---|---|
| Tauri | `#[tauri::command]` | `invoke('name', ...)` |
| napi-rs | `#[napi]` | `import { name }` from native module |
| PyO3 | `#[pyfunction]`, `#[pymethods]` | `from module import name` |
| wasm-bindgen | `#[wasm_bindgen]` | JS/TS import from WASM module |
| HTTP routes | `#[get]`, `#[post]`, etc. | `fetch('/path')`, `axios.get('/path')` |

## Behavior

- Links are resolved with exact match first, then case-normalized, then semantic embedding fallback
- Confidence scores below 0.85 indicate fuzzy matches that should be verified
- Use `iris_references` with a symbol ID to find references without the cross-language filter
