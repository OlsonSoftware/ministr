# Tool Reference

iris exposes MCP tools that give the agent explicit control over context retrieval. The tools cover document search, code navigation, cross-language bridges, budget management, and multi-source corpus ingestion.

## Search & retrieval

| Tool | Purpose | Typical Token Cost |
|---|---|---|
| [`iris_survey`](survey.md) | Semantic search across docs and code | 100-300 |
| [`iris_read`](read.md) | Read a section in full | 200-2000 |
| [`iris_extract`](extract.md) | Pull specific claims | 50-500 |
| [`iris_related`](related.md) | Follow claim relationships | 50-300 |
| [`iris_toc`](toc.md) | Structural overview of the corpus | 100-500 |

## Code navigation

| Tool | Purpose | Typical Token Cost |
|---|---|---|
| [`iris_symbols`](symbols.md) | Find structs, functions, traits, enums | 50-300 |
| [`iris_definition`](definition.md) | Get full source of a symbol | 100-2000 |
| [`iris_references`](references.md) | Find callers, implementors, importers | 50-300 |
| [`iris_bridge`](bridge.md) | Query cross-language bindings | 50-200 |

## Budget management

| Tool | Purpose | Typical Token Cost |
|---|---|---|
| [`iris_budget`](budget.md) | Check budget status | minimal |
| [`iris_compress`](compress.md) | Get compressed summaries | 50-200 |
| [`iris_evicted`](evicted.md) | Signal evicted content | minimal |

## Multi-source corpora

| Tool | Purpose | Typical Token Cost | Latency |
|---|---|---|---|
| [`iris_fetch`](fetch.md) | Fetch web content into the corpus | 50-200 | seconds (network) |
| [`iris_clone`](clone.md) | Clone and index a git repo | 50-200 | seconds to minutes |
| [`iris_refresh`](refresh.md) | Re-fetch changed sources | 50-200 | seconds (network) |

## Common Response Fields

Every tool response includes:

- **`budget_status`** — current token budget snapshot with `tokens_used`, `tokens_remaining`, `pressure_level`, and `utilization`
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
