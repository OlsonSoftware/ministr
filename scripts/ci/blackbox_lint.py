#!/usr/bin/env python3
"""Black-box lint: fail if a PUBLIC-FACING surface leaks closed-source internals.

ministr is proprietary. Public surfaces (the README, the docs site, and
the agent-rule constants scaffolded into *users'* repos) must describe
*what* ministr does, never *how* - no internal crate source paths and no
legacy/internal jargon.

Scope is deliberately narrow to stay false-positive-free: it does NOT
forbid generic `src/foo.rs` (those appear as legitimate example payloads
in the tool docs), only unambiguous internal leaks.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Public-facing files only. Internal Rust source / CONTRIBUTING are NOT
# scanned (they legitimately name internals and never ship to users).
TARGETS: list[str] = [
    "README.md",
    "ministr-core/src/scaffold.rs",  # constants written into users' repos
]
TARGET_DIRS: list[str] = ["docs-next/content"]

# Unambiguous internal leaks. Case-insensitive, substring match.
FORBIDDEN: list[tuple[str, str]] = [
    ("session shadow", "internal mechanism name; describe the behavior instead"),
    ("claim shadow", "internal mechanism name"),
    ("context cache", "legacy positioning; ministr is a code intelligence MCP server"),
    ("context-cache", "legacy positioning; ministr is a code intelligence MCP server"),
    ("ministr-core/src", "internal source path"),
    ("ministr-daemon/src", "internal source path"),
    ("ministr-mcp/src", "internal source path"),
    ("ministr-api/src", "internal source path"),
    ("ministr-cli/src", "internal source path"),
]


def iter_files():
    for rel in TARGETS:
        p = ROOT / rel
        if p.is_file():
            yield p
    for d in TARGET_DIRS:
        base = ROOT / d
        if base.is_dir():
            for p in base.rglob("*"):
                if p.is_file() and p.suffix in {".md", ".mdx", ".json"}:
                    yield p


def main() -> int:
    violations: list[str] = []
    for path in iter_files():
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        lowered = text.lower()
        for needle, why in FORBIDDEN:
            if needle in lowered:
                for n, line in enumerate(text.splitlines(), 1):
                    if needle in line.lower():
                        rel = path.relative_to(ROOT).as_posix()
                        violations.append(f"{rel}:{n}: '{needle}' - {why}")

    if violations:
        print("Black-box lint FAILED - public surfaces must stay black-box:\n")
        print("\n".join(sorted(violations)))
        print(
            "\nFix the wording (describe behavior, not internals) or, if this "
            "file is not actually public, narrow scripts/ci/blackbox_lint.py."
        )
        return 1

    print("black-box lint: clean - no internal leaks in public surfaces.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
