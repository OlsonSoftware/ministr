# iris

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

A context cache controller for LLM agents — an MCP server that manages context windows like a CPU cache controller, with session tracking, predictive prefetching, budget management, and coherence.

## Features

- **Semantic search** — survey documents and code with embedding-based retrieval at multiple resolutions (document, section, claim)
- **Code symbol index** — find structs, functions, traits, and enums by name, kind, module, or visibility
- **Session tracking** — shadow the agent's context window to deduplicate delivered content and track what's been evicted
- **Predictive prefetch** — speculatively pre-embed content the agent is likely to request next
- **Budget management** — monitor token usage, recommend evictions, and provide compressed summaries under pressure
- **Live coherence** — watch the filesystem for changes and alert the agent when delivered content goes stale
- **Multi-source corpora** — index local directories, web URLs, and git repositories

## Installation

### From source

```sh
cargo install --path iris-cli
```

### Pre-built binaries

Download from [GitHub Releases](https://github.com/alrik/iris-rs/releases).

## Quick start

### 1. Pre-warm the index (optional)

```sh
iris index --corpus ./docs
```

### 2. Configure your MCP client

**Claude Code** — create `.mcp.json` at your project root:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "./docs"]
    }
  }
}
```

Or use the CLI shorthand:

```sh
claude mcp add iris -- iris --corpus ./docs
```

**Cursor** — create `.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "./docs"]
    }
  }
}
```

### 3. Start using iris tools

Once connected, the agent has access to these MCP tools:

| Tool | Purpose |
|------|---------|
| `iris_survey` | Semantic search across docs and code |
| `iris_read` | Read a section by ID (with deduplication) |
| `iris_extract` | Get atomic claims from a section |
| `iris_related` | Follow dependency chains between claims |
| `iris_symbols` | Search the code symbol index |
| `iris_definition` | Get full source of a symbol |
| `iris_references` | Find callers, implementors, importers |
| `iris_budget` | Check context budget and get eviction advice |
| `iris_compress` | Generate compressed summaries for eviction |
| `iris_evicted` | Signal that content has been dropped |
| `iris_fetch` | Fetch web content and add to corpus |
| `iris_clone` | Clone a git repo and index it |
| `iris_refresh` | Re-fetch changed web sources |
| `iris_toc` | Structural overview of the corpus |

## CLI usage

```
iris [OPTIONS] [COMMAND]

Commands:
  serve   Start the MCP server over stdio (default)
  index   Run ingestion synchronously and exit

Options:
  -c, --corpus <PATH>   Corpus sources (repeatable): local paths, https:// URLs, github:// URLs
  -C, --config <PATH>   Path to config file (default: ~/.iris/config.toml)
  -h, --help            Print help
  -V, --version         Print version
```

## Architecture

```
iris-core/     — domain logic, no transport dependencies
iris-mcp/      — MCP server adapter (rmcp)
iris-cli/      — binary entry point
```

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
