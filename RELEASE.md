# Releasing ministr

End-to-end checklist for cutting a release. The repo is private and
distribution is intentionally limited to a single GitHub Release per
version, fronted by the `dl.ministr.app` Cloudflare Worker. crates.io
publishing and the Homebrew tap are explicitly disabled — those would
require making the source repo public again.

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
  ministr-core, ministr-daemon, ministr-mcp, ministr-cli,
  ministr-app/src-tauri). Tauri reads its bundle version straight from
  `ministr-app/src-tauri/Cargo.toml` (no `version` field in
  `tauri.conf.json`), so this single bump is enough.
- Prepends a `## [X.Y.Z] — YYYY-MM-DD` section to `CHANGELOG.md` with
  empty `### Added / Changed / Fixed` subsections.
- Runs `cargo check --workspace` so the bump compiles.
- Creates a `release: vX.Y.Z` commit and a `vX.Y.Z` tag.

**Review the generated CHANGELOG section before pushing.** Move any
items from the `[Unreleased]` section into the new `[X.Y.Z]` section,
and fill in anything the auto-generated template missed. Amend the
release commit if you edit it.

## 3. Trigger the unified release

```sh
git push origin main vX.Y.Z
```

`.github/workflows/release.yml` fires on `vX.Y.Z` tags. One workflow,
three job groups, one GitHub Release:

- **`cli` matrix** — Linux x86_64, Linux aarch64, macOS aarch64, Windows
  x86_64 → `ministr-<target>.tar.gz` (or `.zip` for Windows) plus
  per-file `.sha256` companions.
- **`desktop` matrix** — macOS aarch64, Windows x86_64, Linux x86_64 →
  Tauri bundles renamed to
  `ministr-desktop-<target>.<dmg|exe|deb|AppImage>`. The Windows shard
  builds the CLI sidecar with `--features directml` so the bundled
  binary uses DirectX 12 GPU embedding; the bare CLI `.zip` from the
  `cli` job stays non-DirectML for headless installs.
- **`release` job** — depends on both matrices. Downloads every
  artifact, generates a unified `SHA256SUMS`, and creates the GitHub
  Release with `softprops/action-gh-release` (auto-published; tags
  containing `-` are marked prerelease automatically).

Watch the Actions tab until the `release` job goes green. The Cloudflare
Worker at `dl.ministr.app` will start serving the new tag's assets as
soon as the Release is published — no manual sync step.

`x86_64-apple-darwin` is intentionally omitted from both matrices.
`ort-sys` 2.0.0-rc.11 dropped prebuilt binaries for that target, and
macOS 26 dropped Intel x86_64 support. Apple Silicon is the supported
Mac target; Intel users on older macOS can run via Rosetta 2 or build
from source.

### macOS code signing

`tauri-action` produces an unsigned `.dmg` unless Apple Developer ID
secrets are bound on the repo (`APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`). Configure them once on
the repo settings; they wire through the workflow automatically. See
[ministr-app/SIGNING.md](ministr-app/SIGNING.md) for details.

### macOS .pkg (optional, local-only)

The `just pkg` recipe is local-only — CI does not build the signed
`.pkg` installer. If you need one for a specific release, run it
manually from a signed-in macOS machine:

```sh
source .env.signing
just pkg
```

Then upload `target/pkg/ministr-X.Y.Z.pkg` to the GitHub Release as an
extra asset.

## 4. Announce

- Update the docs-next landing hero copy if anything changed.
- Post the release notes summary to the project README's "What's new"
  section (if it has one).

## Rolling back

- A botched `vX.Y.Z` tag can be deleted: `git tag -d vX.Y.Z && git push
  origin :refs/tags/vX.Y.Z`. Also delete the GitHub Release.
- A bad asset can be replaced by force-pushing the tag to a new commit
  and letting the workflow recreate the Release. The Cloudflare Worker
  caches release metadata for 7 days for immutable tags — replacing a
  tag's assets in place is fine, but if you actually need an older
  asset to disappear, also call the cache-purge endpoint on the Worker.
