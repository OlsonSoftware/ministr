# The desktop app

The desktop app is a visual manager for your projects and their indexes —
another client of the same daemon the CLI and agents use, so everything it
shows reflects what your AI actually sees. It is built from source
(`ministr-app/`, Tauri); there is no packaged installer at present.

## Home — your projects

Each project is a card: a status rail, a stat strip of index counts, and
the detected tech stack. Card states come from the hash-verified
[freshness](../concepts/freshness.md) contract — up to date, behind your
working tree, or updating while a reindex consumes the changes. Adding a
project is choosing a folder; indexing starts immediately with determinate
progress.

## Managing an index

Opening a project gives the management panel: what's indexed, per-corpus
settings (the same `[corpus]` table as
[.ministr.toml](configuration.md) — the file stays the source of truth),
reindex, and remove. The read state is live: the panel shows the last time
an agent read from the project.

## Connecting an agent

The connect screen shows copy-paste setup for supported clients and then
waits on the real handshake: the first genuine tool call from a connected
agent confirms the wiring end to end — the confirmation is never
simulated. If nothing arrives, the screen offers troubleshooting steps
rather than an indefinite spinner.

## Settings

The settings menu reports the daemon's actual version and state, autostart
control, and links out to this documentation.
