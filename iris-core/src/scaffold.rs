//! First-run scaffolding for agent configuration files.
//!
//! When iris starts in a repo for the first time, it generates `.claude/rules/`
//! files that teach AI agents how to use iris effectively. Files are never
//! overwritten — only missing files are created (idempotent).

use std::path::Path;

use tracing::{debug, info};

use crate::code::bridge::detector::FrameworkDetector;

/// Scaffold agent configuration files in the given project root.
///
/// Creates `.claude/rules/` with iris tool guides, scope rules, and a
/// project-aware playbook. Skips any file that already exists.
///
/// Returns the number of files created (0 if everything was already in place).
pub fn scaffold_agent_config(project_root: &Path) -> usize {
    let rules_dir = project_root.join(".claude").join("rules");

    let playbook = playbook_for_project(project_root);

    let files: &[(&str, &str)] = &[
        ("iris-scope.md", IRIS_SCOPE),
        ("tools.md", TOOLS),
        ("iris-playbook.md", playbook),
    ];

    let mut created = 0;

    for &(filename, content) in files {
        let path = rules_dir.join(filename);
        if path.exists() {
            debug!(file = %path.display(), "already exists, skipping");
            continue;
        }

        // Create directory structure if needed.
        if let Err(e) = std::fs::create_dir_all(&rules_dir) {
            debug!(error = %e, "failed to create .claude/rules/");
            return created;
        }

        match std::fs::write(&path, content) {
            Ok(()) => {
                created += 1;
                debug!(file = %path.display(), "scaffolded agent config");
            }
            Err(e) => {
                debug!(file = %path.display(), error = %e, "failed to write");
            }
        }
    }

    if created > 0 {
        info!(
            files = created,
            dir = %rules_dir.display(),
            "scaffolded iris agent config"
        );
    }

    created
}

/// Choose the right playbook based on detected bridge frameworks.
fn playbook_for_project(root: &Path) -> &'static str {
    let kinds = FrameworkDetector::detect(root);

    if kinds.iter().any(|k| {
        matches!(
            k,
            crate::code::bridge::BridgeKind::TauriCommand
                | crate::code::bridge::BridgeKind::TauriEvent
        )
    }) {
        return PLAYBOOK_TAURI;
    }

    if kinds.iter().any(|k| {
        matches!(
            k,
            crate::code::bridge::BridgeKind::PyO3
                | crate::code::bridge::BridgeKind::Napi
                | crate::code::bridge::BridgeKind::WasmBindgen
        )
    }) {
        return PLAYBOOK_BRIDGE;
    }

    PLAYBOOK_BASIC
}

// ---------------------------------------------------------------------------
// Embedded templates
// ---------------------------------------------------------------------------

/// Mandatory tool scope rules — always the same regardless of project type.
const IRIS_SCOPE: &str = r#"# iris MCP — Codebase Navigation

iris is the **recommended** tool for all codebase exploration.

## Tool Rules

| Tool                              | Status         | Usage                                                                         |
| --------------------------------- | -------------- | ----------------------------------------------------------------------------- |
| `iris_survey(query: "...")`       | **PRIMARY**    | Semantic search across docs and code. Start here.                             |
| `iris_symbols(query: "...")`      | **PRIMARY**    | Find structs, functions, traits, enums by name/kind/module.                   |
| `iris_definition(id: "...")`      | **PRIMARY**    | Get full source of a symbol by ID.                                            |
| `iris_references(id: "...")`      | **PRIMARY**    | Find callers, implementors, importers of a symbol.                            |
| `iris_read(id: "...")`            | **PRIMARY**    | Read a section by ID (with deduplication and delta delivery).                 |
| `iris_extract(id: "...")`         | **PRIMARY**    | Get atomic claims from a section, optionally filtered by query.               |
| `iris_toc`                        | **PRIMARY**    | Structural overview of the indexed corpus.                                    |
| `iris_bridge(query/kind/...)`     | **PRIMARY**    | Cross-language bridge links (Tauri, PyO3, NAPI, etc.).                        |
| `Read(file)`                      | **RESTRICTED** | Prefer iris tools for exploration. Use `Read` only immediately before `Edit`. |

## Workflow

1. **`iris_survey` first** — semantic search across docs and code. Always start here.
2. **`iris_symbols` for code navigation** — find symbols by name, kind, or module.
3. **`iris_definition` / `iris_read`** — get full source of a symbol or section.
4. **`iris_references` before modifying shared code** — find callers, implementors, importers.
5. **`iris_bridge` before modifying any cross-language boundary** — see all endpoints.
6. **`iris_toc`** — structural overview when you need to understand project layout.

See `iris-playbook.md` for detailed decision trees and chaining patterns.
"#;

/// Tool reference table — iris + common workflow tools.
const TOOLS: &str = r"# Tool Guide

## Codebase Navigation (iris)

| Tool | Purpose |
|------|---------|
| `iris_survey` | Semantic search across docs and code. Start here. |
| `iris_symbols` | Find structs, functions, traits, enums by name/kind/module. |
| `iris_definition` | Get full source of a symbol by ID. |
| `iris_references` | Find callers, implementors, importers of a symbol. |
| `iris_read` | Read a section by ID (with deduplication and delta delivery). |
| `iris_extract` | Get atomic claims from a section, optionally filtered by query. |
| `iris_toc` | Structural overview of the indexed corpus. |
| `iris_bridge` | Cross-language bridge links. **Use before changing any IPC/FFI boundary.** |

Recommended workflow: `iris_survey` → `iris_symbols` → `iris_definition` / `iris_read` → dig deeper with `iris_references` / `iris_bridge`.

See `iris-playbook.md` for decision trees and chaining patterns.

## Tool Preferences

- Use `iris_survey` instead of Glob/find for file and concept discovery.
- Use `iris_symbols` instead of Grep for finding code symbols.
- Use iris tools for exploration; `Read` only immediately before `Edit`.
";

/// Playbook for Tauri projects (Rust backend + JS/TS frontend).
const PLAYBOOK_TAURI: &str = r#"# iris Playbook

Decision guide for using iris tools effectively in this project.

## Decision Tree

### "I need to understand something"

- **Vague question** → `iris_survey(query: "natural language question")`
- **Know the symbol name** → `iris_symbols(query: "name")` → `iris_definition(symbol_id)`
- **Know the file** → `iris_toc(document_id: "path")` → `iris_read(section_id)`
- **Need project layout** → `iris_toc(limit: 100)`

### "I need to change something"

- **Before touching shared code:**
  1. `iris_references(symbol_id)` — who calls it?
  2. `iris_bridge(query: "name")` — does it cross a language boundary?
  3. Only then `Read` → `Edit`

- **Before changing a Tauri command:**
  1. `iris_bridge(query: "command_name")` — get ALL Rust↔TS endpoints
  2. This shows: the Rust export, the TS binding, the store callsite, and test mocks
  3. Change all of them in one pass — don't discover broken callsites one at a time

### "I need to find something"

- **A concept** → `iris_survey`
- **A specific symbol** → `iris_symbols`
- **All symbols of a kind** → `iris_symbols(kind: "struct")` or `iris_symbols(module: "commands")`

## The Bridge Rule

This is a Tauri project. Every feature spans Rust and TypeScript. **Always check `iris_bridge` before modifying any Tauri command.**

| Situation | Call |
|-----------|------|
| Changing command params/return type | `iris_bridge(query: "command_name")` |
| Renaming a command | `iris_bridge(query: "old_name")` — update every endpoint |
| Auditing IPC surface | `iris_bridge(bridge_kind: "tauri_command")` |
| Checking test coverage for a command | `iris_bridge(query: "name")` — look for test file imports |

## Anti-Patterns

- **Don't `Read` to explore.** Use `iris_read` or `iris_definition`.
- **Don't change a Tauri command without `iris_bridge`.** You WILL miss a callsite.
- **Don't grep for string matches across languages.** `iris_bridge` has semantic links.
- **Don't skip `iris_references` before modifying shared code.**
"#;

/// Playbook for cross-language projects with bridge frameworks (`PyO3`, NAPI, etc.).
const PLAYBOOK_BRIDGE: &str = r#"# iris Playbook

Decision guide for using iris tools effectively in this project.

## Decision Tree

### "I need to understand something"

- **Vague question** → `iris_survey(query: "natural language question")`
- **Know the symbol name** → `iris_symbols(query: "name")` → `iris_definition(symbol_id)`
- **Know the file** → `iris_toc(document_id: "path")` → `iris_read(section_id)`
- **Need project layout** → `iris_toc(limit: 100)`

### "I need to change something"

- **Before touching shared code:**
  1. `iris_references(symbol_id)` — who calls it?
  2. `iris_bridge(query: "name")` — does it cross a language boundary?
  3. Only then `Read` → `Edit`

- **Before changing an exported binding (pyclass, pyfunction, napi, wasm_bindgen):**
  1. `iris_bridge(query: "binding_name")` — see all cross-language endpoints
  2. Update both the native export and the language-side import together

### "I need to find something"

- **A concept** → `iris_survey`
- **A specific symbol** → `iris_symbols`
- **Cross-language links** → `iris_bridge(bridge_kind: "pyo3")` (or `napi`, `wasm_bindgen`)

## The Bridge Rule

This project has cross-language bindings. **Always check `iris_bridge` before modifying any exported binding.**

## Anti-Patterns

- **Don't `Read` to explore.** Use `iris_read` or `iris_definition`.
- **Don't change an exported binding without `iris_bridge`.** You'll miss consumers.
- **Don't grep for string matches across languages.** `iris_bridge` has semantic links.
- **Don't skip `iris_references` before modifying shared code.**
"#;

/// Playbook for single-language projects (no bridge frameworks detected).
const PLAYBOOK_BASIC: &str = r#"# iris Playbook

Decision guide for using iris tools effectively in this project.

## Decision Tree

### "I need to understand something"

- **Vague question** → `iris_survey(query: "natural language question")`
- **Know the symbol name** → `iris_symbols(query: "name")` → `iris_definition(symbol_id)`
- **Know the file** → `iris_toc(document_id: "path")` → `iris_read(section_id)`
- **Need project layout** → `iris_toc(limit: 100)`

### "I need to change something"

- **Before touching shared code:**
  1. `iris_references(symbol_id)` — who calls it? who imports it?
  2. Only then `Read` → `Edit`

- **Before deleting code:**
  1. `iris_references(symbol_id)` — is anything still using it?
  2. Zero references = safe to delete

### "I need to find something"

- **A concept** → `iris_survey`
- **A specific symbol** → `iris_symbols`
- **All symbols of a kind** → `iris_symbols(kind: "struct")` or `iris_symbols(module: "name")`

## Anti-Patterns

- **Don't `Read` to explore.** Use `iris_read` or `iris_definition`.
- **Don't skip `iris_references` before modifying shared code.**
"#;
