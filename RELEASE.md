# Releasing ministr

> ## The release contract
>
> **The only supported way to cut a release:**
> 1. Land work on `main` (Conventional Commit messages)
> 2. The `request-release` workflow detects releasable commits and
>    hands the job to a cloud agent, which decides the semver bump,
>    updates crate versions in lockstep, authors a CHANGELOG section,
>    and opens a `chore: release vX.Y.Z` PR
> 3. You review + **merge that PR**
> 4. `build-then-tag` builds every artifact; only if the build is
>    fully green does it push the `vX.Y.Z` tag and create the
>    GitHub Release with artifacts attached
>
> **Never** create, move, or delete `v*` tags by hand. Tags are a
> consequence of a green build — never hand-made.

Releases are automated via
[`.github/workflows/release-automation.yml`](.github/workflows/release-automation.yml).
You don't hand-bump versions or hand-write the changelog — you just
review and merge a bot PR. Distribution is via GitHub Releases;
crates.io publishing stays disabled (`publish = false`) until the API
surface stabilises.

## How it works

```
push to main ─▶ request-release     detects releasable commits,
  (Conventional                     opens a release PR with:
   Commit msgs)                     version bump (all crates, lockstep)
                                    + CHANGELOG.md section
        │
   merge that PR ─▶ build-then-tag  builds EVERY artifact first
        │                           ONLY tags if fully green
        ▼
  vX.Y.Z tag + GitHub Release       artifacts already built + attached
```

- **Conventional Commits drive everything.** `feat:` → minor + *Added*,
  `fix:` → patch + *Fixed*, `feat!:`/`BREAKING CHANGE:` → major.
- All workspace crates are pinned to one shared version — a single
  product version. Tauri reads its bundle version from
  `ministr-app/src-tauri/Cargo.toml`, so the lockstep bump covers it.
- A failed build never strands a tag. Nothing is tagged until green.

## 1. Land changes on `main`

Use Conventional Commit messages. When releasable commits accumulate,
the automation opens a Release PR with the computed version and
generated CHANGELOG section.

## 2. Pre-flight, then merge the Release PR

```sh
just release-preflight      # validate + audit + eval-gate + docs
```

All must exit 0. Then **merge the Release PR**.

## 3. The build-then-tag flow

`build-then-tag` fires on the merge commit. It:

1. Gates: is the manifest version still un-tagged?
2. Builds: calls `release.yml` — builds every artifact, no tag yet
3. Tags: only if the build is fully green, pushes the `vX.Y.Z` tag
   and creates the GitHub Release with artifacts attached

### Build matrix

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
  Release with `softprops/action-gh-release`.

`x86_64-apple-darwin` is intentionally omitted from both matrices.
Apple Silicon is the supported Mac target; Intel users can run via
Rosetta 2 or build from source.

### macOS code signing

`tauri-action` produces an unsigned `.dmg` unless Apple Developer ID
secrets are bound on the repo (`APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`). Configure them once on
the repo settings; they wire through the workflow automatically. See
[ministr-app/SIGNING.md](ministr-app/SIGNING.md) for details.

## 4. Announce

- Update the web landing hero copy if anything changed.
- Post the release notes summary to the project README's "What's new"
  section (if it has one).

## Rolling back

- A botched `vX.Y.Z` tag can be deleted: `git tag -d vX.Y.Z && git push
  origin :refs/tags/vX.Y.Z`. Also delete the GitHub Release.
- A bad asset can be replaced by force-pushing the tag to a new commit
  and letting the workflow recreate the Release.
