# MCP Registry Submissions

## awesome-mcp-servers (punkpeye/awesome-mcp-servers)

**PR description:**

Add iris — context cache controller for LLM agents.

**Entry to add under "Code & Development" or "Knowledge & Memory":**

```markdown
- [iris](https://github.com/alrik/iris-rs) — Context cache controller for LLM agents. Multi-resolution semantic search, code symbol navigation (12 languages), cross-language bridge tracing, token budget management, predictive prefetch, and live coherence. Written in Rust.
```

---

## awesome-mcp-servers (wong2/awesome-mcp-servers)

**Entry to add under "Code" section:**

```markdown
- [iris](https://github.com/alrik/iris-rs) <img src="https://img.shields.io/github/stars/alrik/iris-rs" alt="stars"> - Manages LLM agent context windows with semantic search, code intelligence (12 languages via tree-sitter), cross-language bridge tracing, budget management, and predictive prefetch. Rust.
```

---

## Smithery (smithery.ai)

**Manifest (`smithery.yaml`):**

```yaml
name: iris
description: Context cache controller for LLM agents — semantic search, code symbols, cross-language bridges, budget management
version: 0.1.0
author: alrik
repository: https://github.com/alrik/iris-rs
license: MIT OR Apache-2.0
language: rust
install:
  cargo: iris-cli
  binary: iris
transport: stdio
command: iris serve --transport stdio
tools:
  - iris_survey
  - iris_read
  - iris_extract
  - iris_symbols
  - iris_definition
  - iris_references
  - iris_toc
  - iris_related
  - iris_bridge
  - iris_budget
  - iris_compress
  - iris_evicted
  - iris_fetch
  - iris_clone
  - iris_refresh
tags:
  - code
  - search
  - context
  - codebase
  - rust
```

---

## mcpservers.org

**Submission fields:**

- **Name:** iris
- **Description:** MCP server that manages LLM agent context windows like a CPU cache controller. Semantic search, code intelligence for 12 languages, cross-language bridge tracing, token budget management, predictive prefetch, and live file coherence.
- **GitHub:** https://github.com/alrik/iris-rs
- **Category:** Code & Development
- **Language:** Rust
- **Transport:** stdio, streamable HTTP
- **Install:** `cargo install --git https://github.com/alrik/iris-rs iris-cli`
