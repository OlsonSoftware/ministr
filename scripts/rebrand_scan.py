#!/usr/bin/env python3
"""Report any residual iris references left after the rebrand."""
from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

SKIP_PATHS = {
    "docs-next/package-lock.json",
    "pnpm-lock.yaml",
    "ministr-app/pnpm-lock.yaml",
    "Cargo.lock",
    "scripts/rebrand_ministr.py",
    "scripts/rebrand_pass2.py",
    "scripts/rebrand_scan.py",
}
# These paths carry iris tokens intentionally (or acceptably).
ACCEPT_PATHS: set[str] = set()

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

# Match the 4-letter token "iris" in any case. We only care about it as a
# standalone token; too-broad matches (random base64) are skipped via
# SKIP_PATHS above.
PATTERN = re.compile(r"[Ii][Rr][Ii][Ss]")


def main() -> int:
    out = subprocess.check_output(["git", "ls-files"], cwd=REPO, text=True)
    files = [line for line in out.splitlines() if line]
    hits: dict[str, list[str]] = {}
    for rel in files + EXTRA_UNTRACKED:
        if rel in SKIP_PATHS:
            continue
        if rel in ACCEPT_PATHS:
            continue
        path = REPO / rel
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
        matches = []
        for m in PATTERN.finditer(text):
            start = max(0, m.start() - 25)
            end = m.end() + 25
            ctx = text[start:end].replace("\n", " ")
            matches.append(f"{m.group()}: ...{ctx}...")
        if matches:
            hits[rel] = matches

    print(f"files with residual iris tokens: {len(hits)}")
    total = sum(len(v) for v in hits.values())
    print(f"total matches: {total}")
    for f in sorted(hits):
        print(f"\n{f}  ({len(hits[f])} hits)")
        for line in hits[f][:8]:
            print(f"  {line}")
        if len(hits[f]) > 8:
            print(f"  ... and {len(hits[f]) - 8} more")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
