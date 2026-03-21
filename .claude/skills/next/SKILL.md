# /next

Implement the next meaningful chunk from the project spec with mandatory planning phase.

## STRICT WORKFLOW — PHASES ARE GATED, DO NOT SKIP

Read `.magistr/project.json` to get the spec file, quality gate commands, and testing config.
Read the spec file. Identify the next coherent group of unchecked items.

**Plan entry status:** Keep plan entry statuses up to date throughout the iteration. Mark entries as `in_progress` when you start them and `completed` when done. Only one entry should be `in_progress` at a time. All entries from a phase should be `completed` before starting the next phase.

### PHASE 0 — RESEARCH (mandatory, before planning)

> **Start by calling `magistr_set_phase(phase: "research")`** to update the TUI progress indicator.

Before planning anything, research the items you're about to implement. This is NON-NEGOTIABLE.

1. **Read the spec file** — identify the next coherent group of unchecked items (just read, don't plan yet)

2. **Web search** — use `web_search` to research:
   - Best practices and idiomatic patterns for what you're about to build
   - Recent API changes, breaking changes, or deprecations in relevant libraries/frameworks
   - Known pitfalls, edge cases, or common mistakes for this type of work
   - Performance considerations or security implications
   - Formulate 2-4 targeted search queries based on the specific items

3. **Deep dive** (when applicable) — use `firecrawl` to scrape:
   - Official documentation pages for libraries/APIs you'll use
   - Relevant GitHub issues, discussions, or changelogs
   - Reference implementations or examples from authoritative sources
   - Only scrape URLs surfaced by web search or already known — don't guess URLs

4. **Capture findings** — mentally note key insights that will inform the plan:
   - API signatures, type constraints, or interface contracts discovered
   - Patterns to follow (or anti-patterns to avoid)
   - Version-specific behavior or compatibility notes
   - Any "gotchas" that should shape the implementation approach

**Do NOT skip this phase.** Even for seemingly simple items, a 2-minute research pass prevents hours of rework from outdated assumptions or missed best practices.

---

### PHASE 1 — PLAN (read-only, NO edits to source files)

> **Start by calling `magistr_set_phase(phase: "plan")`** to update the TUI progress indicator.

You MUST complete all of these before moving to Phase 2:

1. **Orient** — quick state check:
   - Run the quality gate check command — build status
   - `git log --oneline -3` — recent commits
   - Read the spec file to see what's checked off

2. **Explore the codebase** — use magistr-scope tools (NOT Read/Glob/Grep):
   - `scope_search` to find relevant files and understand structure
   - `scope_read(mode: "stubs")` for structural overview of key files
   - `scope_imports(path: "file", transitive: true)` before modifying shared code
   - `scope_grep` for targeted pattern searches
   - Do NOT use Read for exploration — it is ONLY for immediately before Edit

3. **Identify scope** — find the next coherent group of unchecked items. Group by logical affinity:
   - Related types, functions, or modules that belong together
   - A complete subsystem or feature slice
   - Aim for 3-6 new source files per iteration
   - **Too small**: a single function or trivial change
   - **Too large**: an entire phase or more than ~8 new files

4. **Consult references** — if `reference_repos` is configured in project.json, read the reference source for every type/function you plan to implement.

5. **Write the plan** to `.magistr/current-plan.md` containing:
   - What you'll implement
   - **Research findings** — key insights from Phase 0 that shape the approach
   - Which reference source to consult (if applicable)
   - File layout (which files to create/modify)
   - Key design decisions
   - Step-by-step implementation order
   - Test strategy

6. **Write the task name** to `.magistr/current-task.md`

DO NOT edit any source files. DO NOT run build commands besides reads.
DO NOT proceed to Phase 2 until `.magistr/current-plan.md` exists and is complete.

### PHASE 2 — EXECUTE

> **Start by calling `magistr_set_phase(phase: "execute")`** to update the TUI progress indicator.

Now implement the plan from `.magistr/current-plan.md`:

1. Follow the plan step by step — do not deviate without good reason
2. Use `scope_read` to understand code before modifying. Use Read only immediately before Edit.
3. For each unit of work, use TDD:
   - **RED**: Write failing tests
   - **GREEN**: Write minimal code to pass
   - **REFACTOR**: Clean up with tests green
4. Run the check command periodically to catch issues early
5. Wire new modules into the project's entry points

Follow all project conventions from `.claude/rules/conventions.md`.

### PHASE 3 — VERIFY & CLOSE (ALL steps are NON-NEGOTIABLE)

> **Start by calling `magistr_set_phase(phase: "verify")`** to update the TUI progress indicator.

**Step 1: Quality gates** — Run the full quality gate (read commands from project.json):
```
[check_cmd] && [test_cmd] && [lint_cmd]
```
All must pass. If any fail, fix before proceeding. Do NOT skip to step 2 with failing gates.

**Step 2: Update `ROADMAP.md`** — MANDATORY, NOT OPTIONAL:
- Check off (`- [x]`) every item that was just implemented
- Add and check off new items discovered during implementation
- Update design notes if implementation diverged from the plan
- If no existing items match what you did, ADD new items and check them off
- The roadmap MUST reflect reality after every iteration — no invisible work

**Step 3: Commit** — MANDATORY, NOT OPTIONAL:
- Stage ALL changed files including `ROADMAP.md`
- Use conventional commit format: `feat:`, `fix:`, `refactor:`, `test:`
- The commit MUST include the roadmap update — code and spec travel together
- Do NOT leave uncommitted work. Do NOT defer the commit to later.
- Do NOT end the iteration without a commit.

**Step 4: Clean up** — delete `.magistr/current-plan.md` and `.magistr/current-task.md`

**Step 5: Mark done** — call `magistr_set_phase(phase: "done")` to complete the progress indicator.

**Step 6: Log** — Append summary to `.magistr/iteration-log.md`:
```
## Iteration N — <date>
- [what was implemented]
- Tests: X new (Y total)
- Quality gate: PASS
```

**Step 6: Report** to the user:
```
## Done
- [what was implemented — group by category]
- [total new tests]

## Quality Gate
- check: PASS/FAIL
- test: PASS/FAIL  (N total)
- lint: PASS/FAIL

## Commit
- [commit hash and message]

## Next `/next` will...
- [what the next iteration will tackle]
```

## Non-Negotiable Rules

These are not guidelines. They are hard requirements. Violating them means the iteration FAILED.

1. **ALWAYS research before planning.** Use `web_search` and/or `firecrawl` to inform every iteration. No blind coding.
2. **ALWAYS update `ROADMAP.md`.** Every iteration. No exceptions. No "I'll do it next time."
3. **ALWAYS commit at the end.** Uncommitted work is unfinished work. The iteration is not done until the commit lands.
4. **ALWAYS pass quality gates before committing.** Broken code does not get committed.
5. **ALWAYS plan before executing.** The PreToolUse hook will BLOCK source file edits until `.magistr/current-plan.md` exists.
6. **ALWAYS use TDD.** Every significant unit of work gets tests. Write the test first.
7. **No agents or teams.** Solo work only.
8. **Meaningful chunks.** Group related items together. Don't do one tiny thing per iteration.
9. **Don't repeat work.** Check what exists before building.
10. **ONE iteration only.** After Phase 3, STOP. Do NOT invoke `/next` or `/next-loop` yourself. The orchestrator (magistr TUI or run-autonomous.sh) manages iteration chaining with fresh context windows. Self-invoking `/next` corrupts iteration tracking and causes context degradation.

**Hooks enforce these rules:**
- Stop hook blocks you from finishing with uncommitted changes
- Stop hook blocks you if the spec file wasn't updated
- PreToolUse hook blocks git commit if quality gates fail
- You cannot skip these steps — they are deterministic enforcement
