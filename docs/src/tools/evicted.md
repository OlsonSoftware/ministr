<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#x"/></svg>
</div>

# iris_evicted

Signal that content IDs have been evicted from the agent's context window. Updates session tracking for accurate budget and deduplication.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `content_ids` | list of strings | yes | Content IDs that have been evicted from the agent's context window |

## Response

```json
{
  "evicted": ["docs/setup.md#prerequisites", "docs/intro.md#overview"],
  "not_found": ["unknown-id"],
  "budget_status": { ... }
}
```

### Response Fields

| Field | Description |
|---|---|
| `evicted` | Content IDs successfully removed from the session shadow |
| `not_found` | Content IDs that were not in the session shadow |

## Behavior

- Removes the specified content from the session shadow
- Reduces the estimated token usage in the budget tracker
- Future `iris_survey` calls will no longer filter out evicted content
- Improves the accuracy of window estimation — without explicit signals, iris relies on heuristic estimation which may drift over long sessions
- This is the agent's way of correcting iris's window estimate, analogous to an explicit cache invalidation
