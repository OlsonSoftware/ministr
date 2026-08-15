# Stability and compatibility

What ministr promises not to break, and what it explicitly reserves the right
to change.

ministr follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Within the 1.x line, a **minor** release may add surfaces but never breaks a
stable one; a **patch** release only fixes behaviour. Breaking a stable surface
requires a major release.

## Stable surfaces

These are covered by the compatibility promise from 1.0 onward.

| Surface | What is guaranteed |
|---|---|
| **MCP tool names** | A tool listed in the [tool reference](tools/README.md) keeps its name |
| **MCP tool inputs** | Existing parameters keep their names, types, and meaning. New parameters are optional |
| **MCP tool outputs** | Existing fields in structured content keep their names, types, and meaning. New fields may be added, so parse permissively |
| **CLI subcommands** | Existing subcommands and flags keep working. New flags default to today's behaviour |
| **Configuration keys** | Keys documented in the [configuration guide](../guides/configuration.md) keep their names and semantics |
| **Daemon HTTP API** | Routes documented in the [HTTP API reference](http-api.md) keep their paths and response shapes |
| **Data directory** | An index written by one 1.x version is readable by later 1.x versions, or is transparently rebuilt |

Adding a new tool, a new optional parameter, or a new response field is **not**
a breaking change. Clients must ignore fields they do not recognise.

## Unstable surfaces

These may change in any release, including a patch.

| Surface | Why it is excluded |
|---|---|
| **`ministr ui`** (terminal console) | Under active development. Layout, keys, and the screens themselves are still moving |
| **Windows builds** | Published but not covered by continuous integration — see [supported platforms](../getting-started/installation.md#supported-platforms) |
| **Rust crate APIs** | All workspace crates are `publish = false`. ministr is distributed as a binary, not a library. Item visibility is an implementation detail, and the crates are not on crates.io by design |
| **Scoring and ranking** | Which results come back first, and their scores, will keep improving. The result *shape* is stable; the ordering is not |
| **Log and trace output** | Format and verbosity may change at any time |
| **Anything marked experimental** | In `--help` text or docs |

## Removing something stable

A stable surface is never removed without warning:

1. It is marked deprecated in the release notes and in its own documentation,
   and keeps working unchanged.
2. It keeps working for at least one further minor release.
3. It is removed only in a major release.

Where a rename is involved, the old name keeps working for the deprecation
window and the release notes give the `old → new` mapping.

## Prereleases

Versions carrying a SemVer prerelease suffix (`-beta.N`, `-rc.N`) are published
as GitHub prereleases. They carry the same compatibility intent as the release
they lead to, but bugs are expected and the promises above are aspirational
until the matching stable release ships.

`install.sh` and `install.ps1` install the newest **stable** release by default.
While only prereleases exist, they fall back to the newest prerelease. Pin an
exact version with `MINISTR_VERSION`.

## Reporting a break

If an upgrade within a minor or patch release breaks a surface listed as
stable, that is a bug, not a policy change — please
[open an issue](https://github.com/OlsonSoftware/ministr/issues) with the two
versions and what changed.
