# Prompt: make ministr-mcp tool calls cascade-safe

Paste the section below into a **fresh `claude` session started in `~/Code/ministr`**.
It mirrors the server-side fix already shipped in
`~/Code/think-and-ship` (commits `9f43d01`, `b32480d`, `7968802` on its
`main`).

---

Make the ministr MCP server's tool calls incapable of triggering Claude
Code's parallel-batch cascade-cancel bug (anthropics/claude-code#22264),
the same way it was done for think-and-ship.

**The bug.** When one tool call in a parallel batch errors, Claude Code
cancels every sibling call. A tool "errors" two ways: a JSON-RPC `-32602`
(strict serde rejecting a wrong-typed/missing arg) or a `CallToolResult`
with `is_error: true`. Eliminate both classes by construction.

**Two-part invariant to enforce across `ministr-mcp`:**

1. **Args deserialize infallibly.** Every tool-arg struct field gets
   `#[serde(default)]` plus, where the type isn't already forgiving, a
   custom `deserialize_with` that returns a fallback instead of erroring:
   - int | numeric-string → `u32` (fallback 0)
   - bool | "true"/"1"/"yes" | number → `bool`
   - unknown enum variant → a default or an `Unknown` variant
     (`#[serde(other)]` + `#[default]`), NOT a hard reject
   - single string → `Vec<String>`
   No bare required fields, no strict enums. Move required-ness *into the
   handler* as a soft error. Put all coercion helpers in ONE module
   (SRP) — check whether ministr already has a `coerce`-like util before
   creating one.

2. **Handlers never return `is_error: true`.** Find ministr-mcp's result
   helper (the equivalent of think-and-ship's
   `infra::tool_result::soft_error` — search for
   `CallToolResult::structured_error`, `is_error`, or an `err`-builder).
   Make logical failures return `is_error: Some(false)` with a loud
   structured `{ ok: false, error_kind, message }` envelope plus a
   `"⚠ kind: message"` text line. The tool *succeeds at reporting the bad
   request*; no errored sibling, nothing cascades.

**Reference implementation to mirror:**
- `~/Code/think-and-ship/crates/think-and-ship/src/infra/coerce.rs`
- `~/Code/think-and-ship/crates/think-and-ship/src/infra/tool_result.rs`
- how the three families' services delegate their error helpers to
  `soft_error` (think-and-ship `main`, commit `7968802`).

**Process discipline (important):**
- Start by reading `ministr-mcp/src/server/{mod,types,builders}.rs` to
  find where tool-arg structs and the result/error helper live. Don't
  assume shapes — read each file immediately before editing it; never
  fabricate a struct field, symbol, or path.
- Work in small slices: coerce helpers → result helper → args per tool
  group. After EACH slice: `cargo build -p ministr-mcp`, then
  `cargo clippy -p ministr-mcp --all-targets -- -D warnings`, then
  `cargo test -p ministr-mcp`. Commit only after all three are green —
  never commit before verification completes.
- Stay on `main`. Don't push.
- Run one tool call at a time; do not batch a discovery call with the
  call that consumes its output (that batching is what triggers the very
  cascade you're fixing).

---

## Context: the harness-level safety net is already on

`CLAUDE_CODE_MAX_TOOL_USE_CONCURRENCY=1` is set in `~/.claude/settings.json`
(user scope), which prevents parallel batches everywhere — so ministr is
already protected harness-side. This server-side fix makes it robust even
if concurrency is ever raised again.
