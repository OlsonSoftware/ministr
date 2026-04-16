<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#package"/></svg>
</div>

# iris_compress

Generate compressed summaries for sections the agent wants to evict from context. Returns short extractive summaries preserving the gist, with original and compressed token counts.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `content_ids` | list of strings | yes | Section IDs to generate compressed summaries for |

## Response

```json
{
  "summaries": [
    {
      "original_id": "docs/auth.md#jwt-validation",
      "summary": "JWT validation uses RS256 signing with 1-hour expiry. Tokens are verified against the public key from the JWKS endpoint.",
      "original_tokens": 847,
      "compressed_tokens": 95
    }
  ],
  "budget_status": { ... }
}
```

### Response Fields

| Field | Description |
|---|---|
| `summaries` | List of compressed summaries |
| `summaries[].original_id` | The section ID that was compressed |
| `summaries[].summary` | Short extractive summary preserving key information |
| `summaries[].original_tokens` | Token count of the original full section |
| `summaries[].compressed_tokens` | Token count of the compressed summary |

## Behavior

- Compression is extractive: selects the most information-dense sentences from the section
- Typically achieves 60-80% token reduction
- Unknown content IDs are silently skipped (not included in the response)
- Use this before evicting content — replace the full section with the compressed summary in the agent's context to preserve the gist while freeing budget
