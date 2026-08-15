# ministr-tui

The terminal console for the ministr index engine, opened with `ministr ui`.

A full-screen ratatui instrument for index management: see which projects
are indexed, whether anything needs an update, and whether anything is
building — then rebuild, add, or remove projects without leaving the
terminal. The binding design direction is `GUI-BLUEPRINT-v8.md` in the
planning repo (Master-Control Console: one channel strip per project, a
master section for machine state, exactly one accent color reserved for
live activity).

## Place in the workspace

- Talks to the engine through `ministr_api::client::DaemonClient` over the
  local transport — the same client every other surface uses. If no engine
  is running it spawns one, detached, exactly like the MCP proxy does.
- Surfaced as the `ministr ui` subcommand in `ministr-cli`; this crate owns
  terminal setup/teardown (raw mode, alternate screen, panic-hook restore),
  the event loop, and every frame.

## Verification

Every screen state renders deterministically through ratatui's
`TestBackend` and is pinned as an `insta` snapshot (`tests/snapshot.rs`) —
a state without a snapshot does not ship. The UI language rules
(plain words only — "project" never "corpus", "engine" never "daemon",
no emoji, no exclamation marks) are enforced mechanically by
`tests/language.rs`, which scans every string literal in `src/`.

Run the crate's gates with `cargo test -p ministr-tui`; the workspace gate
is `just validate`.
