# Releasing

Releases are cut from this repository and ship the MIT `ministr-cli` binary,
built from this source. Each `vX.Y.Z` tag is a single product version shared by
all workspace crates; the version source of truth is `ministr-cli/Cargo.toml`.
CLI archives plus a `SHA256SUMS` are attached to the matching
[GitHub Release](https://github.com/OlsonSoftware/ministr/releases).

## Versioning

[Conventional Commit](https://www.conventionalcommits.org/) messages indicate
the bump:

- `feat:` → minor
- `fix:` / `perf:` → patch
- `!` or `BREAKING CHANGE:` → major

All workspace crates share one product version, bumped in lockstep. What a
given bump promises about compatibility is defined in
[docs/reference/stability.md](docs/reference/stability.md). `crates.io`
publishing is disabled (`publish = false`): ministr is distributed as a binary,
not a library.

## Cutting a release

### 1. Bump the version

Edit these by hand — this is a text-only change, and it must be exact:

- `[package] version` in all seven crate manifests: `ministr-api`,
  `ministr-core`, `ministr-daemon`, `ministr-backend`, `ministr-mcp`,
  `ministr-cli`, `ministr-tui`. Leave `third_party/tree-sitter-unreal-cpp`
  alone — it is vendored and carries its own upstream version.
- The seven internal entries under `[workspace.dependencies]` in the root
  `Cargo.toml`, each of which carries `{ path = "...", version = "X.Y.Z" }`.
- `Cargo.lock` — update the `version` line of each `[[package]]` block whose
  `name` is one of the seven crates. Edit it as text; do **not** run
  `cargo update`, which would also churn every transitive dependency.

### 2. Write the changelog section

Add a `## X.Y.Z - <UTC date>` section at the top of `CHANGELOG.md`, in
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/) form, grouped under
`### Added / Changed / Fixed / Removed`. Write it for humans — one bullet per
real change, never raw squashed commit subjects. Breaking changes get their own
prominent block listing every renamed or removed public surface as `old → new`.

The release workflow reads this section verbatim for the GitHub Release notes
and **fails** if it is missing or empty. That is deliberate: a release whose
notes silently degrade to "see the changelog" is how changelogs rot.

### 3. Validate and commit

```sh
just validate
git commit -am "chore: release vX.Y.Z"
git push origin main
```

The commit subject matters — the workflow uses it to confirm this is a
deliberate release commit.

### 4. Run the release workflow

Actions → **Release automation** → Run workflow. It runs:

```
state → gate → build → tag → publish
```

`build` calls `release.yml`, which builds the four target archives and their
checksums. `tag` needs `build`, so **a tag can only ever exist for a commit
whose entire release matrix went green** — a failed build leaves nothing to
clean up. `publish` then creates the GitHub Release and attaches every
artifact.

A version carrying a SemVer prerelease suffix (`1.0.0-beta.1`) is published as
a GitHub prerelease automatically, which keeps `releases/latest` pointing at
the newest stable release.

### 5. Verify

- The Release lists all four archives, their `.sha256` files, and `SHA256SUMS`.
- `curl -fsSL https://ministr.ai/install.sh | bash` installs it.
- `ministr --version` reports the new version.

## Recovery

**Build failed.** Nothing was tagged or published. Fix and re-run.

**Build succeeded, publish failed.** The tag exists. Re-run the workflow with
`force: true` — asset upload is idempotent (existing assets are clobbered, the
release is not recreated).

**Wrong version shipped.** Do not delete a published release that people may
have installed. Bump to the next patch and release again.

## Building from source

Any commit builds a working local binary:

```sh
cargo install --path ministr-cli --locked
```

## What is not in this pipeline

`ministr-private` has its own `release.yml` for a possible future cloud
artifact. It is **not** on the public download path and must never publish to
this repository's Releases under the `ministr` binary name — that would put a
proprietary build behind an MIT project's install instructions.
