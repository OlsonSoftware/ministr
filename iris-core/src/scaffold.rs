//! First-run scaffolding for agent configuration files.
//!
//! When iris starts in a repo for the first time, it generates configuration
//! files that teach AI agents how to use iris effectively:
//!
//! - `.claude/rules/` — Claude Code tool rules, scope, and playbook
//! - `.claude/settings.json` — PreToolUse hooks that redirect Grep/Glob to iris
//! - `.cursor/rules/iris.mdc` — Cursor IDE rules
//! - `.github/copilot-instructions.md` — GitHub Copilot instructions
//! - `AGENTS.md` — Universal agent instructions
//!
//! Files are never overwritten — only missing files are created (idempotent).

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
    let playbook = playbook_for_project(project_root);

    let mut created = 0;

    // ── Claude Code: .claude/rules/ ─────────────────────────────────────
    let claude_rules_dir = project_root.join(".claude").join("rules");
    let claude_rules: &[(&str, &str)] = &[
        ("iris-scope.md", IRIS_SCOPE),
        ("tools.md", TOOLS),
        ("iris-playbook.md", playbook),
    ];
    created += write_files(&claude_rules_dir, claude_rules);

    // ── Claude Code: .claude/settings.json (PreToolUse hooks) ───────────
    created += write_claude_hooks(project_root);

    // ── Cursor: .cursor/rules/ ──────────────────────────────────────────
    let cursor_rules_dir = project_root.join(".cursor").join("rules");
    let cursor_rules: &[(&str, &str)] = &[("iris.mdc", CURSOR_RULES)];
    created += write_files(&cursor_rules_dir, cursor_rules);

    // ── GitHub Copilot: .github/copilot-instructions.md ─────────────────
    let github_dir = project_root.join(".github");
    let copilot_files: &[(&str, &str)] =
        &[("copilot-instructions.md", COPILOT_INSTRUCTIONS)];
    created += write_files(&github_dir, copilot_files);

    // ── Universal: AGENTS.md ────────────────────────────────────────────
    let agents_files: &[(&str, &str)] = &[("AGENTS.md", AGENTS_MD)];
    created += write_files(project_root, agents_files);

    if created > 0 {
        info!(
            files = created,
            root = %project_root.display(),
            "scaffolded iris agent config"
        );
    }

    created
}

/// Write a set of files into a directory. Creates the directory if needed.
/// Skips files that already exist. Returns the number of files created.
fn write_files(dir: &Path, files: &[(&str, &str)]) -> usize {
    let mut created = 0;
    for &(filename, content) in files {
        let path = dir.join(filename);
        if path.exists() {
            debug!(file = %path.display(), "already exists, skipping");
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            debug!(error = %e, dir = %dir.display(), "failed to create directory");
            return created;
        }
        match std::fs::write(&path, content) {
            Ok(()) => {
                created += 1;
                debug!(file = %path.display(), "scaffolded");
            }
            Err(e) => {
                debug!(file = %path.display(), error = %e, "failed to write");
            }
        }
    }
    created
}

/// Write `.claude/settings.json` with `PreToolUse` hooks that redirect
/// Grep/Glob usage to iris. Merges non-destructively — only adds the
/// hooks key if not already present.
fn write_claude_hooks(project_root: &Path) -> usize {
    let settings_path = project_root.join(".claude").join("settings.json");

    // Don't overwrite if the file already has hooks configured.
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if val.get("hooks").is_some() {
                    debug!(
                        file = %settings_path.display(),
                        "hooks already configured, skipping"
                    );
                    return 0;
                }
            }
        }
    }

    let settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Grep|Glob",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "printf '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Use iris_survey or iris_symbols instead. iris provides semantic code search with better results. See .claude/rules/iris-scope.md for the full tool guide.\"}}'"
                        }
                    ]
                }
            ]
        }
    });

    // Merge with existing settings if the file exists.
    let merged = if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut existing) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert("hooks".to_string(), settings["hooks"].clone());
                }
                existing
            } else {
                settings
            }
        } else {
            settings
        }
    } else {
        settings
    };

    if let Err(e) = std::fs::create_dir_all(settings_path.parent().unwrap_or(project_root)) {
        debug!(error = %e, "failed to create .claude/");
        return 0;
    }

    let json_str = serde_json::to_string_pretty(&merged).unwrap_or_default();
    match std::fs::write(&settings_path, format!("{json_str}\n")) {
        Ok(()) => {
            debug!(file = %settings_path.display(), "wrote Claude Code hooks");
            1
        }
        Err(e) => {
            debug!(file = %settings_path.display(), error = %e, "failed to write");
            0
        }
    }
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

// ---------------------------------------------------------------------------
// Cursor rules
// ---------------------------------------------------------------------------

/// Cursor IDE rules file (`.cursor/rules/iris.mdc`).
///
/// Cursor uses MDC (Markdown Configuration) format — standard markdown with
/// optional YAML frontmatter. The `description` and `globs` fields control
/// when the rules are shown to the agent.
const CURSOR_RULES: &str = r#"---
description: iris MCP codebase navigation tools — use instead of built-in search
globs:
  - "**/*"
---

# iris MCP — Codebase Navigation

iris is an MCP server providing semantic code search. **Always prefer iris tools over built-in search.**

## Available Tools

| Tool | Purpose |
|------|---------|
| `iris_survey` | Semantic search across docs and code. **Start here.** |
| `iris_symbols` | Find structs, functions, traits, enums by name/kind/module. |
| `iris_definition` | Get full source of a symbol by ID. |
| `iris_references` | Find callers, implementors, importers of a symbol. |
| `iris_read` | Read a section by ID (with deduplication and delta delivery). |
| `iris_extract` | Get atomic claims from a section, optionally filtered by query. |
| `iris_toc` | Structural overview of the indexed corpus. |
| `iris_bridge` | Cross-language bridge links (Tauri, PyO3, NAPI, etc.). |

## Rules

1. **Use `iris_survey` instead of file search or grep** for discovering code and documentation.
2. **Use `iris_symbols` instead of grep** for finding functions, structs, traits, or enums.
3. **Use `iris_definition` instead of reading whole files** to get a symbol's source.
4. **Use `iris_references` before modifying any shared symbol** to find all callers.
5. **Use `iris_bridge` before modifying any cross-language boundary** (Tauri commands, FFI, etc.).

## Workflow

`iris_survey` → `iris_symbols` → `iris_definition` / `iris_read` → `iris_references` / `iris_bridge`
"#;

// ---------------------------------------------------------------------------
// GitHub Copilot instructions
// ---------------------------------------------------------------------------

/// GitHub Copilot custom instructions (`.github/copilot-instructions.md`).
///
/// Used by Copilot CLI, Copilot Chat in VS Code, and Copilot cloud agent.
const COPILOT_INSTRUCTIONS: &str = r#"# iris MCP — Codebase Navigation

This project uses [iris](https://github.com/iris-rs/iris) as an MCP server for semantic code search.

## Tool Preferences

**Always prefer iris MCP tools over built-in search tools** (grep, glob, file reading for exploration):

| Instead of… | Use… |
|-------------|------|
| `grep` / text search | `iris_survey(query: "...")` — semantic search across docs and code |
| File listing / glob | `iris_toc` — structural overview of the indexed corpus |
| Reading a file to find symbols | `iris_symbols(query: "name")` — find by name/kind/module |
| Reading a file for a specific function | `iris_definition(symbol_id: "...")` — get full source |

## Workflow

1. **Start with `iris_survey`** for any question about the codebase
2. **Use `iris_symbols`** to find specific code symbols
3. **Use `iris_definition` or `iris_read`** to get full source
4. **Use `iris_references`** before modifying shared code (find all callers)
5. **Use `iris_bridge`** before modifying cross-language boundaries (Tauri, FFI, etc.)

## Key Rules

- Only use file `Read` immediately before `Edit` — for everything else, use iris tools
- Always check `iris_references` before modifying or deleting shared code
- If the project has cross-language bindings, check `iris_bridge` before changing IPC/FFI boundaries
"#;

// ---------------------------------------------------------------------------
// AGENTS.md (universal)
// ---------------------------------------------------------------------------

/// Universal agent instructions (`AGENTS.md`).
///
/// Supported by GitHub Copilot CLI (via `AGENTS.md` at project root) and
/// other agents that follow the agents.md convention.
const AGENTS_MD: &str = r#"# Agent Instructions

This project uses **iris** as an MCP server for semantic code search and navigation.
All AI agents working on this codebase should prefer iris tools over built-in alternatives.

## MCP Server: iris

iris is automatically configured via `.mcp.json` (Claude Code), `.vscode/mcp.json` (VS Code / Copilot), and `.cursor/mcp.json` (Cursor).

### Tool Reference

| Tool | Purpose |
|------|---------|
| `iris_survey(query)` | Semantic search across docs and code. **Start here.** |
| `iris_symbols(query)` | Find structs, functions, traits, enums by name/kind/module. |
| `iris_definition(symbol_id)` | Get full source of a symbol by ID. |
| `iris_references(symbol_id)` | Find callers, implementors, importers of a symbol. |
| `iris_read(section_id)` | Read a section by ID. |
| `iris_extract(section_id)` | Get atomic claims from a section. |
| `iris_toc` | Structural overview of the indexed corpus. |
| `iris_bridge(query)` | Cross-language bridge links (Tauri, PyO3, NAPI, etc.). |

### Preferred Over Built-in Tools

| Instead of… | Use… |
|-------------|------|
| Grep / text search | `iris_survey` |
| Glob / file listing | `iris_toc` |
| Reading files for exploration | `iris_symbols` → `iris_definition` |
| Finding references manually | `iris_references` |

### Workflow

1. `iris_survey` → understand concepts, find relevant code
2. `iris_symbols` → locate specific symbols
3. `iris_definition` / `iris_read` → get full source
4. `iris_references` → check impact before modifying
5. `iris_bridge` → check cross-language boundaries
6. Only then: `Read` → `Edit`
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scaffold_creates_all_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let created = scaffold_agent_config(root);

        // Should create: 3 claude rules + 1 settings.json + 1 cursor rule
        //   + 1 copilot instructions + 1 AGENTS.md = 7
        assert_eq!(created, 7);

        // Claude Code files
        assert!(root.join(".claude/rules/iris-scope.md").exists());
        assert!(root.join(".claude/rules/tools.md").exists());
        assert!(root.join(".claude/rules/iris-playbook.md").exists());
        assert!(root.join(".claude/settings.json").exists());

        // Cursor files
        assert!(root.join(".cursor/rules/iris.mdc").exists());

        // Copilot files
        assert!(root.join(".github/copilot-instructions.md").exists());

        // Universal
        assert!(root.join("AGENTS.md").exists());

        // Verify Claude hooks contain PreToolUse
        let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(val["hooks"]["PreToolUse"].is_array());
    }

    #[test]
    fn scaffold_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let first = scaffold_agent_config(root);
        assert_eq!(first, 7);

        let second = scaffold_agent_config(root);
        assert_eq!(second, 0);
    }

    #[test]
    fn scaffold_does_not_overwrite_existing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create a file with custom content.
        std::fs::create_dir_all(root.join(".claude/rules")).unwrap();
        std::fs::write(root.join(".claude/rules/tools.md"), "custom content").unwrap();

        scaffold_agent_config(root);

        // Should not overwrite.
        let content = std::fs::read_to_string(root.join(".claude/rules/tools.md")).unwrap();
        assert_eq!(content, "custom content");
    }

    #[test]
    fn claude_hooks_merge_with_existing_settings() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create settings with existing content.
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"permissions": {"allow": ["Bash(cargo test)"]}}"#,
        )
        .unwrap();

        write_claude_hooks(root);

        let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&settings).unwrap();

        // Should have both hooks and permissions.
        assert!(val["hooks"]["PreToolUse"].is_array());
        assert!(val["permissions"]["allow"].is_array());
    }

    #[test]
    fn claude_hooks_skip_when_hooks_exist() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create settings with existing hooks.
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks": {"PostToolUse": []}}"#,
        )
        .unwrap();

        let created = write_claude_hooks(root);
        assert_eq!(created, 0); // Should skip — hooks already present.
    }
}
