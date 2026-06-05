/**
 * InitWiresUp — what `ministr init` actually does, made visible.
 *
 * The page tells you to "run `ministr init`" in a footer card, but the product's
 * whole integration value — one command registers ministr with every assistant
 * in your project — is invisible. This figure shows it: the command, then the
 * exact files it writes (as a diff-add list), each tagged with the assistant it
 * configures.
 *
 * Grounded, not invented: the file set mirrors ministr-core's
 * `write_mcp_configs` (Claude Code + VS Code/Copilot + Cursor) plus the
 * `.ministr.toml` corpus config — and the honest footnote that Codex is left
 * alone because it's user-global, not per-project.
 *
 * Static, in the page's own v2 language (warm ink, single amber, sharp corners,
 * hairline rules, mono). No motion, no new deps.
 */

interface Written {
  file: string;
  configures: string;
}

const WRITES: Written[] = [
  { file: '.ministr.toml', configures: 'Corpus paths — auto-detected from Cargo.toml, package.json, pyproject.toml' },
  { file: '.mcp.json', configures: 'Claude Code' },
  { file: '.cursor/mcp.json', configures: 'Cursor' },
  { file: '.vscode/mcp.json', configures: 'VS Code · GitHub Copilot' },
];

export function InitWiresUp() {
  return (
    <figure
      className="v2-init"
      aria-label="What ministr init writes: a .ministr.toml corpus config plus MCP server configs for Claude Code, Cursor, and VS Code / GitHub Copilot."
    >
      <div className="v2-init-cmd">
        <span className="v2-init-prompt">$</span> ministr init
      </div>
      <div className="v2-init-result">
        ✓ detected your stack · registered ministr for every assistant in the project
      </div>

      <ul className="v2-init-writes">
        {WRITES.map((w) => (
          <li key={w.file}>
            <span className="v2-init-add" aria-hidden="true">
              +
            </span>
            <code className="v2-init-file">{w.file}</code>
            <span className="v2-init-configures">{w.configures}</span>
          </li>
        ))}
      </ul>

      <figcaption className="v2-init-cap">
        One command, every assistant. ministr registers itself as an MCP server
        in each editor&apos;s project config — no copy-pasting JSON. Codex stays
        untouched: it&apos;s user-global, not per-project.
      </figcaption>
    </figure>
  );
}
