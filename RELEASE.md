# Releasing ministr

> ## ⚠️ THE RELEASE CONTRACT — read this first
>
> **The only supported way to cut a release:**
> 1. land work on `main` (Conventional Commit messages)
> 2. release-plz keeps **one** `chore: release` PR updated (version + CHANGELOG)
> 3. you review + **merge that PR**
> 4. release-plz tags `vX.Y.Z` *because of step 3* (`release_always=false`) → `release.yml` builds + publishes
>
> **HARD INVARIANT — never violate:** do **not** create, move, or delete
> `v*` tags by hand, and do **not** hand-edit crate versions. `main`'s
> version always equals the last released tag; the *only* thing that
> introduces a new untagged version is merging the release PR. Every
> release outage in this repo's history came from breaking this. If a
> `chore: release` PR looks wrong, **close it** — the next push to `main`
> regenerates it. Never hand-merge a stale one.

Releases are **automated by [release-plz](https://release-plz.dev)**
(config: [`release-plz.toml`](release-plz.toml), workflow:
[`.github/workflows/release-plz.yml`](.github/workflows/release-plz.yml)).
You don't hand-bump versions or hand-write the changelog or tag — you
just merge a bot PR. The repo is private and distribution is
intentionally limited to a single GitHub Release per version, fronted by
the `dl.ministr.app` Cloudflare Worker; crates.io publishing and the
Homebrew tap stay disabled (`publish = false`).

## How it works

```
push to main ─▶ release-plz-pr     keeps a "release" PR updated:
  (Conventional               version bump (all 6 crates, lockstep)
   Commit msgs)               + CHANGELOG.md from the commit log
        │
   merge that PR ─▶ release-plz-release   pushes ONE `vX.Y.Z` tag
        │                                  (no crates.io, no GH Release)
        ▼
  release.yml (tag trigger)   builds every artifact + SHA256SUMS and
                              creates the single GitHub Release
```

- **Conventional Commits drive everything.** `feat:` → minor + *Added*,
  `fix:` → patch + *Fixed*, `feat!:`/`BREAKING CHANGE:` → major;
  `ci:`/`chore:`/`build:`/`test:` are kept out of the changelog.
- All six crates (ministr-api/core/daemon/mcp/cli/app) are pinned to one
  shared version via `version_group` — a single product version, exactly
  like before. Tauri reads its bundle version from
  `ministr-app/src-tauri/Cargo.toml`, so the lockstep bump covers it.
- Exactly one tag, `vX.Y.Z` (ministr-cli owns it), so the existing
  `release.yml` tag filter fires unchanged.

## 1. Land changes on `main`

Use Conventional Commit messages. As commits land, the **Release PR**
(labelled `release`) is opened/refreshed automatically with the computed
version and the generated `CHANGELOG.md` diff. Review that PR's
changelog as you would any PR.

## 2. Pre-flight, then merge the Release PR

```sh
just release-preflight      # validate + deny + eval-gate + audit + docs
```

All must exit 0. Then **merge the Release PR**. That's the entire
release action — `release-plz-release` then pushes the `vX.Y.Z` tag.

> Preview locally any time without side effects:
> `release-plz update --config release-plz.toml` (prints the version +
> changelog diff it would make).

> **Token:** the workflow needs the `RELEASE_PLZ_TOKEN` repo secret (a
> fine-grained PAT with *Contents: RW* + *Pull requests: RW*). The
> default `GITHUB_TOKEN` cannot trigger other workflows, so without the
> PAT the pushed tag would not start `release.yml`.

## 3. The tag-triggered build (unchanged)

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
