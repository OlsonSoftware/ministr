# assets/launch.tape — VHS source for the README demo GIF.
#
# Regenerate with:
#   ANTHROPIC_API_KEY=sk-...  vhs assets/launch.tape
#
# Requirements on PATH: vhs, claude, ministr.
# Uses a fresh mktemp dir — leaves nothing on disk after.
#
# Story:
#   1. `ministr init` scaffolds .ministr.toml for the current project.
#   2. `claude mcp add` wires up the MCP server (local scope).
#   3. `claude` launches; we ask it to use ministr tools on a sample file.
#   4. Exit cleanly.

Output assets/launch.gif

# `Require` is checked against the shell that launched vhs — before the
# tape's Hide block extends PATH. Only guard on `claude` (in ~/.local/bin
# after the standard installer). `ministr` lives under ~/.ministr/bin and
# is added to PATH inside the tape body.
Require claude

Set Shell           "bash"
Set FontFamily      "JetBrains Mono"
Set FontSize        14
Set Width           1200
Set Height          720
Set Theme           "Dracula"
Set TypingSpeed     45ms
Set PlaybackSpeed   1.0
Set WindowBar       Colorful
Set WindowBarSize   28
Set Padding         16
Set BorderRadius    8

# -----------------------------------------------------------------------------
# Hidden setup: make a tmp demo project with one sample file and a
# .claude/settings.local.json that pre-allows the ministr MCP tools
# (otherwise Claude Code would pause on permission prompts).
# -----------------------------------------------------------------------------
Hide
# Ensure ministr + claude resolve via PATH in the recording shell.
Type "export PATH=$HOME/.ministr/bin:$HOME/.local/bin:$PATH"
Enter
Sleep 100ms
Type "REPO=$PWD && DEMO_DIR=$(mktemp -d)"
Enter
Sleep 150ms
Type "$REPO/scripts/demo-setup.sh $DEMO_DIR"
Enter
Sleep 200ms
Type "cd $DEMO_DIR && clear"
Enter
Sleep 200ms
Show

# -----------------------------------------------------------------------------
# Act 1 — scaffold the ministr config.
# -----------------------------------------------------------------------------
Type "ministr init"
Enter
Sleep 2500ms

# -----------------------------------------------------------------------------
# Act 2 — register ministr as an MCP server for Claude Code.
# -----------------------------------------------------------------------------
Type "claude mcp add ministr -- ministr"
Enter
Sleep 1800ms

# -----------------------------------------------------------------------------
# Act 3 — launch Claude Code and exercise ministr tools.
# -----------------------------------------------------------------------------
Type "claude --permission-mode acceptEdits"
Enter

# Workspace-trust dialog renders first; default selection is "Yes, I trust".
# One Enter confirms and drops us into the REPL.
Sleep 2500ms
Enter

# Welcome banner + MCP handshake + REPL ready.
Sleep 4s

Type "Using ministr tools, summarize what src/greeter.py does in one sentence."
Sleep 400ms
Enter

# Let the model run ministr_survey / ministr_read and render a reply.
Sleep 18s

# -----------------------------------------------------------------------------
# Clean exit.
# -----------------------------------------------------------------------------
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 800ms
