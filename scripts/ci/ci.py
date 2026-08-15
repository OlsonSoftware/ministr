#!/usr/bin/env python3
"""Idempotent CI build helpers for the self-hosted Windows release path.

The Windows release runner uses ZERO bash (its `shell: bash` is the
System32 WSL stub, which exits 1 with no distro). win_setup.ps1
guarantees Python + Rust; this script then carries all build logic that
the Linux/macOS shards do in bash. Pure stdlib, cross-platform, and each
subcommand is safe to re-run.

    python scripts/ci/ci.py lld-config
    python scripts/ci/ci.py build       --target T -p ministr-cli [--features F]
    python scripts/ci/ci.py package-cli --target T --binary ministr.exe --archive A.zip
    python scripts/ci/ci.py checksums   --dir artifacts
"""
from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def run(cmd: list[str]) -> None:
    print("+ " + " ".join(cmd), flush=True)
    r = subprocess.run(cmd, cwd=REPO)
    if r.returncode != 0:
        sys.exit(r.returncode)


def sha256_companion(path: Path) -> None:
    """Write `<hex>  <basename>` to <path>.sha256 (shasum -a 256 format)."""
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    (path.parent / f"{path.name}.sha256").write_text(
        f"{h.hexdigest()}  {path.name}\n", encoding="ascii"
    )
    print(f"sha256 {path.name}: {h.hexdigest()}")


def cmd_lld_config(_: argparse.Namespace) -> None:
    """Point the MSVC target at rust-lld (idempotent)."""
    cfg = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "config.toml"
    cfg.parent.mkdir(parents=True, exist_ok=True)
    block = (
        "[target.x86_64-pc-windows-msvc]\n"
        'linker = "rust-lld"\n'
        'rustflags = ["-Clink-arg=-fuse-ld=lld"]\n'
    )
    existing = cfg.read_text(encoding="utf-8") if cfg.exists() else ""
    if "[target.x86_64-pc-windows-msvc]" in existing:
        print(f"{cfg}: msvc target block already present — skip")
        return
    with cfg.open("a", encoding="utf-8") as f:
        if existing and not existing.endswith("\n"):
            f.write("\n")
        f.write(block)
    print(f"{cfg}: appended rust-lld config")


def cmd_build(a: argparse.Namespace) -> None:
    cmd = ["cargo", "build", "--release", "--target", a.target, "-p", a.package]
    if a.features:
        cmd += ["--features", a.features]
    run(cmd)


def cmd_package_cli(a: argparse.Namespace) -> None:
    """Archive the built CLI binary. `.zip` (Windows) or `.tar.gz`
    (Unix) is chosen by the archive name — ONE cross-platform path,
    no per-OS workflow steps."""
    src = REPO / "target" / a.target / "release" / a.binary
    if not src.is_file():
        sys.exit(f"binary not found: {src}")
    dst = REPO / a.archive
    if a.archive.endswith(".zip"):
        with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
            z.write(src, arcname=src.name)
    elif a.archive.endswith((".tar.gz", ".tgz")):
        import tarfile

        with tarfile.open(dst, "w:gz") as t:
            t.add(src, arcname=src.name)
    else:
        sys.exit(f"unsupported archive type: {a.archive}")
    print(f"packaged {src} -> {dst}")
    sha256_companion(dst)


def cmd_checksums(a: argparse.Namespace) -> None:
    """Aggregate one SHA256SUMS over every release artifact in a dir
    (replaces the inline bash in release.yml's `release` job)."""
    d = Path(a.dir).resolve()
    exts = (".tar.gz", ".tgz", ".zip")
    lines: list[str] = []
    for p in sorted(d.iterdir()):
        if p.is_file() and p.name.endswith(exts):
            h = hashlib.sha256()
            with p.open("rb") as f:
                for chunk in iter(lambda: f.read(1 << 20), b""):
                    h.update(chunk)
            lines.append(f"{h.hexdigest()}  {p.name}")
    out = d / "SHA256SUMS"
    out.write_text("\n".join(lines) + "\n", encoding="ascii")
    print(out.read_text(encoding="ascii"))


def main() -> None:
    p = argparse.ArgumentParser(prog="ci.py")
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("lld-config").set_defaults(fn=cmd_lld_config)

    b = sub.add_parser("build")
    b.add_argument("--target", required=True)
    b.add_argument("-p", "--package", required=True)
    b.add_argument("--features", default="")
    b.set_defaults(fn=cmd_build)

    pc = sub.add_parser("package-cli")
    pc.add_argument("--target", required=True)
    pc.add_argument("--binary", required=True)
    pc.add_argument("--archive", required=True)
    pc.set_defaults(fn=cmd_package_cli)

    ck = sub.add_parser("checksums")
    ck.add_argument("--dir", required=True)
    ck.set_defaults(fn=cmd_checksums)

    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
