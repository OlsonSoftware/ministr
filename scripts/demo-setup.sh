#!/usr/bin/env bash
# Stages a tmp demo project + sanitized HOME for assets/launch.tape.
# Usage: scripts/demo-setup.sh <target-dir> [<source-home>]
#
# Layout created under <target-dir>:
#   project/src/greeter.py              — one sample file
#   project/.claude/settings.local.json — pre-allows ministr MCP tools so
#                                          Claude Code doesn't stall on
#                                          permission prompts during recording
#   home/.claude.json                   — sanitized clone of <source-home>'s
#                                          .claude.json, with oauthAccount
#                                          displayName/emailAddress/
#                                          organizationName blanked out. The
#                                          accountUuid stays so the macOS
#                                          keychain entry for OAuth tokens
#                                          still resolves (the keychain
#                                          service name "Claude Code" does
#                                          not depend on HOME, so tokens
#                                          stored under the real user's
#                                          login remain reachable from the
#                                          scratch HOME).
#
# <source-home> defaults to $HOME. If the source has no .claude.json (fresh
# install, CI), a minimal stub is written — in that case the recording needs
# ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN to authenticate.

set -euo pipefail

dir="${1:?usage: demo-setup.sh <target-dir> [<source-home>]}"
src_home="${2:-$HOME}"
project="$dir/project"
home="$dir/home"

mkdir -p "$project/src/greeter_cli" "$project/.claude" \
         "$home/.claude" "$home/.local/bin"

# Minimal readline + bash config so an interactive shell spawned under
# HOME=$home (via VHS or asciinema) has correct backspace/arrow-key
# handling. Without this, readline falls back to builtin defaults that
# don't match the key codes terminals like Rio/Kitty/WezTerm send, and
# Backspace visibly renders as a space instead of erasing.
cat > "$home/.inputrc" <<'INPUTRC'
# Both the DEL (0x7F) and BS (0x08) byte sequences should erase back.
"\C-?": backward-delete-char
"\C-h": backward-delete-char
# Arrow keys for command history / cursor movement.
"\e[A": previous-history
"\e[B": next-history
"\e[C": forward-char
"\e[D": backward-char
# Bracketed paste (safer when tutorials paste multi-line snippets).
set enable-bracketed-paste on
set editing-mode emacs
set horizontal-scroll-mode off
INPUTRC

cat > "$home/.bashrc" <<'BASHRC'
# Quiet, deterministic prompt for the recording. Keeps the cast visually
# close to Claude Code's own chrome without leaking the real host/user.
export PS1='$ '
stty sane 2>/dev/null || true
stty erase '^?' 2>/dev/null || true
BASHRC

# Mirror just the two binaries we need into the scratch HOME so:
#   1. Claude Code's installMethod check ($HOME/.local/bin/claude) passes.
#   2. `claude mcp add ministr -- ministr` can spawn the ministr subprocess
#      via PATH lookup once $HOME/.local/bin is prepended.
for bin in claude ministr; do
    for src in "$src_home/.local/bin/$bin" "$src_home/.ministr/bin/$bin"; do
        if [ -x "$src" ]; then
            ln -sf "$src" "$home/.local/bin/$bin"
            break
        fi
    done
done

# The `ministr` CLI is a thin proxy over the ministr daemon's UDS at
# $HOME/.ministr/ministrd.sock. Symlink the whole dir so the scratch HOME
# sees the running daemon (plus models, corpora, etc.) without duplicating.
if [ -d "$src_home/.ministr" ]; then
    ln -sf "$src_home/.ministr" "$home/.ministr"
fi

cat > "$project/src/greeter_cli/__init__.py" <<'PY'
"""A tiny CLI that greets people in three registers."""

from .core import greet, default_greeting
from .cli import main

__all__ = ["greet", "default_greeting", "main"]
PY

cat > "$project/src/greeter_cli/core.py" <<'PY'
"""Greeting logic."""

from .tone import Tone


def greet(name: str, tone: Tone = Tone.FRIENDLY) -> str:
    """Return a greeting for `name` in the requested tone."""
    match tone:
        case Tone.FORMAL:
            return f"Good day, {name}."
        case Tone.FRIENDLY:
            return f"Hello, {name}!"
        case Tone.CASUAL:
            return f"hey {name}"


def default_greeting() -> str:
    """Greet the world in the friendliest tone."""
    return greet("world", Tone.FRIENDLY)
PY

cat > "$project/src/greeter_cli/tone.py" <<'PY'
"""Greeting tone levels."""

from enum import Enum


class Tone(str, Enum):
    FORMAL = "formal"
    FRIENDLY = "friendly"
    CASUAL = "casual"
PY

cat > "$project/src/greeter_cli/cli.py" <<'PY'
"""Command-line entry point."""

import argparse

from .core import greet
from .tone import Tone


def main() -> None:
    parser = argparse.ArgumentParser(description="Greet someone.")
    parser.add_argument("name", nargs="?", default="world")
    parser.add_argument(
        "--tone", choices=[t.value for t in Tone], default=Tone.FRIENDLY.value
    )
    args = parser.parse_args()
    print(greet(args.name, Tone(args.tone)))


if __name__ == "__main__":
    main()
PY

cat > "$project/README.md" <<'MD'
# greeter-cli

A tiny demo CLI that greets by name, in three tones.

- `core.greet(name, tone)` — the greeting primitive.
- `tone.Tone` — enum of supported registers.
- `cli.main` — the argparse entry point.
MD

cat > "$project/.claude/settings.local.json" <<'JSON'
{
  "permissions": {
    "allow": [
      "mcp__ministr__ministr_survey",
      "mcp__ministr__ministr_symbols",
      "mcp__ministr__ministr_definition",
      "mcp__ministr__ministr_references",
      "mcp__ministr__ministr_read",
      "mcp__ministr__ministr_extract",
      "mcp__ministr__ministr_toc",
      "mcp__ministr__ministr_bridge"
    ]
  }
}
JSON

src_config="$src_home/.claude.json"
dst_config="$home/.claude.json"

if [ -f "$src_config" ]; then
    python3 - "$src_config" "$dst_config" <<'PY'
import json
import sys

src, dst = sys.argv[1], sys.argv[2]
with open(src) as f:
    cfg = json.load(f)

# Scrub the personally-identifying fields on the OAuth account entry.
# Leave the UUIDs + tokens-related fields so the macOS keychain lookup for
# OAuth tokens still succeeds (service name "Claude Code" is HOME-agnostic
# when CLAUDE_CONFIG_DIR is unset).
acct = cfg.get("oauthAccount")
if isinstance(acct, dict):
    acct["displayName"] = ""
    acct["emailAddress"] = ""
    acct["organizationName"] = ""

# Drop project-specific history (trust, command usage, etc.) — we'll let the
# demo project re-populate cleanly. Keep top-level account + onboarding state.
cfg["projects"] = {}

# Suppress release-notes + onboarding banners.
cfg["hasCompletedOnboarding"] = True
cfg["lastReleaseNotesSeen"] = "999.0.0"

with open(dst, "w") as f:
    json.dump(cfg, f, indent=2)
PY
else
    cat > "$dst_config" <<'JSON'
{
  "numStartups": 2,
  "hasCompletedOnboarding": true,
  "lastReleaseNotesSeen": "999.0.0"
}
JSON
fi
