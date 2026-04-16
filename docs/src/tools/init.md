# iris init

Generate `.iris.toml` and scaffold agent configuration files for your project.

## Usage

```sh
# Auto-detect and generate (non-interactive)
iris init

# Overwrite existing configuration
iris init --force

# Interactive wizard with prompts
iris init --interactive
```

## What it does

1. **Detects project structure** — scans for manifests (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`), workspace layouts, and bridge frameworks (Tauri, FFI, WASM).

2. **Generates `.iris.toml`** — a per-project configuration file with auto-detected corpus paths, ignore patterns, and source directories.

3. **Scaffolds agent configs** — writes platform-specific rules and hooks for all supported agent platforms.

## Platform Matrix

| Platform | Rules File | Hooks File | Format |
|----------|-----------|------------|--------|
| Claude Code | `.claude/rules/iris-scope.md`, `iris-playbook.md`, `tools.md` | `.claude/settings.json` | Markdown rules + JSON PreToolUse hooks |
| Cursor | `.cursor/rules/iris.mdc` | `.cursor/hooks.json` | MDC rules + JSON hooks |
| GitHub Copilot | `.github/copilot-instructions.md` | `.github/hooks/iris-enforce.json` | Markdown instructions + JSON hooks |
| Windsurf | `.windsurf/rules/iris.md` | `.windsurf/hooks.json` | Markdown rules + JSON hooks |
| Continue.dev | `.continue/rules/iris.md` | — | Markdown rules only |
| Universal | `AGENTS.md` | — | Markdown (agent-agnostic) |

## Hook Formats

### Claude Code (`.claude/settings.json`)

PreToolUse hooks block Glob, Grep, and Bash search commands, forcing agents to use iris semantic search tools instead.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Glob|Grep",
        "command": "...",
        "action": "deny"
      }
    ]
  }
}
```

### Cursor (`.cursor/hooks.json`)

Blocks shell-based search and find commands in terminal tool calls.

### GitHub Copilot (`.github/hooks/iris-enforce.json`)

Same enforcement pattern adapted for Copilot CLI and Copilot cloud agents.

## File Behavior

- **Advisory files** (`.md`, `.mdc`) are created if missing but **never overwritten** — you can customize them freely.
- **Machine-generated hook files** (`.json`) are **auto-healed** — if the on-disk content differs from the current template, iris overwrites it to ensure enforcement stays current.

## Interactive Mode

`iris init --interactive` launches a guided wizard that prompts for:

1. **Project type confirmation** — auto-detected, with option to override
2. **Agent platforms** — multi-select checkboxes for which platforms to configure
3. **Hook strictness level**:
   - **Strict** — block Grep/Glob/Bash search outright (recommended)
   - **Moderate** — warn on blocked tools, allow with confirmation
   - **Advisory** — suggest iris tools, never block

## Custom Rules

Add custom agent rules via the `[agent]` section in `.iris.toml`:

```toml
[agent]
rules = [
  "Always use iris_survey before modifying shared code",
  "Run iris_references before deleting any public API",
]
```

These rules are injected into `iris-custom.md` across all configured platforms.

## Language-Specific Rules

iris auto-detects project languages and generates `iris-lang-rules.md` with best practices for each detected language (Rust, TypeScript, Python, Go, etc.).

## Troubleshooting

**Q: `.iris.toml` already exists**
Use `--force` to overwrite, or edit it manually.

**Q: Hooks aren't blocking search tools**
Run `iris hooks test` to validate hook installation and simulate tool calls.

**Q: Agent ignores iris rules**
Check that your agent platform reads from the correct rules directory. Run `iris init` again to re-scaffold (advisory files are preserved, hooks are auto-healed).
