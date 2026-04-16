<div class="iris-tool-head">
<svg class="icon icon-xl iris-tool-icon"><use href="../assets/icons.svg#arrow-up-right"/></svg>
</div>

# iris_fetch

Fetch a web URL and add its content to the corpus. Supports single pages, sitemaps, and llms.txt feeds.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `url` | string | yes | HTTP(S) URL to fetch |
| `recursive` | boolean | no | Follow sitemap.xml or llms.txt if the URL points to a site root |
| `max_pages` | integer | no | Maximum pages to fetch when `recursive` is true (default: 100) |

## Response

```json
{
  "ingested": [
    {
      "url": "https://example.com/docs/auth",
      "document_id": "https://example.com/docs/auth",
      "section_count": 5,
      "token_count": 1240
    }
  ],
  "skipped": [],
  "errors": [],
  "budget_status": { ... }
}
```

## Behavior

- HTML is converted to Markdown via readability extraction before indexing
- PDF URLs are text-extracted via `pdf-extract`
- `sitemap.xml` and `llms.txt` are auto-detected for recursive fetching
- Fetched content is cached locally with ETag / Last-Modified headers
- Use `iris_refresh` to re-fetch changed sources
- Subject to rate limiting (default: 4 concurrent requests)
