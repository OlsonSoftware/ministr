# Tool reference

<!-- @generated tool-index — do not edit; regenerate: cargo run -p ministr-mcp --example gen_tool_docs -->

Generated from the MCP tool manifest — the same schemas agents receive.

| Tool | Description |
|---|---|
| [`ministr_bridge`](bridge.md) | Cross-language bridge links (Tauri commands, NAPI exports, PyO3 functions, FFI, HTTP routes, etc.). |
| [`ministr_clone`](clone.md) | Clone a git repository and index its content. |
| [`ministr_compress`](compress.md) | Extractive TF-IDF summaries (roughly 60-80% shorter) for sections you want to keep referenceable without their full text. |
| [`ministr_dead`](dead.md) | Find symbols with zero references — candidates for safe deletion. |
| [`ministr_definition`](definition.md) | Full source of a code symbol by ID. |
| [`ministr_diagnostics`](diagnostics.md) | Run the project's own toolchain(s) (cargo/tsc/eslint/ruff/go vet/…, plus any SARIF-emitting tool) and return bounded STRUCTURED diagnostics (file, range, severity, code, message), errors first, each cross-linked to the enclosing symbol. |
| [`ministr_dropped`](dropped.md) | Call immediately after dropping content you previously received. |
| [`ministr_extract`](extract.md) | Atomic claims from a section, optionally query-filtered. |
| [`ministr_fetch`](fetch.md) | Fetch a URL from the web and index its content. |
| [`ministr_impact`](impact.md) | Transitive blast radius of changing a symbol. |
| [`ministr_projects`](projects.md) | List the current project plus any linked projects you can query in this session. |
| [`ministr_read`](read.md) | Full content of a section by ID. |
| [`ministr_references`](references.md) | All callers, implementors, and importers of a code symbol. |
| [`ministr_refresh`](refresh.md) | Check cached web and git sources for staleness and re-fetch changed content. |
| [`ministr_related`](related.md) | Follow relationship edges (references, contradicts, depends_on, updates) from a claim. |
| [`ministr_run`](run.md) | Run a shell command (recorded + captured). |
| [`ministr_run_kill`](run-kill.md) | Cancel a running run; kills the whole process group. |
| [`ministr_run_logs`](run-logs.md) | Page a run's captured log (delta: only what you haven't seen) or filter it with query=substring. |
| [`ministr_run_status`](run-status.md) | Poll a run's status (running/exited/killed/timed_out, exit code, duration, bytes). |
| [`ministr_solid`](solid.md) | Detect possible SOLID-principle violations across the codebase deterministically. |
| [`ministr_survey`](survey.md) | Search the indexed corpus by natural-language query. |
| [`ministr_symbols`](symbols.md) | Find code symbols (functions, structs, traits, etc.) by name, kind, module, or visibility. |
| [`ministr_task`](task.md) | Poll a background task status. |
| [`ministr_toc`](toc.md) | Structural overview (table of contents) of the indexed corpus. |
| [`ministr_usage`](usage.md) | Internal ministr accounting (a rough token estimate of what it has delivered so far). |
