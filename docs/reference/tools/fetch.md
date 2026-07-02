# ministr_fetch

<!-- @generated tool-docs start — do not edit this block; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

> Fetch a URL from the web and index its content. Tries llms.txt first, falls back to direct page fetch.

## Parameters

| Parameter | Type | Required | Description |
|---|---|---|---|
| `depth` | integer | no | Crawl depth for following links (default: 0 = single page only) |
| `max_pages` | integer | no | Maximum number of pages to fetch when crawling (default: 50) |
| `path_filter` | string | no | Only fetch URLs whose path starts with this prefix (e.g. '/docs/') |
| `url` | string | no | URL to fetch content from (e.g. 'https://docs.example.com/') |

Annotations: open-world.

<small>This block is generated from the live tool schema — the same definition agents receive.</small>

<!-- @generated tool-docs end -->
