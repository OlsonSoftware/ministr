# Getting Started

## Installation

### From source (requires Rust 1.85+)

```sh
cargo install --path iris-cli
```

### Pre-built binaries

Download from the [GitHub Releases](https://github.com/alrik/iris-rs/releases) page.

## Quick Start

### 1. Point iris at a document corpus

```sh
# iris indexes a directory of Markdown, HTML, or PDF files
iris --corpus ./my-docs
```

This starts the MCP server over stdio, ready to accept tool calls from any MCP client.

### 2. Configure your MCP client

**Claude Code** — add to your MCP settings:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "/path/to/your/docs"]
    }
  }
}
```

**Cursor** — add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "/path/to/your/docs"]
    }
  }
}
```

### 3. Use the tools

Once connected, your agent has access to these tools:

1. **`iris_survey`** — search the corpus with a natural language query. Returns ranked summaries.
2. **`iris_read`** — read the full text of a specific section.
3. **`iris_extract`** — pull atomic claims from a section, optionally filtered by relevance.
4. **`iris_related`** — follow dependency chains between claims.
5. **`iris_budget`** — check context budget status and get eviction recommendations.
6. **`iris_compress`** — generate compressed summaries for sections to evict.
7. **`iris_evicted`** — tell iris what you dropped from context.

### Typical Workflow

The tools mirror how a human researcher navigates a knowledge base:

```
iris_survey("authentication")
  → 5 relevant sections found

iris_read("docs/auth.md#jwt-validation")
  → full section text, 847 tokens

iris_extract("token expiry", section_id="docs/auth.md#jwt-validation")
  → 3 claims about JWT expiry behavior

iris_related(claim_id="docs/auth.md#jwt-validation/claim-2")
  → related claims about refresh token handling
```

Each response includes a `budget_status` showing how much context budget has been consumed, so the agent can make informed decisions about what to read next.

## Configuration

iris uses a global config file at `~/.iris/config.toml`. See the [Configuration](configuration.md) chapter for full details.

A minimal config:

```toml
default_model = "all-MiniLM-L6-v2"
default_context_budget = 100000

[prefetch]
enabled = true
cache_size = 50
```
