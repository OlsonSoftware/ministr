#!/usr/bin/env python3
"""Rebrand the shared /Users/alrik/Code/.claude/rules/ directory.

Files renamed: iris-playbook.md -> ministr-playbook.md
               iris-scope.md    -> ministr-scope.md
Content rewritten in: all .md files (iris_* tool names, Iris identifiers).
"""
from __future__ import annotations

import re
from pathlib import Path

RULES = Path("/Users/alrik/Code/.claude/rules")

RENAMES = [
    ("iris-playbook.md", "ministr-playbook.md"),
    ("iris-scope.md", "ministr-scope.md"),
    ("iris-lang-rules.md", "ministr-lang-rules.md"),
]


def transform(text: str) -> str:
    out = text
    out = out.replace("iris-rs", "ministr-rs")
    out = out.replace("dev.iris/", "dev.ministr/")
    for crate in ("core", "api", "daemon", "mcp", "cli", "app"):
        out = out.replace(f"iris_{crate}", f"ministr_{crate}")
        out = out.replace(f"iris-{crate}", f"ministr-{crate}")
    out = out.replace(".iris.toml", ".ministr.toml")
    out = out.replace("IRIS_", "MINISTR_")
    out = out.replace("iris_", "ministr_")
    out = out.replace("Iris_", "Ministr_")
    out = out.replace("irisd", "ministrd")
    # CamelCase and bare tokens
    out = re.sub(r"Iris(?=[A-Za-z0-9])", "Ministr", out)
    out = re.sub(r"Iris\b", "Ministr", out)
    out = re.sub(r"\biris\b", "ministr", out)
    out = re.sub(r"IRIS(?=[_A-Z0-9]|\b)", "MINISTR", out)
    return out


def main() -> int:
    # Rename files.
    for old, new in RENAMES:
        src, dst = RULES / old, RULES / new
        if src.exists() and not dst.exists():
            print(f"mv  {old} -> {new}")
            src.rename(dst)
    # Rewrite contents of all .md files.
    for p in sorted(RULES.glob("*.md")):
        text = p.read_text(encoding="utf-8")
        new = transform(text)
        if new != text:
            p.write_text(new, encoding="utf-8")
            print(f"rewrote {p.name}")
    print("done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
