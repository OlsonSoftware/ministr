# Vendored: tree-sitter-unreal-cpp

This directory is a **verbatim, in-tree copy** of the
`tree-sitter-unreal-cpp` crate — a strict superset of `tree-sitter-cpp`
that additionally recognizes Unreal Engine reflection macros (`UCLASS`,
`UFUNCTION`, `GENERATED_BODY`, …) as first-class syntax nodes.

| | |
|---|---|
| Upstream | https://github.com/taku25/tree-sitter-unreal-cpp |
| Pinned rev | `92eee7d1ac994e408c208bcb1b73170c8746356f` |
| Crate version | `0.23.4` |
| License | MIT (see `LICENSE`) |

## Why vendored

It was previously a `git = …` Cargo dependency. The upstream repo is not
on crates.io, and its git history is pathologically large — cloning it
(which `cargo` does for every cold build / CI runner) is prohibitively
slow. Vendoring the crate's source removes the git fetch entirely:
deterministic, offline-buildable, CI-friendly.

## What's here

Exactly the crate's own `include` set needed to build:

- `Cargo.toml`, `LICENSE`
- `bindings/rust/{build.rs,lib.rs}` — the Rust binding + `cc` build
- `src/parser.c`, `src/scanner.c`, `src/tree_sitter/*.h` — the generated
  parser (large; generated, not hand-edited)
- `src/{grammar.json,node-types.json}`, `queries/*`, `grammar.js`,
  `tree-sitter.json` — referenced by `lib.rs` / kept for completeness

Consumed via `tree-sitter-unreal-cpp = { path = "third_party/..." }` in
the workspace `Cargo.toml`, gated behind `ministr-core`'s `lang-cpp`
feature.

## Updating

To bump to a newer upstream rev, re-copy the same file set from a
checkout of the new commit (do **not** add a git submodule or
dependency — the clone cost is the whole reason this is vendored),
update the rev/version above, and run `cargo update -p
tree-sitter-unreal-cpp`.
