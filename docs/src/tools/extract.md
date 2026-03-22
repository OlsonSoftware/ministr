# iris_extract

Extract atomic claims from a section, optionally filtered by relevance to a query. Claims are single factual statements that can stand alone.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `section_id` | string | yes | Section ID to extract claims from |
| `query` | string | no | Optional query to filter claims by relevance |

## Response

```json
{
  "claims": [
    {
      "id": "docs/auth.md#jwt-validation/claim-1",
      "text": "JWT tokens use RS256 signing algorithm.",
      "relevance_score": 0.92
    },
    {
      "id": "docs/auth.md#jwt-validation/claim-2",
      "text": "Token expiry is set to 1 hour by default.",
      "relevance_score": 0.85
    }
  ],
  "budget_status": { ... }
}
```

### Response Fields

| Field | Description |
|---|---|
| `claims` | List of extracted claims |
| `claims[].id` | Claim ID for use with `iris_related` |
| `claims[].text` | The atomic factual statement |
| `claims[].relevance_score` | Relevance to the query (1.0 if no query provided) |

## Behavior

- When `query` is provided, claims are ranked by embedding similarity to the query
- When `query` is omitted, all claims from the section are returned in document order
- Claims are the highest-resolution unit in iris — use them for surgical precision
- Each claim ID can be passed to `iris_related` to follow dependency chains
