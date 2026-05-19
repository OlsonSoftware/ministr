---
description: Trace an HTTP route end-to-end — route definition → handler function → downstream calls. Uses ministr_route.
---

Trace the route(s) matching: $ARGUMENTS

Use the `ministr_route` MCP tool. The input can be:
- A path pattern (e.g. `/api/users/:id`)
- A handler symbol name
- A method + path (e.g. `POST /login`)
- Empty — return all detected routes in the corpus

The response shows `{ method, path, handler_symbol, downstream: [...], external_calls: [...] }` for each route. Use this to understand request-path code before touching it; the downstream walk surfaces DB queries, RPC calls, and side effects that a route definition alone doesn't reveal.
