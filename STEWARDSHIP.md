# ministr stewardship

This document is ministr's open-core posture and our public commitment to
contributors and users. It is borrowed in shape — and partly in phrasing —
from [GitLab's stewardship handbook](https://handbook.gitlab.com/handbook/company/stewardship/),
because the model works: a permissive open-core that runs locally for free,
forever, alongside a commercial cloud and enterprise offering that funds the
work.

## The promise

**When a feature is open source, we won't move that feature to a paid tier.**

A feature that ships under MIT in this repository stays under MIT. We may
remove a feature outright if the underlying capability is being removed from
the whole product. We will not paywall existing open-source functionality.

## What is MIT (and stays MIT)

The local stack — everything that runs on a user's own machine — is
MIT-licensed. The six MIT crates are:

| Crate | Role |
|---|---|
| [`ministr-core`](ministr-core/) | Domain logic — indexing, embedding, SOLID detector, cross-language bridge graph, 13 bridge kinds, ~40 language parsers, claim extraction, session shadow, coherence |
| [`ministr-api`](ministr-api/) | Shared request/response types |
| [`ministr-daemon`](ministr-daemon/) | HTTP API over Unix domain socket |
| [`ministr-mcp`](ministr-mcp/) | MCP server adapter (all 20 MCP tools) |
| [`ministr-cli`](ministr-cli/) | Binary entry point + `ministr serve` |
| [`ministr-app/src-tauri`](ministr-app/src-tauri/) | Desktop app (Tauri v2, macOS/Windows/Linux) |

A user who runs `ministr serve --transport http --oauth` on their own box
gets the complete tool surface, OAuth issuer included, private-repo PAT path
included, bundle export/import included, and all 20 MCP tools included. This
will remain true.

## What is closed (and why)

The hosted **ministr Cloud** service at `mcp.ministr.ai` and the **Enterprise**
on-prem image are paid products. The code that exists *only because* we run a
multi-tenant service or sell an enterprise SKU lives in proprietary crates in
a separate **private sibling repository** at
`github.com/OlsonSoftware/ministr-private` (owner-only). That repo hosts:

| Crate | License | Purpose |
|---|---|---|
| `ministr-cloud` | LicenseRef-Proprietary | Tenant data model, Stripe glue, GitHub-OAuth adapter, quota middleware, billing portal |
| `ministr-atlas` | LicenseRef-Proprietary | Curated repo list, scheduler, re-index cron, license filter, opt-out registry |
| `ministr-cloud-tools` | LicenseRef-Proprietary | Operator CLI: atlas reindex, audit retention, license mint/rotate/revoke, deployment diagnostics |
| `ministr-cli-cloud` | LicenseRef-Proprietary | Official cloud-capable `ministr` binary (5-line shim over `ministr_cloud_tools::run`) |

The split happened in F31.3–F31.4 (2026-05-27) via `git filter-repo`. Before
that, the three closed crates were source-available in this repository; they
were moved out wholesale, with full authorship history, into the private
sibling. Existing clones from before that date still contain the
source-available history — the scrub is forward-looking for new clones.

None of this code is useful on the local stack — it only exists because we
run a multi-tenant service or sell into compliance-bound buyers. Keeping it
closed is how the cloud and enterprise products fund the open core.

## What this means in practice

- **Forks are welcome.** MIT explicitly permits commercial use, modification,
  and redistribution. We ask only that the copyright notice is preserved.
- **The MCP tool surface is open.** All 20 tools — `ministr_survey`,
  `ministr_symbols`, `ministr_definition`, `ministr_references`,
  `ministr_read`, `ministr_extract`, `ministr_toc`, `ministr_bridge`,
  `ministr_compress`, `ministr_usage`, `ministr_dropped`, `ministr_solid`,
  `ministr_impact`, `ministr_dead`, `ministr_related`, `ministr_clone`,
  `ministr_fetch`, `ministr_refresh`, `ministr_task`, `ministr_projects`
  — are MIT.
- **Self-host is fully featured.** Running ministr on your own box gives you
  the same indexing, the same parsers, the same SOLID detector, the same
  bridge graph, and the same agent primitives that the cloud uses. The cloud
  sells *hosting + scale + team + compliance*, not the toolset itself.
- **No relicensing trap.** Contributors do not assign copyright to a single
  entity; contributions remain owned by the contributor and licensed inbound
  under the same MIT license as outbound (the standard
  inbound=outbound model). We will not relicense the OSS crates to a
  source-available or commercial license.

## Release pipeline

F31.5 (2026-05-27): the released `ministr` binary is built in
ministr-private and published to **this** repo's
[Releases page](https://github.com/OlsonSoftware/ministr/releases) at
the same `v<X.Y.Z>` tag. The build chain:

1. The Copilot coding agent authors a `chore: release vX.Y.Z` PR in
   this repo (bumps Cargo.toml versions + writes CHANGELOG) — version
   source of truth stays here.
2. Merging the PR pushes tag `v<X.Y.Z>` on this repo's main and fires
   a `repository_dispatch` event into ministr-private's
   `release.yml`.
3. ministr-private's workflow clones BOTH repos as siblings, builds
   `ministr-cli-cloud` (binary name `ministr`) against the MIT crates
   that live here, runs the same macOS signing + notarization path,
   then uploads the cloud-capable artifacts to this repo's release at
   `v<X.Y.Z>` via a cross-repo PAT.

`install.sh` (`curl -fsSL https://ministr.ai/install.sh | bash`) and
`homebrew/ministr.rb` are unchanged — they fetch from this repo's
releases page. MIT contributors can also build the local-only binary
directly with `cargo build -p ministr-cli --release` from this
workspace.

The deterministic "no release artifacts without a green build"
property is preserved by ministr-private's matrix: every shard must
pass before the cross-repo upload step runs.

## Why we publish this

Sourcegraph killed Cody Free and Cody Pro in July 2025 and went
Enterprise-only. That move is the cautionary tale that motivates this
document. Open-core trust is a reputational moat, not a marketing slogan;
publishing this posture explicitly is the price of buying into it.

If we ever break this commitment, hold us to it.

---

*Borrowed from GitLab's stewardship handbook, with thanks. See
[handbook.gitlab.com/handbook/company/stewardship](https://handbook.gitlab.com/handbook/company/stewardship/).*
