#!/usr/bin/env python3
"""Docs freshness gate.

Checks the hand-written docs corpus (docs/**/*.md + AGENTS.md + README.md):

1. Link check — every relative link target is a *tracked* file
   (``git ls-files``, not disk existence: an existing-but-gitignored
   target 404s on GitHub).
2. llms.txt coverage — every hand-written docs content page appears in
   docs/llms.txt.
3. Style probes — no meta commentary, marketing microcopy, or
   percentage/benchmark literals in user-facing pages.
4. Version literals — the workspace version must not be hard-coded in
   docs prose (it drifts on every release).

Exclusions: docs/reference/tools/ (generated; gated by
tests/tool_manifest_parity.rs) and docs/adr/ (historical decision
records, checked for links only).

Run from the repo root: ``python3 scripts/ci/check_docs.py``
"""

import os
import re
import subprocess
import sys

failures: list[str] = []


def fail(msg: str) -> None:
    failures.append(msg)
    print(f"FAIL: {msg}")


tracked = set(
    subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout.split()
)

pages = sorted(
    f
    for f in tracked
    if (f.startswith("docs/") and f.endswith(".md")) or f in ("AGENTS.md", "README.md")
)
generated = [p for p in pages if p.startswith("docs/reference/tools/")]
adr = [p for p in pages if p.startswith("docs/adr/")]
hand = [p for p in pages if p not in generated and p not in adr]

# ── 1. tracked-ness link check (all pages, including ADRs) ──────────────
for page in pages:
    root = os.path.dirname(page)
    body = open(page, encoding="utf-8").read()
    for target in re.findall(r"\]\(([^)#\s]+)", body):
        if target.startswith(("http://", "https://", "mailto:")):
            continue
        resolved = os.path.normpath(os.path.join(root, target))
        if resolved not in tracked:
            fail(f"{page}: link to untracked path '{target}'")

# ── 2. llms.txt coverage ────────────────────────────────────────────────
LLMS = "docs/llms.txt"
if LLMS not in tracked:
    fail(f"{LLMS} is missing or untracked")
else:
    llms = open(LLMS, encoding="utf-8").read()
    content_pages = [
        p
        for p in hand
        if p.startswith("docs/") and os.path.basename(p) != "README.md"
    ]
    for page in content_pages:
        if page not in llms:
            fail(f"{LLMS}: does not link {page}")

# ── 3. style probes (hand-written user-facing pages only) ──────────────
META = re.compile(
    r"(?i)(this (guide|page|section|document) (covers|describes|explains|will))"
    r"|in this (guide|section)"
)
MARKETING = re.compile(
    r"(?i)\b(blazing|powerful|seamless|effortless|supercharge|revolutioniz)"
)
PERCENT = re.compile(r"\d+(\.\d+)?%")
for page in hand:
    body = re.sub(r"<!--.*?-->", "", open(page, encoding="utf-8").read(), flags=re.S)
    if META.search(body):
        fail(f"{page}: meta commentary")
    if MARKETING.search(body):
        fail(f"{page}: marketing language")
    if PERCENT.search(body):
        fail(f"{page}: percentage literal (benchmark-style numbers drift; keep them out)")

# ── 4. version-literal lint ─────────────────────────────────────────────
version = None
for line in open("Cargo.toml", encoding="utf-8"):
    m = re.match(r'\s*version\s*=\s*"([^"]+)"', line)
    if m:
        version = m.group(1)
        break
if version:
    for page in hand:
        body = re.sub(
            r"<!--.*?-->", "", open(page, encoding="utf-8").read(), flags=re.S
        )
        if version in body:
            fail(f"{page}: hard-coded version '{version}' (drifts on release)")

label = f"{len(pages)} pages ({len(hand)} hand-written, {len(generated)} generated, {len(adr)} ADR)"
if failures:
    print(f"\n{label}: {len(failures)} failure(s)")
    sys.exit(1)
print(f"{label}: all docs checks passed")
