# Agent-efficiency evaluation

This fixture set drives the deterministic MCP task-level regression gate. It
measures whether an agent can reach a correct navigation answer with bounded
calls and payload, rather than scoring retrieval rank in isolation.

`tasks.json` is the executable task contract. The Rust runner exercises the
real in-process MCP transport for navigation tasks and uses committed protocol
traces for failure/completeness states that require a multi-corpus fan-out.
Every payload is counted exactly as a serialized `CallToolResult`, including
both `content` and `structuredContent`.

CI latency is modeled as a fixed 40 ms transport round trip per call until the
first required identity appears. Wall-clock timing is printed only by the
opt-in real-agent harness because shared-runner timing is not deterministic.

The committed floors come from the July 2026 fixture baseline: all 11 tasks
completed in 12 calls using 7,375 literal MCP response tokens (1.492 correct
tasks per 1,000 tokens), with zero repeated deliveries and zero incorrect
absence claims. Compound inspect used one call and 824 tokens versus four
calls and 1,129 tokens for the equivalent granular workflow: 75% fewer calls and
27.0% fewer response tokens. CI gates at 1.3 completions per 1,000 tokens, 70%
inspect call savings, and 25% inspect token savings to leave bounded headroom
for harmless serialization changes.

The schema-economy companion gate measures the complete serialized
`tools/list`, including output schemas, and separately preserves the prior
name/description/input-only comparison. The current 26-tool catalog is 37,415
literal tokens and 4,447 legacy-comparable tokens, or 171.0 comparable tokens
per tool versus the pre-overhaul 239.2. Despite keeping 30% more tools
discoverable, the comparable total fell 7.0% and per-tool routing cost fell
28.5%; selection assertions still distinguish vague survey, exact symbol
lookup, and bounded inspect.

The two corpora intentionally contain the same logical section ID,
`docs/routing.md#dispatch-contract`. Their corpus locators differ; a correct
identity implementation keeps both deliveries distinct while deduplicating a
repeat from the same corpus and resolution.

The same gate starts two real daemon IPC servers to prove linked-project
deduplication, saved-token accounting, exact drop/re-delivery, collision-safe
identities, and mixed-success fan-out. A live ingestion-progress scenario also
proves that an empty symbol lookup is partial and non-conclusive while indexing.

Run the deterministic gate:

```sh
just eval-agent-efficiency
```

The real-model side-by-side benchmark remains opt-in because it uses external
repos, an authenticated agent CLI, and paid model calls:

```sh
just eval-agent-efficiency-real --dry-run --models haiku,sonnet
```
