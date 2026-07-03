# ministr-cli

The `ministr` binary: a thin clap dispatch over library modules — `serve`
(stdio MCP proxy by default, Streamable HTTP with `--transport http`),
`init`, `index`, `status`, `search`, `export`/`import`, `hooks`, and `setup`.
Every command is documented in the [CLI reference](../docs/reference/cli.md).

The crate also exposes `commands::cmd_serve_http` as a library entry point so
a downstream binary can run the same serve flow with a
`ministr_api::CloudRouterMounter` wired in; the public `ministr` binary calls
it with `None`.

Build from the workspace root:

```sh
cargo install --path ministr-cli --locked
```

Place in the workspace: see the
[architecture overview](../docs/concepts/architecture.md).
