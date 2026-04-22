# assets/launch.tape — VHS source for the README demo GIF.
#
# Regenerate:
#   ANTHROPIC_API_KEY=sk-...            vhs assets/launch.tape
#   # OR
#   CLAUDE_CODE_OAUTH_TOKEN=$(claude setup-token)  vhs assets/launch.tape
#
# Requirements on PATH: vhs, claude, ministr.
#
# Why explicit auth?
#   The tape runs with HOME pointed at a scratch dir so the recording leaks
#   no personal data (displayName / email / organization / subscription line
#   are blanked). macOS keychain OAuth tokens are tied to the original
#   login-session binary grant, so they do NOT resolve through the scratch
#   HOME even with the same keychain service name. Env-var auth bypasses
#   keychain and just works.
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
# Hidden setup.
#
# 1. Save the maintainer's real HOME as $REAL_HOME so ~/.ministr/bin remains
#    resolvable after we point HOME at a scratch dir.
# 2. Stage demo-setup.sh's tree (project/ + home/) under $DEMO_DIR.
# 3. Point HOME at the scratch $DEMO_DIR/home so claude never reads the
#    maintainer's oauthAccount / subscription / trust history.
# -----------------------------------------------------------------------------
Hide
Type "export REAL_HOME=$HOME"
Enter
Sleep 80ms
Type "REPO=$PWD && DEMO_DIR=$(mktemp -d)"
Enter
Sleep 80ms
Type "$REPO/scripts/demo-setup.sh $DEMO_DIR $REAL_HOME"
Enter
Sleep 200ms
Type "export HOME=$DEMO_DIR/home"
Enter
Sleep 80ms
# Put the scratch HOME's .local/bin first so `claude`/`ministr` resolve
# through the symlinks demo-setup.sh created there. Claude Code also
# probes $HOME/.local/bin/claude directly for its installMethod check.
Type "export PATH=$HOME/.local/bin:$PATH"
Enter
Sleep 80ms
Type "cd $DEMO_DIR/project && clear"
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

# Two dialogs fire on first launch in a fresh HOME:
#   (a) Workspace trust (default = "Yes, I trust this folder")
#   (b) New MCP server approval for `ministr` (default = "Use this and all
#       future MCP servers in this project")
# Press Enter on each to accept defaults.
Sleep 2500ms
Enter
Sleep 2500ms
Enter

# Welcome banner + MCP handshake + REPL ready.
Sleep 4s

Type "Using ministr, trace how greet is wired up — find its definition, list every caller, and tell me what the three tones resolve to."
Sleep 400ms
Enter

# Expected ministr calls: symbols(greet) → definition → references → read(Tone).
# Budget generously so the full tool trace lands before we linger.
Sleep 26s

# -----------------------------------------------------------------------------
# Hold on the final answer so a reader can actually parse it.
# GIF compresses duplicate frames cheaply, so this costs almost nothing.
# -----------------------------------------------------------------------------
Sleep 8s

# -----------------------------------------------------------------------------
# Clean exit.
# -----------------------------------------------------------------------------
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 800ms
