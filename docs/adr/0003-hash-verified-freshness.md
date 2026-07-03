# ADR 0003 — Freshness verdicts are hash-verified; mtime is never trusted for reporting

Status: accepted (decided 2026-06, recorded retroactively 2026-07-02)

## Context

The desktop app's central question is "is my AI up to date with my code?"
File timestamps are cheap to check but lie in practice: editors rewrite
files preserving mtime, `git checkout` restores old timestamps, build tools
touch files without changing content. A freshness display that trusts
timestamps will sometimes claim "up to date" falsely — the one failure mode
a trust surface cannot have.

Separately, an investigation into "GUI always shows out of date"
(2026-06-13) found the freshness join failing on key-scheme mismatches
between stored records and live sweeps — and an initial fix proposal
(relative-only storage keys) was refuted when reference-tracing showed
absolute source paths are load-bearing for git blame and diff-impact. The
final design decoupled the index key from the on-disk locator.

## Decision

Two-tier policy:

- **Reporting** (anything a user sees): every freshness verdict is produced
  by hashing the working tree. There is deliberately no stat/mtime shortcut
  on this path. Cost is controlled by a short-lived daemon cache of the
  last verified sweep (below the UI poll cadence, invalidated on reindex) —
  a cached answer is still a full hash-verified sweep.
- **Indexing** (deciding what to re-process): mtime+size may serve as an
  accelerator to pick candidates, but the per-file content hash always has
  the final word, and a whole-tree check short-circuits no-op reindexes.

## Consequences

- No false "up to date": the invariant is never traded for speed; speed
  comes from caching verified results, not weakening verification.
- Index storage keys are relative/namespaced (portable, collision-managed)
  while the document locator remains an absolute path (blame and
  diff-impact need it) — two roles, two fields.
- The same change tracking feeds agents as `coherence_alerts` in tool
  responses: the UI's freshness display and the agent's staleness signal
  are two views of one mechanism.
