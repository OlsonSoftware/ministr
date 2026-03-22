# Tool Reference

iris exposes seven MCP tools that give the agent explicit control over context retrieval. The tools mirror how a human researcher navigates a knowledge base: survey, read, extract, connect.

## Tool Overview

| Tool | Purpose | Typical Token Cost | Latency |
|---|---|---|---|
| [`iris_survey`](survey.md) | Search for relevant sections | 100-300 | <50ms cold, <5ms warm |
| [`iris_read`](read.md) | Read a section in full | 200-2000 | <10ms |
| [`iris_extract`](extract.md) | Pull specific claims | 50-500 | <50ms cold, <5ms warm |
| [`iris_related`](related.md) | Follow claim relationships | 50-300 | <20ms |
| [`iris_budget`](budget.md) | Check budget status | minimal | <1ms |
| [`iris_compress`](compress.md) | Get compressed summaries | 50-200 | <10ms |
| [`iris_evicted`](evicted.md) | Signal evicted content | minimal | <1ms |

## Common Response Fields

Every tool response includes:

- **`budget_status`** — current token budget snapshot with `total_budget`, `estimated_used`, `estimated_remaining`, and `pressure_level`
- **`coherence_alerts`** — (when present) notifications about changed underlying documents

## Typical Workflow

```
survey → read → extract → related
  ↓                          ↓
budget ← compress ← evicted
```

1. **Survey** to orient — find relevant areas of the corpus
2. **Read** specific sections identified by the survey
3. **Extract** claims for surgical precision
4. **Related** to follow reasoning chains
5. **Budget** to check pressure and get eviction recommendations
6. **Compress** sections before evicting them
7. **Evicted** to tell iris what was dropped (improves tracking accuracy)
