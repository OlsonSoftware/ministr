#!/usr/bin/env bash
# Stages a tmp demo project for assets/launch.tape.
# Usage: scripts/demo-setup.sh <target-dir>
#
# Creates:
#   <target-dir>/src/greeter.py          — one small sample file
#   <target-dir>/.claude/settings.local.json — pre-allows ministr MCP tools
#     so Claude Code doesn't pause on permission prompts during recording.

set -euo pipefail

dir="${1:?usage: demo-setup.sh <target-dir>}"
mkdir -p "$dir/src" "$dir/.claude"

cat > "$dir/src/greeter.py" <<'PY'
def greet(name: str) -> str:
    """Return a friendly greeting for name."""
    return f'Hello, {name}!'
PY

cat > "$dir/.claude/settings.local.json" <<'JSON'
{
  "permissions": {
    "allow": [
      "mcp__ministr__ministr_survey",
      "mcp__ministr__ministr_symbols",
      "mcp__ministr__ministr_definition",
      "mcp__ministr__ministr_read",
      "mcp__ministr__ministr_toc"
    ]
  }
}
JSON
