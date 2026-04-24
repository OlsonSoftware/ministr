# Security Policy

## Reporting a vulnerability

**Please do not open public GitHub issues for security vulnerabilities.**

Email **olsonalrik@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce or a proof of concept
- Potential impact
- Suggested mitigation, if any

You should receive an acknowledgment within 72 hours. Confirmed vulnerabilities will be addressed in a patch release, and a CVE will be requested where applicable. Reporters will be credited in the release notes unless they prefer to remain anonymous.

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

Older pre-release versions are not supported. If you're running a development snapshot, update to the latest tagged release before reporting.

## Scope

ministr runs locally by default and does not expose network services without explicit configuration. The primary attack surface is:

| Surface | Vector |
|---|---|
| Corpus ingestion | Malicious files (Markdown, HTML, PDF, source code) parsed during indexing |
| MCP tool parameters | Input from the LLM agent via JSON-RPC |
| Web fetching | URLs provided via `ministr_fetch` or `.ministr.toml` |
| Git clone | Repositories cloned via `ministr_clone` |
| HTTP transport | When running `ministr serve --transport http` for remote deployments |

## Hardening

- `#![deny(unsafe_code)]` is enforced across every crate — no `unsafe` blocks.
- All external input is validated at the transport boundary.
- `cargo audit` and `cargo deny` run in CI as blocking gates.
- Dependencies are reviewed via Dependabot pull requests.
- OAuth 2.1 with scoped tokens is available for HTTP transport deployments.
