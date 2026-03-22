# iris_survey

Search the indexed corpus for sections relevant to a natural language query. Returns ranked summaries with relevance scores. Already-delivered content is filtered out.

## Parameters

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | string | yes | — | Natural language query to search the corpus |
| `top_k` | integer | no | 10 | Maximum number of results to return |

## Response

```json
{
  "results": [
    {
      "content_id": "docs/auth.md#jwt-validation",
      "title": "JWT Validation",
      "text": "Section summary text...",
      "relevance_score": 0.87,
      "resolution": "section",
      "token_count": 245
    }
  ],
  "deduplicated_count": 2,
  "budget_status": {
    "total_budget": 100000,
    "estimated_used": 245,
    "estimated_remaining": 99755,
    "pressure_level": "normal"
  }
}
```

### Response Fields

| Field | Description |
|---|---|
| `results` | Ranked list of matching sections with summaries |
| `results[].content_id` | Hierarchical section ID for use with `iris_read` |
| `results[].relevance_score` | Similarity score (0.0-1.0) |
| `results[].resolution` | Index level matched: `"summary"`, `"section"`, or `"claim"` |
| `results[].token_count` | Token count of the returned text |
| `deduplicated_count` | Number of results filtered out (already delivered this session) |

## Behavior

- Results that were already delivered in this session are filtered out automatically
- The response text is recorded in the session shadow for future deduplication
- Token counts are added to the running budget
- Matching occurs across all three resolution levels (summaries, sections, claims)
