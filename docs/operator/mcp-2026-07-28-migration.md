
# MCP 2026-07-28 Migration Plan

Audit of ministr's MCP surface against the 2026-07-28 specification
release candidate (stateless protocol core). 

## Spec changes that affect ministr

| Change | Impact | ministr dependency |
|--------|--------|--------------------|
| **No `initialize` handshake** | HIGH | `MinistrServer::initialize` captures `tenant_id_hint` + `client_name_hint` + negotiates extensions |
| **No `Mcp-Session-Id` header** | MEDIUM | `fork_for_new_session` per-connection isolation relies on rmcp's session management |
| **No resumable streams** | LOW | ministr doesn't use resumable streams |
| **Extensions framework** replaces capability negotiation | MEDIUM | `NegotiatedExtensions::negotiate` called from `initialize` |
| **Enhanced OAuth** | LOW-MEDIUM | Our OAuth 2.1 + DCR + PKCE may need alignment |

## Dependency map

### 1. Tenant capture

**Current path:** `initialize` → `context.extensions` → `Parts` →
`Tenant` → `tenant_id_hint` (F-Test-3b-fix-1).

**13 downstream call sites** use `self.current_tenant_subject()` which
reads `tenant_id_hint`. All tool handlers depend on this for:
- Default corpus selection
- Cross-corpus survey tenant filtering
- Session stamping (`ensure_session_mut`)
- Audit emission
- Per-tenant visibility filtering

**Migration path:** In the stateless world, every request must carry
the bearer token. rmcp's stateless mode will need to expose the
`Parts` (or equivalent) on every `RequestContext`, not just
`initialize`. If rmcp provides per-request `Parts`, we read the
`Tenant` from every tool call's context rather than caching it once.
If rmcp doesn't, we need a middleware that extracts the tenant from
the `Authorization` header and injects it into a server-side field
that tool handlers read.

### 2. Client name capture

**Current:** `initialize` → `request.client_info.name` →
`client_name_hint`.

**Migration:** Client info may be sent via a new mechanism in the
extensions framework, or may be dropped entirely. The hint is
cosmetic (tray tooltip + session dashboard). Acceptable to lose
temporarily.

### 3. Extension negotiation

**Current:** `NegotiatedExtensions::negotiate` runs in `initialize`
and stores in `Arc<Mutex<NegotiatedExtensions>>`. The struct has
fields: `usage_protocol`, `coherence`, `compression`.

**Read sites:** `negotiated_extensions()` accessor in `builders.rs`
— but currently **no tool handler reads it at runtime** (the
accessor exists for future use). The negotiation result is logged
but never consumed.

**Migration:** Extensions framework in the new spec replaces
`initialize`-based negotiation. Since no tool handler reads the
negotiated state today, the migration is: (a) register ministr's
extensions via the new framework, (b) remove the `initialize`-based
negotiation code. **Low urgency** — can be deferred to a later release.

### 4. Fork-per-connection

**Current:** `server_factory` at `commands.rs:499` calls
`server.fork_for_new_session()` which assigns a fresh `uuid_v4`
`active_session_id`. rmcp's `StreamableHttpService` calls the factory
per-connection.

**Migration:** In the stateless world, there may not be a
"connection" concept at all — each request is independent. The
`fork_for_new_session` pattern may become fork-per-request or
unnecessary entirely if sessions are identified by bearer token +
explicit session parameter.

### 5. Session binding

**Current:** `active_session_id` is assigned per-fork (uuid_v4).
Session entry is created lazily on first tool call via
`ensure_session_mut`. Sessions are tenant-scoped via the
`tenant_id` field stamped from `current_tenant_subject()`.

**Migration options:**
- **A. Client-provided session ID in tool params.** Add an optional
  `session_id` param to every tool. The client carries it across
  requests. Simple but pollutes every tool schema.
- **B. Server-inferred from bearer token.** One session per bearer.
  Matches the "stateless" ethos but loses multi-session-per-user.
- **C. Session ID in a custom header.** Clients opt into session
  continuity by sending `X-Ministr-Session: <id>`. Invisible to
  tools. Cleaner than A.
- **Recommended: C** — custom header, fallback to bearer-derived.

### 6. OAuth hardening

**Current:** OAuth 2.1 + DCR + PKCE, well-aligned with prior spec.

**Migration:** Audit the RC's enhanced OAuth requirements. Likely
minor — our existing implementation is already strict (RS256,
PKCE S256, short-lived tokens). May need to add specific OAuth
metadata fields the new spec requires.

## Recommended execution order

1. Audit (this document) — done.
2. Stateless tenant resolution. Highest impact; unblocks
   all tool handlers.
3. Session continuity. Depends on tenant resolution.
4. rmcp version bump. The integration test.
5. Extensions framework. Can follow the bump.
6. OAuth hardening. Lowest urgency; audit-only.

## Blocking question

**Does rmcp expose `RequestContext.extensions` (with `Parts`) on
every tool call in the stateless mode?** This is the single
load-bearing question for the tenant-resolution work. If yes, the migration is a
~50-line refactor (read tenant per-request instead of per-init).
If no, we need an rmcp upstream contribution or a middleware
workaround.

**Action:** Check rmcp's `main` branch or issue tracker for
2026-07-28 RC support before starting the migration.

## RC findings (updated 2026-05-24)

The MCP 2026-07-28 RC dropped **2026-05-21**. Key additional changes
not in the original audit above:

- **Sampling deprecated** (SEP-2577): `sampling/createMessage` is
  deprecated. ministr's `SamplingCompressor` uses this. Extractive
  compression is unaffected. 12-month removal window per lifecycle
  policy.
- **Roots deprecated** (SEP-2577): ministr doesn't use Roots — no
  impact.
- **Logging deprecated** (SEP-2577): ministr doesn't use MCP-level
  logging — no impact.
- **Header-based routing** (SEP-2243): `Mcp-Method` and `Mcp-Name`
  headers enable gateway routing without body inspection. Plus
  `MCP-Protocol-Version` header. Low-priority; verify rmcp handles.
- **Tasks extension** (SEP-2663): long-running operations with
  `tasks/get`/`tasks/update`/`tasks/cancel`. May be relevant for
  indexing jobs. Evaluate during F7.4.
- **JSON Schema 2020-12** (SEP-2106): input schemas now support
  `oneOf`/`anyOf`/`allOf`/`$ref`. Verify our tool schemas are
  compatible.
- **Error code change** (SEP-2164): missing resource error changes
  from `-32002` to `-32602`. Check our error returns.

**rmcp status**: v1.6.0 on crates.io already has stateless mode
(per GitHub issue #841, May 9 2026). ministr is on 0.14. The
0.x→1.x upgrade (F7.6) is the gating prerequisite for all other
F7 chunks.
