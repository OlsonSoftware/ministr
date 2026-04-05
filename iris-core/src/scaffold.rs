//! First-run scaffolding for agent configuration files.
//!
//! When iris starts in a repo for the first time, it generates configuration
//! files that teach AI agents how to use iris effectively:
//!
//! - `.claude/rules/` — Claude Code tool rules, scope, and playbook
//! - `.claude/settings.json` — PreToolUse hooks that redirect Grep/Glob to iris
//! - `.cursor/rules/iris.mdc` — Cursor IDE rules
//! - `.cursor/hooks.json` — Cursor hooks (blocks shell search/find/pipes)
//! - `.github/hooks/iris-enforce.json` — Copilot CLI + cloud agent hooks
//! - `.github/copilot-instructions.md` — GitHub Copilot instructions
//! - `AGENTS.md` — Universal agent instructions
//!
//! Files are never overwritten — only missing files are created.
//! Machine-generated hook files are auto-healed if their content is stale.

use std::path::Path;

use tracing::{debug, info};

use crate::code::bridge::detector::FrameworkDetector;

/// Result of a scaffold operation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScaffoldResult {
    /// Number of brand-new files written.
    pub created: usize,
    /// Number of existing files overwritten because their content was stale.
    pub healed: usize,
}

impl ScaffoldResult {
    fn merge(&mut self, other: Self) {
        self.created += other.created;
        self.healed += other.healed;
    }

    /// Total files touched (created + healed).
    #[must_use]
    pub fn touched(&self) -> usize {
        self.created + self.healed
    }
}

/// Scaffold agent configuration files in the given project root.
///
/// - Advisory files (`.md`, `.mdc`) are created if missing but never overwritten
///   (users may customise them).
/// - Machine-generated hook files (`.json`) are auto-healed: if the on-disk
///   content differs from the current template, the file is overwritten.
///
/// Returns a [`ScaffoldResult`] with created/healed counts.
pub fn scaffold_agent_config(project_root: &Path) -> ScaffoldResult {
    let playbook = playbook_for_project(project_root);
    let mut result = ScaffoldResult::default();

    // ── Claude Code: .claude/rules/ (advisory — never overwrite) ────────
    let claude_rules_dir = project_root.join(".claude").join("rules");
    let claude_rules: &[(&str, &str)] = &[
        ("iris-scope.md", IRIS_SCOPE),
        ("tools.md", TOOLS),
        ("iris-playbook.md", playbook),
    ];
    result.merge(write_files(&claude_rules_dir, claude_rules, false));

    // ── Claude Code + VS Code: .claude/settings.json (hooks — autoheal) ─
    result.merge(write_claude_hooks(project_root));

    // ── Copilot CLI: .github/hooks/ (hooks — autoheal) ──────────────────
    let hooks_dir = project_root.join(".github").join("hooks");
    let hooks_files: &[(&str, &str)] = &[("iris-enforce.json", COPILOT_HOOKS)];
    result.merge(write_files(&hooks_dir, hooks_files, true));

    // ── Cursor: .cursor/rules/ (advisory — never overwrite) ─────────────
    let cursor_rules_dir = project_root.join(".cursor").join("rules");
    let cursor_rules: &[(&str, &str)] = &[("iris.mdc", CURSOR_RULES)];
    result.merge(write_files(&cursor_rules_dir, cursor_rules, false));

    // ── Cursor: .cursor/hooks.json (hooks — autoheal) ───────────────────
    let cursor_dir = project_root.join(".cursor");
    let cursor_hooks: &[(&str, &str)] = &[("hooks.json", CURSOR_HOOKS)];
    result.merge(write_files(&cursor_dir, cursor_hooks, true));

    // ── GitHub Copilot: .github/copilot-instructions.md (advisory) ──────
    let github_dir = project_root.join(".github");
    let copilot_files: &[(&str, &str)] =
        &[("copilot-instructions.md", COPILOT_INSTRUCTIONS)];
    result.merge(write_files(&github_dir, copilot_files, false));

    // ── Universal: AGENTS.md (advisory) ─────────────────────────────────
    let agents_files: &[(&str, &str)] = &[("AGENTS.md", AGENTS_MD)];
    result.merge(write_files(project_root, agents_files, false));

    if result.touched() > 0 {
        info!(
            created = result.created,
            healed = result.healed,
            root = %project_root.display(),
            "scaffolded iris agent config"
        );
    }

    result
}

/// Write a set of files into a directory. Creates the directory if needed.
///
/// When `heal` is `false`, existing files are skipped (advisory content the
/// user may have customised). When `heal` is `true`, existing files whose
/// content doesn't match the template are overwritten (machine-generated
/// hooks that must stay in sync with the iris version).
fn write_files(dir: &Path, files: &[(&str, &str)], heal: bool) -> ScaffoldResult {
    let mut result = ScaffoldResult::default();
    for &(filename, content) in files {
        let path = dir.join(filename);
        if path.exists() {
            if heal {
                if let Ok(existing) = std::fs::read_to_string(&path) {
                    if existing.trim() == content.trim() {
                        debug!(file = %path.display(), "up to date");
                        continue;
                    }
                    // Content is stale — overwrite.
                    match std::fs::write(&path, content) {
                        Ok(()) => {
                            result.healed += 1;
                            info!(file = %path.display(), "healed stale hook file");
                        }
                        Err(e) => {
                            debug!(file = %path.display(), error = %e, "failed to heal");
                        }
                    }
                }
            } else {
                debug!(file = %path.display(), "already exists, skipping");
            }
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            debug!(error = %e, dir = %dir.display(), "failed to create directory");
            return result;
        }
        match std::fs::write(&path, content) {
            Ok(()) => {
                result.created += 1;
                debug!(file = %path.display(), "scaffolded");
            }
            Err(e) => {
                debug!(file = %path.display(), error = %e, "failed to write");
            }
        }
    }
    result
}

/// Write `.claude/settings.json` with `PreToolUse` hooks that redirect
/// Grep/Glob/Bash-search usage to iris.
///
/// Merges non-destructively with existing settings (preserves user keys).
/// Auto-heals: if the file already has a `hooks` key but the content
/// differs from what iris would generate, the `hooks` key is replaced.
fn write_claude_hooks(project_root: &Path) -> ScaffoldResult {
    let settings_path = project_root.join(".claude").join("settings.json");

    let hooks_value = build_claude_hooks();

    // If the file exists and already has our exact hooks, nothing to do.
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                if val.get("hooks") == Some(&hooks_value["hooks"]) {
                    debug!(file = %settings_path.display(), "hooks up to date");
                    return ScaffoldResult::default();
                }
            }
        }
    }

    let is_heal = settings_path.exists();

    // Merge with existing settings (preserves user keys like "permissions").
    let merged = if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut existing) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(obj) = existing.as_object_mut() {
                    obj.insert("hooks".to_string(), hooks_value["hooks"].clone());
                }
                existing
            } else {
                hooks_value
            }
        } else {
            hooks_value
        }
    } else {
        hooks_value
    };

    if let Err(e) = std::fs::create_dir_all(settings_path.parent().unwrap_or(project_root)) {
        debug!(error = %e, "failed to create .claude/");
        return ScaffoldResult::default();
    }

    let json_str = serde_json::to_string_pretty(&merged).unwrap_or_default();
    match std::fs::write(&settings_path, format!("{json_str}\n")) {
        Ok(()) => {
            if is_heal {
                info!(file = %settings_path.display(), "healed stale Claude Code hooks");
                ScaffoldResult { created: 0, healed: 1 }
            } else {
                debug!(file = %settings_path.display(), "wrote Claude Code hooks");
                ScaffoldResult { created: 1, healed: 0 }
            }
        }
        Err(e) => {
            debug!(file = %settings_path.display(), error = %e, "failed to write");
            ScaffoldResult::default()
        }
    }
}

/// Build the hooks JSON value for `.claude/settings.json`.
fn build_claude_hooks() -> serde_json::Value {
    let deny_search = "Use iris_survey or iris_symbols instead of shell search tools. \
        iris provides semantic code search with better results. \
        See .claude/rules/iris-scope.md for the full tool guide.";
    let deny_files = "Use iris_toc or iris_survey instead of shell file-finding tools.";
    let deny_pipe = "Don't pipe to search/filter tools for code exploration. \
        Use iris_survey for search, iris_toc for structure, iris_read for content.";

    let mut bash_hooks: Vec<serde_json::Value> = Vec::new();

    for cmd in &["grep", "egrep", "fgrep", "rg", "ag", "ack"] {
        bash_hooks.push(deny_hook(&format!("Bash({cmd} *)"), deny_search));
    }
    for cmd in &["find", "fd"] {
        bash_hooks.push(deny_hook(&format!("Bash({cmd} *)"), deny_files));
    }
    for cmd in &["grep", "rg", "ag", "ack"] {
        bash_hooks.push(deny_hook(&format!("Bash(*|*{cmd} *)"), deny_pipe));
    }
    for cmd in &["wc", "head", "tail"] {
        bash_hooks.push(deny_hook(&format!("Bash(*|*{cmd}*)"), deny_pipe));
    }

    serde_json::json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Grep|Glob",
                    "hooks": [deny_hook("", deny_search)]
                },
                {
                    "matcher": "Bash",
                    "hooks": bash_hooks
                }
            ]
        }
    })
}

/// Build a single PreToolUse deny hook entry.
///
/// When the `if_pattern` matches a tool invocation, the hook returns a JSON
/// deny decision with the given `reason`. If `if_pattern` is empty, the hook
/// fires for all invocations matching the parent matcher.
fn deny_hook(if_pattern: &str, reason: &str) -> serde_json::Value {
    // Escape quotes in the reason for the printf JSON string.
    let escaped = reason.replace('"', r#"\""#);
    let printf_json = format!(
        "printf '{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\
         \"permissionDecision\":\"deny\",\
         \"permissionDecisionReason\":\"{escaped}\"}}}}'",
    );

    let mut hook = serde_json::json!({
        "type": "command",
        "command": printf_json
    });

    if !if_pattern.is_empty() {
        hook.as_object_mut()
            .unwrap()
            .insert("if".to_string(), serde_json::Value::String(if_pattern.to_string()));
    }

    hook
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

/// Copilot CLI / cloud agent hooks (`.github/hooks/iris-enforce.json`).
///
/// Copilot CLI reads `.github/hooks/*.json` with `"version": 1` format.
/// VS Code Copilot also reads these files (and `.claude/settings.json`).
/// Uses camelCase event names and bash/powershell keys per GitHub docs.
///
/// The preToolUse hook inspects toolName and toolArgs to block search/exploration
/// tools and redirect to iris MCP tools.
const COPILOT_HOOKS: &str = r#"{
  "version": 1,
  "hooks": {
    "preToolUse": [
      {
        "type": "command",
        "bash": "INPUT=$(cat); TN=$(echo \"$INPUT\" | jq -r '.toolName'); TA=$(echo \"$INPUT\" | jq -r '.toolArgs // \"\"'); case \"$TN\" in grep|Grep) echo '{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Use iris_survey instead of grep. iris provides semantic code search.\"}'; exit 0;; glob|Glob) echo '{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Use iris_toc instead of glob. iris provides structural overview.\"}'; exit 0;; bash|Bash|shell) CMD=$(echo \"$TA\" | jq -r '.command // \"\"'); case \"$CMD\" in grep\\ *|egrep\\ *|fgrep\\ *|rg\\ *|ag\\ *|ack\\ *) echo '{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Use iris_survey instead of shell search commands.\"}'; exit 0;; find\\ *|fd\\ *) echo '{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Use iris_toc instead of shell file-finding commands.\"}'; exit 0;; esac; if echo \"$CMD\" | grep -qE '\\|\\s*(grep|rg|ag|ack|head|tail|wc)'; then echo '{\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"Do not pipe to search/filter tools. Use iris_survey, iris_toc, or iris_read.\"}'; exit 0; fi;; esac",
        "powershell": "$input = [Console]::In.ReadToEnd() | ConvertFrom-Json; $tn = $input.toolName; $ta = if ($input.toolArgs) { $input.toolArgs } else { '' }; $blocked = @('grep','Grep','glob','Glob'); if ($blocked -contains $tn) { @{permissionDecision='deny'; permissionDecisionReason='Use iris MCP tools instead of built-in search.'} | ConvertTo-Json -Compress; exit 0 }; if ($tn -in @('bash','Bash','shell')) { $cmd = ($ta | ConvertFrom-Json).command; if ($cmd -match '^(grep|egrep|fgrep|rg|ag|ack|find|fd)\\s') { @{permissionDecision='deny'; permissionDecisionReason='Use iris MCP tools instead of shell search.'} | ConvertTo-Json -Compress; exit 0 }; if ($cmd -match '\\|\\s*(grep|rg|ag|ack|head|tail|wc)') { @{permissionDecision='deny'; permissionDecisionReason='Do not pipe to search/filter tools. Use iris tools.'} | ConvertTo-Json -Compress; exit 0 } }",
        "timeoutSec": 5
      }
    ]
  }
}
"#;

/// Cursor hooks (`.cursor/hooks.json`).
///
/// Cursor reads `.cursor/hooks.json` (project), `~/.cursor/hooks.json` (user).
/// Uses `beforeShellExecution` to block grep/rg/find/fd and piped exploration.
/// Cursor's hook events differ from Claude Code / Copilot CLI:
/// - `beforeShellExecution` — fires before any shell command
/// - `beforeReadFile` — fires before reading files (informational only here)
/// - No generic "preToolUse" — built-in tools like grep/glob aren't shell commands
///   in Cursor, so we rely on `.cursor/rules/iris.mdc` for those.
const CURSOR_HOOKS: &str = r#"{
  "version": 1,
  "hooks": {
    "beforeShellExecution": [
      {
        "command": "bash -c 'INPUT=$(cat); CMD=$(echo \"$INPUT\" | jq -r \".command // \\\"\\\"\"); case \"$CMD\" in grep\\ *|egrep\\ *|fgrep\\ *|rg\\ *|ag\\ *|ack\\ *) echo \"{\\\"permission\\\":\\\"deny\\\",\\\"agentMessage\\\":\\\"Use iris_survey instead of shell search. iris provides semantic code search.\\\",\\\"userMessage\\\":\\\"Blocked: shell search command. Use iris_survey.\\\"}\"; exit 0;; find\\ *|fd\\ *) echo \"{\\\"permission\\\":\\\"deny\\\",\\\"agentMessage\\\":\\\"Use iris_toc instead of shell file-finding. iris provides structural overview.\\\",\\\"userMessage\\\":\\\"Blocked: shell file-find. Use iris_toc.\\\"}\"; exit 0;; esac; if echo \"$CMD\" | grep -qE \"\\\\|\\\\s*(grep|rg|ag|ack|head|tail|wc)\"; then echo \"{\\\"permission\\\":\\\"deny\\\",\\\"agentMessage\\\":\\\"Do not pipe to search/filter tools. Use iris_survey, iris_toc, or iris_read.\\\",\\\"userMessage\\\":\\\"Blocked: piped exploration. Use iris tools.\\\"}\"; exit 0; fi'"
      }
    ]
  }
}
"#;

/// Mandatory tool scope rules — always the same regardless of project type.
const IRIS_SCOPE: &str = r#"# iris MCP — Codebase Navigation

iris is the **required** tool for all codebase exploration. Do NOT use built-in search tools.

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
| `Grep` / `Glob`                   | **BLOCKED**    | Denied by PreToolUse hook. Use iris_survey or iris_symbols instead.           |
| `Bash(grep/rg/find/...)`          | **BLOCKED**    | Denied by PreToolUse hook. Do NOT shell out for search or file discovery.     |
| `Bash(... \| grep/head/tail/wc)`  | **BLOCKED**    | Denied by PreToolUse hook. Do NOT pipe to search/filter tools.               |
| `Read(file)`                      | **RESTRICTED** | Use `Read` only immediately before `Edit`. Never for exploration.             |

## Prohibited Patterns

These are **hard-blocked** by PreToolUse hooks and will be denied:

- `grep`, `rg`, `ag`, `ack`, `egrep`, `fgrep` — use `iris_survey` instead
- `find`, `fd` — use `iris_toc` or `iris_survey` instead
- `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` — use iris tools instead
- `Grep(pattern)`, `Glob(pattern)` — use `iris_survey` or `iris_symbols` instead

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
description: iris MCP codebase navigation — REQUIRED for all code search and exploration
globs:
  - "**/*"
---

# iris MCP — Codebase Navigation (MANDATORY)

iris is an MCP server providing semantic code search. **You MUST use iris tools for ALL codebase exploration.**

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

## PROHIBITED — Do NOT Use These for Exploration

**NEVER use these tools for code discovery, search, or exploration:**

- ❌ `grep`, `rg`, `ripgrep`, `ag`, `ack` — use `iris_survey` instead
- ❌ `find`, `fd`, `ls -R`, directory traversal — use `iris_toc` instead
- ❌ `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` — use iris tools
- ❌ Built-in file search / text search / Grep tool — use `iris_survey`
- ❌ Built-in Glob / file listing — use `iris_toc`
- ❌ Reading files to discover structure — use `iris_toc` or `iris_symbols`
- ❌ Shell/Bash/Terminal for ANY search, file discovery, or piped exploration

The ONLY acceptable use of file Read is immediately before Edit.
The ONLY acceptable use of Shell/Bash is for building, testing, and running commands (not exploration).

## Rules

1. **Use `iris_survey` instead of ANY search** for discovering code and documentation.
2. **Use `iris_symbols` instead of grep** for finding functions, structs, traits, or enums.
3. **Use `iris_definition` instead of reading whole files** to get a symbol's source.
4. **Use `iris_references` before modifying any shared symbol** to find all callers.
5. **Use `iris_bridge` before modifying any cross-language boundary** (Tauri commands, FFI, etc.).
6. **NEVER use Bash/Shell for code search, file discovery, or piped exploration.**

## Workflow

`iris_survey` → `iris_symbols` → `iris_definition` / `iris_read` → `iris_references` / `iris_bridge`
"#;

// ---------------------------------------------------------------------------
// GitHub Copilot instructions
// ---------------------------------------------------------------------------

/// GitHub Copilot custom instructions (`.github/copilot-instructions.md`).
///
/// Used by Copilot CLI, Copilot Chat in VS Code, and Copilot cloud agent.
const COPILOT_INSTRUCTIONS: &str = r#"# iris MCP — Codebase Navigation (MANDATORY)

This project uses [iris](https://github.com/iris-rs/iris) as an MCP server for semantic code search.

## CRITICAL: Tool Restrictions

**You MUST use iris MCP tools for ALL codebase exploration.** The following are PROHIBITED:

### Prohibited Tools and Patterns

- ❌ **Grep tool** — use `iris_survey(query: "...")` instead
- ❌ **Glob tool** — use `iris_toc` instead
- ❌ **`grep`**, **`rg`**, **`ag`**, **`ack`** in Bash/Shell — use `iris_survey` instead
- ❌ **`find`**, **`fd`**, **`ls -R`** in Bash/Shell — use `iris_toc` instead
- ❌ **`cat | grep`**, **`cmd | head`**, **`cmd | tail`**, **`cmd | wc`** — use iris tools
- ❌ **ANY piped shell command** for code exploration — use iris tools
- ❌ **Reading files** for exploration — use `iris_symbols` → `iris_definition`

### Allowed Uses of Shell/Bash

Shell is ONLY acceptable for: building code, running tests, installing dependencies, git operations, and running the project. NEVER for searching, file discovery, or piped exploration.

### Allowed Uses of file Read

File Read is ONLY acceptable immediately before Edit — never for exploration or discovery.

## Required Tool Mapping

| Instead of… | Use… |
|-------------|------|
| `grep` / `Grep` / text search | `iris_survey(query: "...")` — semantic search across docs and code |
| `find` / `Glob` / file listing | `iris_toc` — structural overview of the indexed corpus |
| Reading a file to find symbols | `iris_symbols(query: "name")` — find by name/kind/module |
| Reading a file for a specific function | `iris_definition(symbol_id: "...")` — get full source |
| Checking who calls a function | `iris_references(symbol_id: "...")` — find all callers |

## Workflow

1. **Start with `iris_survey`** for any question about the codebase
2. **Use `iris_symbols`** to find specific code symbols
3. **Use `iris_definition` or `iris_read`** to get full source
4. **Use `iris_references`** before modifying shared code (find all callers)
5. **Use `iris_bridge`** before modifying cross-language boundaries (Tauri, FFI, etc.)
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
All AI agents working on this codebase **MUST** use iris tools instead of built-in alternatives.

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

### PROHIBITED — Do NOT Use for Exploration

**These are BLOCKED and must NEVER be used for code discovery or search:**

- ❌ `grep`, `rg`, `ripgrep`, `ag`, `ack`, `egrep`, `fgrep` → use `iris_survey`
- ❌ `find`, `fd`, `ls -R`, `tree`, directory listing → use `iris_toc`
- ❌ `cat file | grep`, `cmd | head`, `cmd | tail`, `cmd | wc` → use iris tools
- ❌ Built-in Grep/Glob tools → use `iris_survey` / `iris_toc`
- ❌ Reading files for exploration → use `iris_symbols` → `iris_definition`
- ❌ Any Shell/Bash/Terminal command for search or file discovery

**Allowed uses of Shell/Bash:** building, testing, git, installing dependencies, running the project.
**Allowed uses of file Read:** only immediately before Edit — never for exploration.

### Required Tool Mapping

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

        let result = scaffold_agent_config(root);

        // Should create: 3 claude rules + 1 settings.json + 1 copilot hooks
        //   + 1 cursor rule + 1 cursor hooks + 1 copilot instructions + 1 AGENTS.md = 9
        assert_eq!(result.created, 9);
        assert_eq!(result.healed, 0);

        // Claude Code files
        assert!(root.join(".claude/rules/iris-scope.md").exists());
        assert!(root.join(".claude/rules/tools.md").exists());
        assert!(root.join(".claude/rules/iris-playbook.md").exists());
        assert!(root.join(".claude/settings.json").exists());

        // Copilot CLI hooks
        assert!(root.join(".github/hooks/iris-enforce.json").exists());

        // Cursor files
        assert!(root.join(".cursor/rules/iris.mdc").exists());
        assert!(root.join(".cursor/hooks.json").exists());

        // Copilot files
        assert!(root.join(".github/copilot-instructions.md").exists());

        // Universal
        assert!(root.join("AGENTS.md").exists());

        // Verify Claude hooks contain PreToolUse with Bash matchers
        let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&settings).unwrap();
        let hooks = val["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(hooks.len() >= 2); // Grep|Glob + Bash matchers
        // Verify the Bash matcher has hooks with "if" patterns
        let bash_matcher = hooks.iter().find(|h| {
            h["matcher"].as_str() == Some("Bash")
        }).unwrap();
        assert!(bash_matcher["hooks"].as_array().unwrap().len() >= 6);

        // Verify Copilot CLI hooks contain preToolUse (camelCase) and version
        let copilot = std::fs::read_to_string(root.join(".github/hooks/iris-enforce.json")).unwrap();
        let cval: serde_json::Value = serde_json::from_str(&copilot).unwrap();
        assert_eq!(cval["version"], 1);
        assert!(cval["hooks"]["preToolUse"].is_array());

        // Verify Cursor hooks contain beforeShellExecution and version
        let cursor = std::fs::read_to_string(root.join(".cursor/hooks.json")).unwrap();
        let curval: serde_json::Value = serde_json::from_str(&cursor).unwrap();
        assert_eq!(curval["version"], 1);
        assert!(curval["hooks"]["beforeShellExecution"].is_array());
    }

    #[test]
    fn scaffold_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let first = scaffold_agent_config(root);
        assert_eq!(first.created, 9);
        assert_eq!(first.healed, 0);

        let second = scaffold_agent_config(root);
        assert_eq!(second.created, 0);
        assert_eq!(second.healed, 0);
    }

    #[test]
    fn scaffold_does_not_overwrite_existing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create an advisory file with custom content.
        std::fs::create_dir_all(root.join(".claude/rules")).unwrap();
        std::fs::write(root.join(".claude/rules/tools.md"), "custom content").unwrap();

        scaffold_agent_config(root);

        // Advisory files should not be overwritten.
        let content = std::fs::read_to_string(root.join(".claude/rules/tools.md")).unwrap();
        assert_eq!(content, "custom content");
    }

    #[test]
    fn claude_hooks_merge_with_existing_settings() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create settings with existing content (no hooks key).
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"permissions": {"allow": ["Bash(cargo test)"]}}"#,
        )
        .unwrap();

        let result = write_claude_hooks(root);
        // File existed but had no hooks — treated as heal (overwrites hooks key).
        assert_eq!(result.healed, 1);

        let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&settings).unwrap();

        // Should have both hooks and permissions.
        assert!(val["hooks"]["PreToolUse"].is_array());
        assert!(val["permissions"]["allow"].is_array());
    }

    #[test]
    fn claude_hooks_heals_stale_hooks() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Pre-create settings with outdated hooks content.
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks": {"PostToolUse": []}}"#,
        )
        .unwrap();

        let result = write_claude_hooks(root);
        assert_eq!(result.healed, 1); // Should heal — hooks are stale.
        assert_eq!(result.created, 0);

        // Verify the hooks were replaced with the correct content.
        let settings = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        let val: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert!(val["hooks"]["PreToolUse"].is_array());
    }

    #[test]
    fn autoheal_overwrites_stale_hook_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // First scaffold creates everything.
        let first = scaffold_agent_config(root);
        assert_eq!(first.created, 9);

        // Corrupt a hook file (machine-generated — should be healed).
        std::fs::write(
            root.join(".github/hooks/iris-enforce.json"),
            r#"{"version": 1, "hooks": {}}"#,
        )
        .unwrap();

        // Corrupt cursor hooks too.
        std::fs::write(root.join(".cursor/hooks.json"), "{}").unwrap();

        let second = scaffold_agent_config(root);
        assert_eq!(second.created, 0);
        assert_eq!(second.healed, 2); // Both hook files healed.

        // Verify content was restored.
        let copilot = std::fs::read_to_string(root.join(".github/hooks/iris-enforce.json")).unwrap();
        let cval: serde_json::Value = serde_json::from_str(&copilot).unwrap();
        assert!(cval["hooks"]["preToolUse"].is_array());
    }
}
