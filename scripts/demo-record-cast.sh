#!/usr/bin/env bash
# Record assets/launch.cast — the asciinema recording that drives the
# interactive terminal demo on the web site.
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

# Normalize TERM so `clear` (tput) and asciinema's ncurses bits don't
# fail on terminals whose terminfo entries aren't installed system-wide
# (seen with Rio, WezTerm on minimal installs, etc.). xterm-256color is
# the safe lingua franca that every macOS / Linux setup has.
if ! infocmp -x "${TERM:-}" >/dev/null 2>&1; then
    export TERM=xterm-256color
fi

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CAST="$REPO/assets/launch.cast"
DEMO_DIR=$(mktemp -d)

# Stage scratch env (same logic as the VHS tape's Hide block).
"$REPO/scripts/demo-setup.sh" "$DEMO_DIR" "$HOME"
export HOME="$DEMO_DIR/home"
export PATH="$HOME/.local/bin:$PATH"

# Clear without going through `tput` — avoids terminfo lookup failures.
printf '\033[2J\033[H'

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

# Reset TTY settings in the parent so the PTY asciinema spawns inherits
# them sane. Force a landscape geometry (96 cols × 26 rows) so the cast
# header reflects the hero's 3:2 aspect-ratio target — no post-hoc
# rewriting of the cast JSON needed. 26 rows comfortably holds one
# Claude Code alt-screen; anything beyond scrolls naturally.
stty sane 2>/dev/null || true
stty cols 96 rows 26 2>/dev/null || true

# Point readline at the inputrc demo-setup.sh staged inside the scratch
# HOME — ensures Backspace / arrow keys work the same on Rio, WezTerm,
# Kitty, iTerm2. Force bash (not the user's $SHELL, which may be zsh
# with custom key bindings that don't match the scratch HOME).
export INPUTRC="$HOME/.inputrc"

# Use asciicast-v2 for broadest player compatibility.
# --idle-time-limit caps long pauses so the cast file stays small and
# the playback stays brisk.
asciinema rec \
    --overwrite \
    --output-format asciicast-v2 \
    --idle-time-limit 2 \
    --command "/bin/bash --rcfile $HOME/.bashrc -i" \
    "$CAST"

echo
echo "✓ Recorded: $CAST"
echo "  Cast size: $(wc -c < "$CAST" | tr -d ' ') bytes"
