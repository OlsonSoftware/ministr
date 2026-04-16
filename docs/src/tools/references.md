<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#arrow-up-right"/></svg>
</div>

# iris_references

Find all references to a symbol — callers, implementors, importers, and cross-language binding sites.

## Parameters

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `symbol_id` | string | yes | — | Symbol ID from `iris_symbols` |
| `ref_kind` | string | no | — | Filter by reference kind: `calls`, `implements`, `imports`, `extends`, `mentions` |
| `top_k` | integer | no | 50 | Maximum number of results |

## Response

```json
{
  "references": [
    {
      "from_symbol_id": "sym-src/handlers.rs::handlers::login_handler",
      "from_name": "login_handler",
      "from_file": "src/handlers.rs",
      "ref_kind": "calls",
      "line": 28,
      "context": "let claims = validate_token(&token)?;"
    },
    {
      "from_symbol_id": "sym-tests/auth_test.rs::tests::rejects_expired",
      "from_name": "rejects_expired",
      "from_file": "tests/auth_test.rs",
      "ref_kind": "calls",
      "line": 15,
      "context": "let result = validate_token(\"expired.token.here\");"
    }
  ],
  "budget_status": { ... }
}
```

### Response Fields

| Field | Description |
|---|---|
| `references[].from_symbol_id` | The symbol making the reference |
| `references[].ref_kind` | How the symbol is referenced (calls, implements, etc.) |
| `references[].line` | Line number of the reference |
| `references[].context` | The line of code containing the reference |

### Reference Kinds

| Kind | Description |
|---|---|
| `calls` | Function/method call site |
| `implements` | Trait implementation |
| `imports` | `use` or `import` statement |
| `extends` | Class/struct extension or inheritance |
| `mentions` | Type reference in signatures or annotations |

## Behavior

- Includes cross-crate references within the same workspace
- Includes cross-language bridges when applicable (e.g., a Rust `#[tauri::command]` referenced by `invoke()` in TypeScript)
- For finding cross-language bindings explicitly, use `iris_bridge`
- Use this before modifying or deleting shared code to understand blast radius
