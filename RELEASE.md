# Releasing ministr

> ## The release contract (post-F31.5)
>
> **The only supported way to cut a release:**
> 1. Land work on `main` (Conventional Commit messages)
> 2. The `request-release` workflow detects releasable commits and
>    hands the job to the Copilot coding agent, which decides the
>    semver bump, updates crate versions in lockstep, authors a
>    CHANGELOG section, and opens a `chore: release vX.Y.Z` PR
> 3. You review + **merge that PR**
> 4. `release-automation.yml` pushes the `vX.Y.Z` tag here, then
>    fires `repository_dispatch` into `OlsonSoftware/ministr-private`
> 5. `ministr-private/release.yml` clones BOTH repos as siblings,
>    builds every release artifact (cloud-capable `ministr` binary +
>    Tauri bundles), and uploads them to **this repo's** release at
>    `v<X.Y.Z>` via a cross-repo PAT
>
> **Never** create, move, or delete `v*` tags by hand. Tags are a
> consequence of the release-bump merge — never hand-made.

Releases are automated via
[`.github/workflows/release-automation.yml`](.github/workflows/release-automation.yml)
here + `ministr-private/.github/workflows/release.yml` in the private
sibling. You don't hand-bump versions or hand-write the changelog —
you just review and merge a bot PR. Distribution is via GitHub
Releases on **this repo**; the private workflow uploads to it via a
cross-repo PAT. `crates.io` publishing stays disabled
(`publish = false`) until the API surface stabilises.

The split happened in F31 (2026-05-27): the proprietary crates
(`ministr-cloud`, `ministr-atlas`, `ministr-cloud-tools`) moved to
the private sibling. The MIT crates above still build a complete
local-only `ministr` from this workspace via `cargo build`.

## How it works

```
push to main ─▶ request-release     detects releasable commits,
  (Conventional                     opens a release PR with:
   Commit msgs)                     version bump (all crates, lockstep)
                                    + CHANGELOG.md section
        │
   merge the PR ─▶ tag              pushes v<X.Y.Z> here
        │       └▶ dispatch-private repository_dispatch into
        │                           OlsonSoftware/ministr-private
        ▼                                       │
   (this repo)                                  ▼
                                  ministr-private/release.yml:
                                    builds CLI + desktop matrix
                                    signs + notarizes macOS
                                    uploads to THIS repo's release
                                    via PUBLIC_RELEASE_TOKEN PAT
```

- **Conventional Commits drive everything.** `feat:` → minor + *Added*,
  `fix:` → patch + *Fixed*, `feat!:`/`BREAKING CHANGE:` → major.
- All workspace crates here are pinned to one shared version — a
  single product version. Tauri reads its bundle version from
  `ministr-app/src-tauri/Cargo.toml`, so the lockstep bump covers it.
  `ministr-private/ministr-cli-cloud` is bumped manually to match
  (or left at 0.6.0 — it path-deps into this workspace, so the
  binary version it ships is sourced from this side's manifest at
  build time).
- A failed private build never strands the release. The tag exists
  here but the release at that tag has no attached artifacts —
  re-dispatch `ministr-private/release.yml` (workflow_dispatch with
  the same version input) to retry the upload. The upload step is
  idempotent (softprops/action-gh-release appends, not replaces).

## 1. Land changes on `main`

Use Conventional Commit messages. When releasable commits accumulate,
the automation opens a Release PR with the computed version and
generated CHANGELOG section.

## 2. Pre-flight, then merge the Release PR

```sh
just release-preflight      # validate + audit + eval-gate + docs
```

All must exit 0. Then **merge the Release PR**.

## 3. The tag-then-dispatch flow (public)

`release-automation.yml` fires on the merge commit. It:

1. Gates: is the manifest version still un-tagged?
2. Tags: pushes the `vX.Y.Z` tag here
3. Dispatches: fires `repository_dispatch` into
   `OlsonSoftware/ministr-private` so its workflow can build the
   artifacts and populate this repo's release page

No artifacts are built or attached from this repo; that work moved
to ministr-private. The bundled `release.yml` here remains callable
via `workflow_dispatch` for debugging (builds the MIT-only artifacts
to validate the matrix), but its `tag-and-release` invocation path
is gone.

## 4. The build-then-publish flow (private)

`ministr-private/release.yml` receives the dispatch and:

1. Verifies `v<X.Y.Z>` exists on this repo (fails loud otherwise)
2. Runs signing-preflight (Apple creds) — cheap fast-fail
3. CLI matrix — `ministr-{linux x64,linux arm64,macOS aarch64,windows x64}` tarballs / .zip
4. Desktop matrix — Tauri bundles (.pkg / .nsis / .deb / .rpm /
   AppImage), with `ministr-cli-cloud` staged as the sidecar
5. Aggregate SHA256SUMS over every artifact
6. Upload to **this repo's** release at `v<X.Y.Z>` via
   `softprops/action-gh-release` with `repository:` input +
   `PUBLIC_RELEASE_TOKEN` (cross-repo PAT)

`x86_64-apple-darwin` is intentionally omitted from both matrices.
Apple Silicon is the supported Mac target; Intel users can run via
Rosetta 2 or build from source.

### macOS code signing

The desktop app and CLI binary both need Developer ID secrets bound
on the **private** repo (`APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_INSTALLER_CERTIFICATE`,
`APPLE_INSTALLER_CERTIFICATE_PASSWORD`,
`APPLE_INSTALLER_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`,
`APPLE_TEAM_ID`) — 9 secrets total. Same values as the ones this
repo used pre-F31.5; copy them across. See
[ministr-app/SIGNING.md](ministr-app/SIGNING.md) for the env-var
descriptions and the `just pkg` / `just pkg-dev` local workflows.

### Required secrets summary

| Repo | Secret | Purpose |
|------|--------|---------|
| this | `RELEASE_PLZ_TOKEN` | PAT for the Copilot agent + tag push |
| this | `MINISTR_PRIVATE_DISPATCH_TOKEN` | PAT, `actions: write` on ministr-private (fires the dispatch) |
| private | 9 × `APPLE_*` | macOS signing + notarization |
| private | 4 × `SCCACHE_*` + `AWS_*` | R2 sccache (optional but recommended) |
| private | `PUBLIC_RELEASE_TOKEN` | PAT, `contents: write` on this repo (uploads artifacts) |

See `ministr-private/.github/README.md` for setup details.

## 5. Announce

- Update the web landing hero copy if anything changed.
- Post the release notes summary to the project README's "What's new"
  section (if it has one).

## Rolling back

- A botched `vX.Y.Z` tag can be deleted: `git tag -d vX.Y.Z && git push
  origin :refs/tags/vX.Y.Z`. Also delete the GitHub Release on this
  repo. The private workflow has no tags of its own to clean up.
- A bad asset can be replaced by re-dispatching ministr-private's
  workflow (workflow_dispatch with the same version input) — the
  upload step is idempotent / append-only on the existing release.
- The hard recovery anchor for the F31.4 history rewrite is the
  `backup-pre-f31.4` tag on this repo's origin. `git reset --hard
  backup-pre-f31.4 && git push --force origin main` puts public main
  back at the original commit 6ca882b (before the proprietary scrub).
  Don't do this unless you actually want the proprietary crates back
  in public history.
