#!/usr/bin/env python3
"""Black-box lint: guard the surfaces users actually see.

Two independent checks:

1. **No internal leaks.** Public surfaces (the README and the agent-rule
   constants scaffolded into *users'* repos) describe *what* ministr does,
   never *how* - no internal crate source paths, no legacy jargon. Scope is
   deliberately narrow to stay false-positive-free: it does NOT forbid a
   generic `src/foo.rs` (those appear as legitimate example payloads in the
   tool docs), only unambiguous internal leaks.

2. **The published installers match the ones in the repo.** `install.sh` and
   `install.ps1` exist twice: at the repo root, and under `web/public/` where
   the static site serves them as ministr.ai/install.sh. Nothing synced them,
   and they silently drifted - the site served an installer that could not
   resolve a prerelease long after the root copy was fixed. Byte-equality is
   now a gate.
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
# Empty by design. This once pointed at `docs-next/content`, a marketing-site
# content tree that no longer exists (it became `web/`, which is JSX, not
# prose). Do NOT point it at `docs/`: the in-repo documentation legitimately
# names crate paths, and it already has its own gate in check_docs.py.
TARGET_DIRS: list[str] = []

# (repo copy, published-by-the-site copy). These must stay byte-identical:
# the second is what `curl https://ministr.ai/install.sh | bash` actually runs.
INSTALLER_PAIRS: list[tuple[str, str]] = [
    ("install.sh", "web/public/install.sh"),
    ("install.ps1", "web/public/install.ps1"),
]

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
    ("ministr-backend/src", "internal source path"),
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


def installer_drift() -> list[str]:
    """Report installers whose served copy no longer matches the repo copy."""
    out: list[str] = []
    for src_rel, pub_rel in INSTALLER_PAIRS:
        src, pub = ROOT / src_rel, ROOT / pub_rel
        if not src.is_file():
            out.append(f"{src_rel}: missing (the canonical installer)")
            continue
        if not pub.is_file():
            out.append(f"{pub_rel}: missing - the site would 404 on this installer")
            continue
        if src.read_bytes() != pub.read_bytes():
            out.append(
                f"{pub_rel}: out of sync with {src_rel} - the site would serve a "
                f"stale installer. Fix with: cp {src_rel} {pub_rel}"
            )
    return out


def main() -> int:
    violations: list[str] = installer_drift()
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
        print("Black-box lint FAILED - the surfaces users see are wrong:\n")
        print("\n".join(sorted(violations)))
        print(
            "\nFor wording: describe behavior, not internals (or, if the file is "
            "not actually public, narrow scripts/ci/blackbox_lint.py). For an "
            "installer: copy the root file over the web/public one."
        )
        return 1

    print("black-box lint: clean - no internal leaks, installers in sync.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
