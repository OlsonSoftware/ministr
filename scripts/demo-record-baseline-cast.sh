#!/usr/bin/env bash
# Record assets/launch-baseline.cast — the "without ministr" side of
# the landing page's side-by-side comparison.
#
# Stages the SAME scratch project as demo-record-cast.sh (greeter_cli
# with its tone + cli + core modules) but does NOT register the
# ministr MCP server. Claude Code falls back to its built-in
# Glob / Grep / Read tools — the exact behaviour ministr was built to
# replace. The contrast with the ministr recording is the point.
#
# Prerequisite: `brew install asciinema` (v3+).
#
# Usage:
#   scripts/demo-record-baseline-cast.sh

set -euo pipefail

if ! command -v asciinema >/dev/null 2>&1; then
    echo "error: asciinema not found. Install with: brew install asciinema" >&2
    exit 1
fi

if ! infocmp -x "${TERM:-}" >/dev/null 2>&1; then
    export TERM=xterm-256color
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CAST="$REPO/assets/launch-baseline.cast"
DEMO_DIR=$(mktemp -d)

"$REPO/scripts/demo-setup.sh" "$DEMO_DIR" "$HOME"
export HOME="$DEMO_DIR/home"
export PATH="$HOME/.local/bin:$PATH"

printf '\033[2J\033[H'

cat <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Recording BASELINE (no ministr) → $CAST
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Run these (in order) inside the recording. NOTE: no 'claude mcp add'
— we want Claude to fall back to its built-in tools.

  1. claude --permission-mode acceptEdits

     • [Enter] to accept workspace trust
     • Type this prompt and press Enter (SAME prompt as the
       ministr cast so the side-by-side reads as one task,
       two paths):
         Trace how greet is wired up — find its definition,
         list every caller, and tell me what the three tones
         resolve to.
     • Wait for Claude to grep/glob/read its way through
     • Press Esc twice (or /exit) to leave claude
  2. Type 'exit' to end the recording

Keep your terminal roughly 96x26 for consistency with the ministr cast.

Press Enter to start…
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF

read -r

cd "$DEMO_DIR/project"
stty sane 2>/dev/null || true
stty cols 96 rows 26 2>/dev/null || true
export INPUTRC="$HOME/.inputrc"

asciinema rec \
    --overwrite \
    --output-format asciicast-v2 \
    --idle-time-limit 2 \
    --command "/bin/bash --rcfile $HOME/.bashrc -i" \
    "$CAST"

echo
echo "✓ Recorded: $CAST ($(wc -c < "$CAST" | tr -d ' ') bytes)"
