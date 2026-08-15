# Releasing

Releases are cut from this repository's tags. Each `vX.Y.Z` tag is a single
product version shared by all workspace crates; the version source of truth is
`ministr-cli/Cargo.toml`. CLI archives are attached to the matching
[GitHub Release](https://github.com/OlsonSoftware/ministr/releases).

## Versioning

[Conventional Commit](https://www.conventionalcommits.org/) messages drive the
version bump and the changelog:

- `feat:` → minor
- `fix:` / `perf:` → patch
- `!` or `BREAKING CHANGE:` → major

All workspace crates share one product version, bumped in lockstep.
`crates.io` publishing is disabled (`publish = false`) while the API
surface stabilizes.

## Building from source

Any commit builds a working local binary:

```sh
cargo install --path ministr-cli --locked
```
