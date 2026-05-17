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
    python scripts/ci/ci.py stage-sidecar --target T --src ministr.exe --dst NAME
    python scripts/ci/ci.py collect-bundles --target-dir D --triple TR
"""
from __future__ import annotations

import argparse
import hashlib
import os
import shutil
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
    src = REPO / "target" / a.target / "release" / a.binary
    if not src.is_file():
        sys.exit(f"binary not found: {src}")
    dst = REPO / a.archive
    with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
        z.write(src, arcname=src.name)
    print(f"packaged {src} -> {dst}")
    sha256_companion(dst)


def cmd_stage_sidecar(a: argparse.Namespace) -> None:
    src = REPO / "target" / a.target / "release" / a.src
    if not src.is_file():
        sys.exit(f"sidecar not found: {src}")
    dest_dir = REPO / "ministr-app" / "src-tauri" / "binaries"
    dest_dir.mkdir(parents=True, exist_ok=True)
    dst = dest_dir / a.dst
    shutil.copy2(src, dst)
    print(f"staged sidecar {src} -> {dst}")


def cmd_collect_bundles(a: argparse.Namespace) -> None:
    """Rename Tauri outputs to ministr-desktop-<triple>.<ext> (+ sha256)."""
    out = REPO / "_bundles"
    out.mkdir(parents=True, exist_ok=True)
    base = f"ministr-desktop-{a.triple}"
    rename = {
        ".exe": f"{base}-setup.exe",
        ".deb": f"{base}.deb",
        ".rpm": f"{base}.rpm",
        ".AppImage": f"{base}.AppImage",
    }
    target_dir = (REPO / a.target_dir).resolve()
    found = 0
    for p in target_dir.rglob("*"):
        if not p.is_file():
            continue
        dest_name = rename.get(p.suffix)
        if not dest_name:
            continue
        dst = out / dest_name
        shutil.copy2(p, dst)
        print(f"collected {p} -> {dst}")
        sha256_companion(dst)
        found += 1
    if found == 0:
        sys.exit(f"no .exe/.deb/.rpm/.AppImage bundles found under {target_dir}")


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

    ss = sub.add_parser("stage-sidecar")
    ss.add_argument("--target", required=True)
    ss.add_argument("--src", required=True)
    ss.add_argument("--dst", required=True)
    ss.set_defaults(fn=cmd_stage_sidecar)

    cb = sub.add_parser("collect-bundles")
    cb.add_argument("--target-dir", required=True)
    cb.add_argument("--triple", required=True)
    cb.set_defaults(fn=cmd_collect_bundles)

    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
