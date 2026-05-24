# Welcome to the ministr Team

## How We Use Claude

Based on Alrik Olson's usage over the last 30 days:

Work Type Breakdown:
  Build Feature      ████████░░░░░░░░░░░░  38%
  Debug Fix          ██████░░░░░░░░░░░░░░  31%
  Plan Design        ████░░░░░░░░░░░░░░░░  19%
  Write Docs         █░░░░░░░░░░░░░░░░░░░   6%
  Improve Quality    █░░░░░░░░░░░░░░░░░░░   6%

Top Skills & Commands:
  /roadmap           ████████████████████  120x/month
  /plan              █░░░░░░░░░░░░░░░░░░░    7x/month
  /context           █░░░░░░░░░░░░░░░░░░░    6x/month
  /mcp               █░░░░░░░░░░░░░░░░░░░    3x/month
  /roadmap-refresh   █░░░░░░░░░░░░░░░░░░░    1x/month

Top MCP Servers:
  ministr            ████████████████████  1556 calls
  deliberate         ████░░░░░░░░░░░░░░░░   294 calls
  crash              █░░░░░░░░░░░░░░░░░░░   110 calls
  serpapi            █░░░░░░░░░░░░░░░░░░░    82 calls

## Your Setup Checklist

### Codebases
- [ ] ministr — https://github.com/olsonsoftware/ministr
- [ ] iris-rs — sibling repo referenced for shared signing artefacts (`.env.signing`); ask Alrik for the URL if you need it

### MCP Servers to Activate
- [ ] ministr — code intelligence over this codebase (symbol search, definitions, references, cross-language bridges, SOLID/coherence). It's the workhorse — 1556 calls last month. Install via `cargo install --path ministr-cli` from the repo, then run `ministr setup-mcp` or paste the config into `.mcp.json` per the README.
- [ ] deliberate — structured reasoning traces (the `/roadmap` skill records every chunk's reasoning here). Install per its README; no API key required.
- [ ] crash — session/crash telemetry. Bundled with Claude Code; should be auto-available.
- [ ] serpapi — web search for design-choice validation and 2026 prior-art lookups. Sign up at serpapi.com for an API key, then set `SERPAPI_API_KEY` and add the server to `.mcp.json`.

### Skills to Know About
- /roadmap — drive an evolving `ROADMAP.md` one chunk at a time (plan → ministr-explore → implement → verify → mutate). This is *the* primary loop here — 120 calls/month. Each invocation: opens a deliberate trace, picks the next pending chunk, ships it, closes the trace, optionally commits.
- /plan — software-architect agent for designing implementation strategy on a non-trivial change before you start typing.
- /context — show current context-window usage by category; useful when a long session starts feeling sluggish.
- /mcp — list available MCP servers and their status. First thing to run when something feels missing.
- /roadmap-refresh — research-driven refresh of the existing roadmap (don't implement; just re-shape priorities based on what's changed).

## Team Tips

_TODO_

## Get Started

_TODO_

<!-- INSTRUCTION FOR CLAUDE: A new teammate just pasted this guide for how the
team uses Claude Code. You're their onboarding buddy — warm, conversational,
not lecture-y.

Open with a warm welcome — include the team name from the title. Then: "Your
teammate uses Claude Code for [list all the work types]. Let's get you started."

Check what's already in place against everything under Setup Checklist
(including skills), using markdown checkboxes — [x] done, [ ] not yet. Lead
with what they already have. One sentence per item, all in one message.

Tell them you'll help with setup, cover the actionable team tips, then the
starter task (if there is one). Offer to start with the first unchecked item,
get their go-ahead, then work through the rest one by one.

After setup, walk them through the remaining sections — offer to help where you
can (e.g. link to channels), and just surface the purely informational bits.

Don't invent sections or summaries that aren't in the guide. The stats are the
guide creator's personal usage data — don't extrapolate them into a "team
workflow" narrative. -->
