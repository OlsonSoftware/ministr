#!/usr/bin/env python3
"""Fast release-PR engine — replaces release-plz (no cargo packaging).

On a push to main this:
  1. asks git-cliff for the next semver from Conventional Commits since
     the last `vX.Y.Z` tag (`git-cliff --bumped-version`),
  2. if that's a new version, bumps the SINGLE product version across
     every workspace crate + the internal path-dep requirements,
  3. regenerates CHANGELOG.md (Keep-a-Changelog, one section),
  4. refreshes Cargo.lock for the workspace members only.

git-cliff is a single prebuilt binary and touches no registry/deps, so
this is seconds, not the multi-minute `cargo package` release-plz ran.

Outputs (GITHUB_OUTPUT): `release` (true|false) and `version`.
"""
from __future__ import annotations
import os
import re
import subprocess
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]

# All crates share ONE product version (lockstep — mirrors the old
# release-plz version_group). tree-sitter-unreal-cpp is vendored and
# NOT part of the product version (stays pinned).
CRATES = [
    "ministr-api", "ministr-core", "ministr-daemon",
    "ministr-mcp", "ministr-cli", "ministr-app/src-tauri",
]
# Internal path deps in the root [workspace.dependencies] whose
# `version = "..."` requirement must track the product version
# (required for `cargo package`/lockstep correctness).
INTERNAL_DEPS = ["ministr-api", "ministr-core", "ministr-daemon", "ministr-mcp"]


def sh(*args: str, check: bool = True) -> str:
    r = subprocess.run(args, cwd=ROOT, text=True, capture_output=True)
    if check and r.returncode != 0:
        raise SystemExit(f"$ {' '.join(args)}\n{r.stdout}\n{r.stderr}")
    return r.stdout.strip()


def emit(key: str, value: str) -> None:
    print(f"{key}={value}")
    gh = os.environ.get("GITHUB_OUTPUT")
    if gh:
        with open(gh, "a", encoding="utf-8") as f:
            f.write(f"{key}={value}\n")


def manifest_version() -> str:
    txt = (ROOT / "ministr-cli" / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'(?m)^version = "([^"]+)"', txt)
    if not m:
        raise SystemExit("cannot read manifest version")
    return m.group(1)


def main() -> None:
    last_tag = sh("git", "describe", "--tags", "--match", "v[0-9]*",
                  "--abbrev=0", check=False)
    cur = last_tag.lstrip("v") if last_tag else None
    mani = manifest_version()

    # Phase separation: release-pr only PROPOSES from a clean state. If
    # the manifest is already ahead of the last tag, a release is
    # prepared and pending its tag (a release PR was merged but not yet
    # tagged, e.g. because a build is still retrying). In that state the
    # `gate` job owns the outcome (build -> tag); release-pr must stand
    # down, or it opens a PR proposing the very version main is already
    # releasing. Clean state <-> pending state are mutually exclusive.
    if cur is not None and mani != cur:
        print(f"v{mani} already prepared (last tag v{cur}); a release is "
              f"pending its tag - release-pr stands down")
        emit("release", "false")
        return

    nxt = sh("git-cliff", "--bumped-version").lstrip("v")
    print(f"last tag: {last_tag or '(none)'} | manifest: {mani} | "
          f"git-cliff next: {nxt}")

    if cur == nxt:
        print("no releasable commits since last tag — nothing to do")
        emit("release", "false")
        return

    # 1. product version across every crate's [package].
    for crate in CRATES:
        p = ROOT / crate / "Cargo.toml"
        txt = p.read_text(encoding="utf-8")
        new = re.subn(r'(?m)^version = "[^"]+"', f'version = "{nxt}"', txt, count=1)
        if new[1] != 1:
            raise SystemExit(f"could not bump version in {p}")
        p.write_text(new[0], encoding="utf-8")

    # 2. internal path-dep requirements in root [workspace.dependencies].
    rp = ROOT / "Cargo.toml"
    rt = rp.read_text(encoding="utf-8")
    for dep in INTERNAL_DEPS:
        rt, n = re.subn(
            rf'({re.escape(dep)} = \{{ path = "[^"]+", version = ")[^"]+(" \}})',
            rf'\g<1>{nxt}\g<2>', rt)
        if n != 1:
            raise SystemExit(f"could not bump internal dep requirement: {dep}")
    rp.write_text(rt, encoding="utf-8")

    # 3. changelog (full regen; git-cliff places unreleased commits under
    #    the new tag using cliff.toml).
    sh("git-cliff", "--tag", f"v{nxt}", "-o", "CHANGELOG.md")

    # 4. lockfile: workspace members only (fast — no full re-resolve).
    sh("cargo", "update", "--workspace", check=False)

    emit("release", "true")
    emit("version", nxt)
    print(f"prepared release v{nxt}")


if __name__ == "__main__":
    main()
