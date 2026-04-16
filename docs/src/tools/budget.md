<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#gauge"/></svg>
</div>

# iris_budget

Get the current context budget status: total budget, estimated usage, pressure level, and eviction recommendations.

## Parameters

None.

## Response

```json
{
  "total_budget": 100000,
  "estimated_used": 78500,
  "estimated_remaining": 21500,
  "pressure_level": "high",
  "eviction_candidates": [
    {
      "content_id": "docs/setup.md#prerequisites",
      "reason": "low relevance, delivered 8 turns ago",
      "tokens_recoverable": 450
    }
  ],
  "prefetch_metrics": {
    "sequential_hits": 3,
    "sequential_misses": 1,
    "topical_hits": 2,
    "topical_misses": 4,
    "structural_hits": 1,
    "structural_misses": 2
  }
}
```

### Response Fields

| Field | Description |
|---|---|
| `total_budget` | Total context window budget in tokens |
| `estimated_used` | Estimated tokens currently in the agent's window |
| `estimated_remaining` | Estimated tokens available |
| `pressure_level` | `"normal"`, `"elevated"`, or `"high"` |
| `eviction_candidates` | Ranked list of content safe to drop (populated under pressure) |
| `prefetch_metrics` | Hit/miss counts per prefetch strategy |

### Pressure Levels

| Level | Threshold | Behavior |
|---|---|---|
| `normal` | < 80% used | Standard operation |
| `elevated` | 80-90% used | Eviction candidates suggested |
| `high` | > 90% used | Responses auto-compressed, strong eviction recommendations |

## Behavior

- Call this periodically to monitor budget health
- Eviction candidates are ranked by recency, relevance decay, and dependency analysis
- Prefetch metrics help assess whether prefetching is effective for the current session
