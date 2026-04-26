# Releasing ministr

End-to-end checklist for cutting a release. The repo publishes to four
surfaces: GitHub Releases (CLI binaries), GitHub Releases again (Tauri
app installers), crates.io (library crates), and the Homebrew tap.

Prerequisites: the signing env vars described in
[ministr-app/SIGNING.md](ministr-app/SIGNING.md) must be set before `just pkg`
is run, and a crates.io API token must be in `~/.cargo/credentials`
(`cargo login <token>`).

## 1. Pre-flight

```sh
just validate               # fmt-check + lint + test, must be green
just deny                   # license + advisory checks
cargo audit                 # vulnerability scan
just eval-gate              # retrieval quality regression gate
just docs-typecheck         # docs-next types
just docs-build             # docs-next static export
```

All must exit 0. Fix anything red before continuing.

## 2. Bump the version

```sh
just release X.Y.Z
```

This recipe:
- Updates `version = ...` in all six workspace crates (ministr-api,
  ministr-core, ministr-daemon, ministr-mcp, ministr-cli, ministr-app/src-tauri).
- Prepends a `## [X.Y.Z] — YYYY-MM-DD` section to `CHANGELOG.md` with
  empty `### Added / Changed / Fixed` subsections.
- Runs `cargo check --workspace` so the bump compiles.
- Creates a `release: vX.Y.Z` commit and a `vX.Y.Z` tag.

**Review the generated CHANGELOG section before pushing.** Move any
items from the `[Unreleased]` section into the new `[X.Y.Z]` section,
and fill in anything the auto-generated template missed. Amend the
release commit if you edit it.

## 3. Trigger the CLI release

```sh
git push origin main vX.Y.Z
```

`.github/workflows/release.yml` fires on `v*` tags. It builds
`ministr-cli` on the five-target matrix (Linux x86_64/aarch64, macOS
x86_64/aarch64, Windows x86_64) and publishes a GitHub Release with
tarballs plus SHA-256 sums.

Watch the Actions tab until all matrix jobs are green.

## 4. Trigger the Tauri app release

```sh
git tag vX.Y.Z-app
git push origin vX.Y.Z-app
```

`.github/workflows/app-release.yml` fires on `v*-app` tags and runs
`tauri-apps/tauri-action` on macOS (ARM64 + x86_64), Windows, and Linux.
This produces signed `.dmg`s (if the Apple secrets are set on the
repo — see SIGNING.md), NSIS installers, and Linux `.deb` / AppImage
bundles.

The action creates a **draft** release; review the artifacts and
publish it manually.

### macOS .pkg (optional)

The `just pkg` recipe is local-only — CI does not yet build the signed
`.pkg` installer. If you need one for this release, run it manually
from a signed-in macOS machine:

```sh
source .env.signing
just pkg
```

Upload `target/pkg/ministr-X.Y.Z.pkg` to the GitHub Release as an extra
asset.

## 5. Publish to crates.io

Publish in dependency order. Each command blocks for ~30 seconds while
the index updates; do not skip ahead:

```sh
cargo publish -p ministr-api
cargo publish -p ministr-core
cargo publish -p ministr-daemon
cargo publish -p ministr-mcp
cargo publish -p ministr-cli
```

`ministr-app` is `publish = false` — it ships as an installer, not a
library.

If a `cargo publish` fails partway through, do not re-run earlier
crates. Fix the failing one and resume.

## 6. Update the Homebrew tap

The formula lives at `homebrew/` in this repo. Copy it to the tap repo
(`OlsonSoftware/homebrew-tap`) and bump version + SHA-256:

```sh
# In OlsonSoftware/homebrew-tap
cp ~/Code/ministr/homebrew/ministr.rb Formula/ministr.rb
# Update version, URL, and sha256 to match the vX.Y.Z release tarball
brew audit --strict --online ministr
git commit -am "ministr X.Y.Z"
git push
```

Verify with `brew install OlsonSoftware/tap/ministr` on a clean machine.

## 7. Announce

- Update the docs-next landing hero copy if anything changed.
- Post the release notes summary to the project README's "What's new"
  section (if it has one).

## Rolling back

- A botched `vX.Y.Z` tag can be deleted: `git tag -d vX.Y.Z && git push
  origin :refs/tags/vX.Y.Z`. Also delete the GitHub Release draft.
- crates.io does **not** support deleting a published version — only
  `cargo yank -p <crate>@X.Y.Z`. Yanking keeps the version in the
  index but prevents new Cargo.lock files from resolving to it. If
  yank is needed, yank every crate in the release.
- Homebrew tap rollback: revert the formula commit and push.
