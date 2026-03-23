---
name: isolated_test_repo
description: NEVER test iris changes against the live repo — always use an isolated example repository to avoid conflicts with the running iris MCP server
type: feedback
---

When testing iris changes (integration tests, manual smoke tests, stress tests), NEVER point a test iris instance at the same codebase the live iris MCP server is already running on.

**Why:** The live iris MCP server is indexing this repo in real-time. Running a second instance on the same codebase causes conflicts: shared SQLite databases, shared HNSW indexes, shared session state. This leads to corrupted results, stale data, and the agent drawing false conclusions from conflicting state. The user has been burned by this repeatedly and is extremely frustrated.

**How to apply:**
- For integration/smoke tests of iris itself, create or use a small isolated example repository (e.g., a temp dir with a few sample .rs/.md files)
- Never run `iris index --corpus ./iris-core/src` or similar on the live working directory during development
- When writing automated tests, use `tempdir()` or a dedicated fixture directory
- If you need to verify MCP tool behavior, use `cargo test` with the test harness — don't invoke the live MCP server against the same corpus
- This applies to stress tests too — spin up a separate instance pointed at an isolated corpus
