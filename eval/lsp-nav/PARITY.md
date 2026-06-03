# LSP-equivalence parity matrix — ministr MCP ⇄ Language Server Protocol

**FL4.** The bar (the FL epic, set by the user 2026-06-01) is *capability
equivalence*, **not** a literal LSP wire-protocol server: an agent uses
ministr's **existing** MCP operations *instead of* wiring up a per-language LSP
— no per-language build, no editor — and still gets everything it would
otherwise reach an LSP for, across the whole lifecycle (understand → plan →
implement → verify → review).

Every gap is closed by an **existing** op, or by the FL2 (position addressing) /
FL3 (call hierarchy) / FL3b (type-hierarchy refs) / FL5 (diagnostics) / FL6
(test mapping) **extensions to those ops** — never by a redundant new tool
(think:127). The op contract is the `QueryBackend` trait in
`ministr-mcp/src/backend/mod.rs`.

## The position bridge (FL2)

LSP is position-addressed (`file:line:col`); ministr is symbol/name-addressed.
`symbol_at_position(file, line, col)` (FL2, built on the universal occurrence
index from FL1) resolves a cursor position to a symbol id, so every
name-addressed op below is **also** position-addressable — `ministr_definition`
and `ministr_references` accept either `{symbol_id}` or `{file, line, col}`.

## Navigation & symbols

| LSP method | What an agent needs it for | ministr MCP op | Closed by | Status |
|---|---|---|---|---|
| `textDocument/definition` | jump to where a symbol is defined | `ministr_definition` (id or `file:line:col`) | base + FL2 | ✅ equivalent |
| `textDocument/declaration` | same, for forward-declared symbols | `ministr_definition` | base | ✅ equivalent |
| `textDocument/references` | every use of a symbol | `ministr_references` (id or position; `ref_kind`) | base + FL2 | ✅ equivalent |
| `textDocument/implementation` | implementors of a trait / interface | `ministr_references` `ref_kind=implements` | base | ✅ equivalent |
| `textDocument/typeDefinition` | the type of an expression / value | `ministr_definition` on the type symbol (chained from `references` / occurrence) | base + FL2 | ✅ equivalent |
| `textDocument/hover` | signature + doc at a position | `ministr_definition` returns `signature` + `doc_comment` | base + FL2 | ✅ equivalent |
| `textDocument/signatureHelp` | the callee's signature | `ministr_definition.signature` | base | ✅ equivalent |
| `textDocument/documentSymbol` | the symbols in one file | `ministr_symbols` `file_path=…` (or `ministr_read` symbol spans) | base | ✅ equivalent |
| `workspace/symbol` | find a symbol anywhere by name | `ministr_symbols` `name` / `name_exact` / `kind` / `module` | base | ✅ equivalent |
| `textDocument/documentHighlight` | other occurrences of the token | occurrence index (FL1) via `symbol_at_position` | FL1 + FL2 | ✅ equivalent |

## Call & type hierarchy

| LSP method | Agent need | ministr MCP op | Closed by | Status |
|---|---|---|---|---|
| `callHierarchy/incomingCalls` | who calls this (blast radius) | `ministr_impact` `direction=incoming` | FL3 | ✅ equivalent |
| `callHierarchy/outgoingCalls` | what this calls (fan-out) | `ministr_impact` `direction=outgoing` | FL3 | ✅ equivalent |
| `typeHierarchy/supertypes`·`subtypes` | trait/impl hierarchy; interface-method refs across implementors | `ministr_references` `through_implementors=true` (+ `ref_kind=implements`) | FL3b | ✅ equivalent |

## Verify / review — lifecycle stages an LSP only partly reaches

| Need | ministr MCP op | Closed by | Status |
|---|---|---|---|
| `textDocument/publishDiagnostics` — build/lint errors as structured data | `ministr_diagnostics` — the project's own toolchain (cargo / tsc / eslint / ruff / go vet / … + any SARIF tool) normalised to one shape | FL5 | ✅ equivalent |
| "which tests exercise this symbol" | `ministr_impact` `tests_only=true` | FL6 | ➕ beyond LSP |

## ministr-only — what a per-language LSP structurally cannot do

| Capability | ministr MCP op | Note |
|---|---|---|
| cross-language navigation (Tauri / PyO3 / NAPI / wasm-bindgen / HTTP / FFI seams) | `ministr_bridge` | a single-language LSP is blind to the other side of the boundary |
| dead-code candidates | `ministr_dead` | zero-reference symbols, repo-wide |
| SOLID / architecture findings | `ministr_solid` | deterministic violation candidates |
| semantic search across code **and** docs | `ministr_survey` | hybrid retrieval, not text match |

## Intentionally out of scope (editor UI, not agent needs)

`textDocument/{completion, formatting, rangeFormatting, onTypeFormatting,
codeAction, codeLens, foldingRange, selectionRange, inlayHint, semanticTokens
full-document paint, rename apply}` are editor-interaction features. An agent
edits text directly and gets the *information* those would surface from the ops
above — e.g. it renames by editing + `ministr_references` for the call sites; it
completes from `ministr_symbols` + `ministr_definition`. A literal
wire-protocol shim for non-MCP editors is a possible thin follow-up — explicitly
**out of scope** for the equivalence goal.

## How this is verified

- **Always-on parity gate** — `ministr-mcp/tests/lsp_parity.rs` builds a small
  fixture corpus in-process and drives **every navigation/hierarchy/verify row
  above** through the real `Backend::local` MCP op contract, asserting each
  capability is wired and returns a non-degenerate answer. It runs in the
  default `cargo test` pass — *block, don't monitor* (the regression gate the
  `eval/lsp-nav` README deferred as "a deliberate later step").
- **Accuracy vs a real LSP** — `eval/lsp-nav` (`lsp_nav_benchmark`,
  `just bench-lsp`) compares ministr's def/refs answers against `rust-analyzer`
  over a hand-verified ground truth, and shows the cross-language coverage a
  single-language LSP cannot reach. Heavy (full self-index + LSIF), report-only,
  run on demand.
