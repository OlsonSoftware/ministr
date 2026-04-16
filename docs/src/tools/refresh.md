# iris_refresh

Re-fetch changed web sources and pull updates for cloned git repositories.

## Parameters

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `source_id` | string | no | — | Specific source to refresh (URL or clone ID) |
| `all` | boolean | no | false | Refresh all external sources |

If neither is provided, only sources with expired cache headers are refreshed.

## Response

```json
{
  "refreshed": [
    {
      "source_id": "https://example.com/docs",
      "kind": "web",
      "change_detected": true,
      "sections_updated": 3
    },
    {
      "source_id": "github.com/serde-rs/serde",
      "kind": "git",
      "change_detected": false
    }
  ],
  "budget_status": { ... }
}
```

## Behavior

- Web sources are re-fetched only if ETag or Last-Modified indicates change
- Git clones run `git fetch` and compare HEAD against the indexed commit SHA
- Only changed sections are re-embedded; unchanged content retains existing vectors
- Call periodically to keep external content synced with upstream
- Local file sources are handled automatically by the coherence engine — you don't need `iris_refresh` for local files
