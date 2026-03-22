# Core Concepts

iris is built around five mechanisms drawn from CPU cache controller architecture. Each addresses a specific failure mode in how LLM agents consume context.

| Mechanism | CPU Analogy | Problem Solved |
|---|---|---|
| [Session Shadow](session-shadow.md) | Cache directory | Re-delivering identical context every turn |
| [Prefetch Engine](prefetch-engine.md) | Hardware prefetcher | High-latency cold retrievals |
| [Budget Management](budget-management.md) | Replacement policy | Silent context eviction and thrashing |
| [Coherence](coherence.md) | Cache coherence protocol | Stale context from changed documents |

These mechanisms work together. The session shadow feeds the prefetch engine (knowing what was delivered informs what to pre-warm). Budget management uses the shadow to estimate window pressure. Coherence checks the shadow to determine which sessions hold stale content.
