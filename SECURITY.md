# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a vulnerability

If you discover a security vulnerability in iris, please report it responsibly.

**Do not open a public GitHub issue.**

Instead, email **olsonalrik@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

You should receive a response within 72 hours. Once the issue is confirmed, a fix will be developed privately and released as a patch version.

## Scope

iris runs locally and does not expose network services by default. The primary attack surface is:

- **Corpus ingestion** — malicious files (Markdown, HTML, PDF) parsed during indexing
- **MCP tool parameters** — input from the LLM agent via JSON-RPC
- **Web fetching** — URLs provided via `iris_fetch` / `.iris.toml` config
- **Git clone** — repositories cloned via `iris_clone`

All external input is validated. `#![deny(unsafe_code)]` is enforced across every crate.
