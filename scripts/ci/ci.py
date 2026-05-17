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
    exts = (".tar.gz", ".tgz", ".zip", ".pkg", ".exe", ".deb", ".rpm", ".AppImage")
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


def cmd_pkg(a: argparse.Namespace) -> None:
    """Build a signed + notarized macOS .pkg from the Tauri .app.

    Single Python implementation of the old inline release.yml bash:
    pkgbuild (component + postinstall CLI symlink) -> productbuild
    (signed) -> notarytool --wait -> stapler. Env carries the Apple
    secrets (same names as before). macOS only."""
    if sys.platform != "darwin":
        sys.exit("pkg is macOS-only")
    import uuid

    env = os.environ
    app = REPO / "target/aarch64-apple-darwin/release/bundle/macos/ministr.app"
    if not app.is_dir():
        sys.exit(f"ministr.app not found at {app}")
    # Under build-then-tag, this workflow runs on the `main` push BEFORE
    # any tag exists, so GITHUB_REF_NAME is "main" (was producing
    # `pkgbuild --version main`). The product version is the single
    # workspace version in the manifest (same source the release gate
    # uses); fall back to a tag-style ref only if that can't be read.
    version = next(
        (ln.split('"')[1] for ln in
         (REPO / "ministr-cli" / "Cargo.toml").read_text(encoding="utf-8").splitlines()
         if ln.startswith('version = "')),
        env.get("GITHUB_REF_NAME", "0.0.0").lstrip("v"),
    )
    print(f"pkg version: {version}", flush=True)
    rt = Path(env["RUNNER_TEMP"])
    keychain = rt / "installer-signing.keychain-db"
    kc_pw = str(uuid.uuid4())
    cert = rt / "installer-cert.p12"
    scripts = rt / "pkg-scripts"
    scripts.mkdir(parents=True, exist_ok=True)
    (scripts / "postinstall").write_text(
        "#!/bin/bash\nset -e\n"
        "TARGET=/Applications/ministr.app/Contents/MacOS/ministr-cli\n"
        "LINK=/usr/local/bin/ministr\nmkdir -p /usr/local/bin\n"
        'if [ -L "$LINK" ]; then\n'
        '  cur=$(readlink "$LINK")\n'
        '  if [ "$cur" = "$TARGET" ]; then ln -sf "$TARGET" "$LINK"; '
        'else echo "ministr: leaving $LINK ($cur)" >&2; fi\n'
        'elif [ -e "$LINK" ]; then echo "ministr: leaving $LINK" >&2\n'
        'else ln -s "$TARGET" "$LINK"; fi\nexit 0\n',
        encoding="utf-8",
    )
    os.chmod(scripts / "postinstall", 0o755)

    def sh(cmd: list[str], *, redact: bool = False, **kw):
        # Never echo a command line that contains a secret arg (cert
        # password / keychain password / Apple notary password). GitHub
        # masks known secret strings, but don't rely on that — print
        # only the program name for redacted calls.
        print(f"+ {cmd[0]} (args redacted)" if redact else "+ " + " ".join(cmd), flush=True)
        subprocess.run(cmd, check=True, **kw)

    try:
        sh(["security", "create-keychain", "-p", kc_pw, str(keychain)], redact=True)
        sh(["security", "set-keychain-settings", "-lut", "21600", str(keychain)])
        sh(["security", "unlock-keychain", "-p", kc_pw, str(keychain)], redact=True)
        existing = subprocess.check_output(
            ["security", "list-keychains", "-d", "user"], text=True
        )
        kcs = [x.strip().strip('"') for x in existing.split()] + [str(keychain)]
        sh(["security", "list-keychains", "-d", "user", "-s", *kcs])
        cert.write_bytes(
            __import__("base64").b64decode(env["APPLE_INSTALLER_CERTIFICATE"])
        )
        # Import the FULL identity (cert + private key) from the .p12.
        # `-t cert` (the old flag) restricts the import to certificate
        # items, so the private key is dropped and `productbuild` finds
        # "no appropriate signing identity". Drop `-t`, and grant the
        # signing tools access via `-T` instead of blanket `-A`.
        sh([
            "security", "import", str(cert), "-P",
            env["APPLE_INSTALLER_CERTIFICATE_PASSWORD"],
            "-f", "pkcs12", "-k", str(keychain),
            "-T", "/usr/bin/productbuild",
            "-T", "/usr/bin/pkgbuild",
            "-T", "/usr/bin/codesign",
        ], redact=True)
        sh([
            "security", "set-key-partition-list", "-S",
            "apple-tool:,apple:,codesign:",
            "-s", "-k", kc_pw, str(keychain),
        ], redact=True)
        # Diagnostic (non-fatal): list the identities actually present
        # so a cert/identity-string mismatch in the secrets is obvious
        # from the log instead of needing another blind iteration.
        subprocess.run(
            ["security", "find-identity", "-v", str(keychain)], check=False
        )
        comp = rt / "ministr-component.pkg"
        sh([
            "pkgbuild", "--component", str(app), "--install-location",
            "/Applications", "--identifier", "ai.ministr.desktop",
            "--version", version, "--scripts", str(scripts), str(comp),
        ])
        out = REPO / "_bundles"
        out.mkdir(exist_ok=True)
        dist = out / "ministr-desktop-aarch64-apple-darwin.pkg"
        sh([
            "productbuild", "--package", str(comp), "--sign",
            env["APPLE_INSTALLER_SIGNING_IDENTITY"], "--keychain",
            str(keychain), str(dist),
        ])
        sh([
            "xcrun", "notarytool", "submit", str(dist), "--apple-id",
            env["APPLE_ID"], "--password", env["APPLE_PASSWORD"],
            "--team-id", env["APPLE_TEAM_ID"], "--wait",
        ], redact=True)
        sh(["xcrun", "stapler", "staple", str(dist)])
        sha256_companion(dist)
    finally:
        subprocess.run(
            ["security", "delete-keychain", str(keychain)],
            stderr=subprocess.DEVNULL,
        )


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

    ck = sub.add_parser("checksums")
    ck.add_argument("--dir", required=True)
    ck.set_defaults(fn=cmd_checksums)

    sub.add_parser("pkg").set_defaults(fn=cmd_pkg)

    args = p.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
