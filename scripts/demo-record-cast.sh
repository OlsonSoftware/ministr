#!/usr/bin/env bash
# Record assets/launch.cast — the asciinema recording that drives the
# interactive terminal demo on the docs-next site.
#
# Prerequisite: `brew install asciinema` (v3+).
#
# Usage:
#   scripts/demo-record-cast.sh
#
# The script reuses scripts/demo-setup.sh to stage the same scratch project
# + scrubbed HOME the VHS tape uses, so the cast contains no personal data
# (no displayName / email / organization). Auth still works through the
# live ~/.ministr daemon socket (symlinked into the scratch HOME) and
# whichever env-var path you launched with (ANTHROPIC_API_KEY or
# CLAUDE_CODE_OAUTH_TOKEN — see assets/launch.tape header).
#
# What you type inside the recording is the same 3-command sequence the
# VHS tape scripts. The on-screen cue appears BEFORE recording starts so
# the cast itself stays clean.

set -euo pipefail

if ! command -v asciinema >/dev/null 2>&1; then
    echo "error: asciinema not found. Install with: brew install asciinema" >&2
    exit 1
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CAST="$REPO/assets/launch.cast"
DEMO_DIR=$(mktemp -d)

# Stage scratch env (same logic as the VHS tape's Hide block).
"$REPO/scripts/demo-setup.sh" "$DEMO_DIR" "$HOME"
export HOME="$DEMO_DIR/home"
export PATH="$HOME/.local/bin:$PATH"

clear

cat <<EOF
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Recording asciinema demo → $CAST
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
When the recording starts, run these (in order):

  1. ministr init
  2. claude mcp add ministr -- ministr
  3. claude --permission-mode acceptEdits

     • [Enter] to accept workspace trust
     • [Enter] to accept MCP approval
     • Type this prompt and press Enter:
         Using ministr, trace how greet is wired up — find its
         definition, list every caller, and tell me what the three
         tones resolve to.
     • Wait for the response to finish
     • Press Esc twice (or /exit) to leave claude
  4. Type 'exit' to end the recording

Tip: keep your terminal roughly 120×32 or wider so the cast plays back
without reflow on the site.

Press Enter to start recording…
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
EOF

read -r

cd "$DEMO_DIR/project"

# Use asciicast-v2 for broadest player compatibility.
# --idle-time-limit caps long pauses so the cast file stays small and
# the playback stays brisk.
asciinema rec \
    --overwrite \
    --output-format asciicast-v2 \
    --idle-time-limit 2 \
    "$CAST"

echo
echo "✓ Recorded: $CAST"
echo "  Cast size: $(wc -c < "$CAST" | tr -d ' ') bytes"
