#!/usr/bin/env python3
"""Second-pass rebrand cleanup.

First pass missed:
  - `irisd` (daemon suffix; no word boundary before `d`).
  - CamelCase `Iris` inside larger identifiers (e.g., `WhatIrisIsnt`).
  - SCREAMING_SNAKE `IRIS` tokens inside larger identifiers
    (e.g., `WITHOUT_IRIS`, `WITH_IRIS`, `_IRIS_STAGE`).

Safety:
  - Skips lock files and the docs-next lock file where `IRIS` can appear
    inside random base64 tokens.
  - Does not re-process binary files.
"""
from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SELF = Path(__file__).resolve()

SKIP_PATHS = {
    "docs-next/package-lock.json",
    "pnpm-lock.yaml",
    "ministr-app/pnpm-lock.yaml",
    "Cargo.lock",
}

TEXT_EXT = {
    ".rs", ".toml", ".md", ".mdx", ".yml", ".yaml", ".json", ".js",
    ".mjs", ".ts", ".tsx", ".html", ".css", ".py", ".sh", ".rb",
    ".xml", ".svg", ".plist", ".swift", ".txt",
}
TEXT_NAMES = {"Cargo.toml", "Dockerfile", "justfile", "rustfmt.toml"}

EXTRA_UNTRACKED = [
    ".claude/rules/ministr-playbook.md",
    ".claude/rules/ministr-scope.md",
    ".claude/rules/ministr-lang-rules.md",
    ".claude/rules/tools.md",
    ".claude/rules/workflow.md",
    ".claude/rules/conventions.md",
]


def tracked_files() -> list[str]:
    out = subprocess.check_output(
        ["git", "ls-files"], cwd=REPO, text=True
    )
    return [line for line in out.splitlines() if line]


def transform(text: str) -> str:
    out = text
    # Daemon suffix.
    out = out.replace("irisd", "ministrd")
    # CamelCase inside larger identifiers.
    out = re.sub(r"Iris(?=[A-Za-z0-9])", "Ministr", out)
    out = re.sub(r"Iris\b", "Ministr", out)
    # SCREAMING_SNAKE IRIS inside identifiers.
    out = re.sub(r"IRIS(?=[_A-Z0-9]|\b)", "MINISTR", out)
    return out


def main() -> int:
    changed = 0
    for rel in tracked_files() + EXTRA_UNTRACKED:
        if rel in SKIP_PATHS:
            continue
        path = REPO / rel
        if path.resolve() == SELF:
            continue
        if not path.exists():
            continue
        ext = path.suffix
        if ext not in TEXT_EXT and path.name not in TEXT_NAMES:
            continue
        try:
            with path.open("rb") as fh:
                raw = fh.read()
            if b"\0" in raw[:4096]:
                continue
            text = raw.decode("utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        new = transform(text)
        if new != text:
            path.write_text(new, encoding="utf-8")
            changed += 1
    print(f"second pass rewrote {changed} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
