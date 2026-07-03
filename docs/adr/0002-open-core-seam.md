# ADR 0002 — Open-core seam: one trait, consulted once at serve boot

Status: accepted (decided 2026-05-27, recorded retroactively 2026-07-02)

## Context

ministr splits into a public MIT repository (the complete local product) and
a private repository (cloud/enterprise features). The split needed a
mechanism for the private code to extend the public binary without the MIT
tree referencing proprietary crates — and without scattering `#[cfg]` flags
or feature gates across the workspace.

## Decision

All optional non-MIT functionality attaches through exactly one seam: the
`CloudRouterMounter` trait in `ministr-api`, consulted once at HTTP-serve
boot (`ministr-cli`'s `cmd_serve_http`, which is exposed as a library entry
point for downstream binaries).

- The public `ministr` binary always passes `None` — self-hosting with no
  cloud configuration is the fully supported mode, not a degraded one.
- A downstream binary wires in a private implementation and reuses the
  entire serve flow.
- The MIT workspace never depends on proprietary crates; `ministr-cli`
  builds a complete local binary on its own.

## Consequences

- One grep-able attachment point instead of scattered conditional
  compilation; the boundary is auditable in a single file.
- The stdio (local, default) path never consults the seam at all — cloud
  concerns cannot leak into the local product's hot path.
- Residual inline cloud environment wiring in `cmd_serve_http` predates the
  seam and migrates behind it progressively; the invariant that holds today
  is "no proprietary crates compiled into the MIT binary," not "no cloud
  code paths."
- Version source of truth stays in the public repository; the private side
  builds against the MIT crates.
