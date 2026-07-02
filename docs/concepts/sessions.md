# Sessions

Context is the scarce resource in agent work. ministr keeps a per-session
ledger of everything it has delivered, and uses it to never charge twice
for the same content.

## Dedup and delta delivery

- Search results already delivered are excluded before truncation — a
  repeated `ministr_survey` returns new material, with a
  `deduplicated_count` noting what was skipped.
- Re-reading an unchanged section returns a short stub instead of the full
  text.
- Re-reading a changed section is re-delivered — the agent gets the new
  content precisely because it changed underneath.

The contract has an agent-side half: after dropping content from context,
call `ministr_dropped` so future reads return full text again. To keep
something referenceable in less space, `ministr_compress` produces
extractive summaries.

## Coherence

When an indexed file changes after its content was delivered to a session,
subsequent tool responses carry `coherence_alerts` naming the affected
sections — the signal to re-read and pick up the delta. The desktop app's
[freshness](freshness.md) display and this mechanism are two views of the
same change tracking.

## Honest accounting

`ministr_usage` reports what ministr has delivered this session. The
estimate is anchored to a configured window, not the model's real context
window, and the tool's own description says so: it is advisory, and agents
are told not to use it to conclude they are low on context.

## Session identity and recovery

Sessions are tracked per MCP connection and persisted, so delivery state
survives daemon restarts.
