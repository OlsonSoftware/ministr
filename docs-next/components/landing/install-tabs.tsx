'use client';

import { useState } from 'react';
import { CopyButton } from '@/components/landing/copy-button';

// Step 2's command bundle. `ministr init` already writes .mcp.json
// (Claude Code), .cursor/mcp.json, and .vscode/mcp.json — so the
// canonical wire-up is just the two commands, not three.
const STEP_2_COMMANDS = [
  'cd your-project',
  'ministr init',
].join('\n');

const TABS = [
  {
    id: 'macos',
    label: 'macOS',
    body: 'curl -fsSL https://ministr.app/install.sh | bash',
  },
  {
    id: 'linux',
    label: 'Linux',
    body: 'curl -fsSL https://ministr.app/install.sh | bash',
  },
  {
    id: 'cargo',
    label: 'Cargo',
    body: 'cargo install --git https://github.com/OlsonSoftware/ministr ministr-cli',
  },
] as const;

export function InstallTabs() {
  const [active, setActive] = useState<(typeof TABS)[number]['id']>('macos');
  const current = TABS.find((t) => t.id === active) ?? TABS[0];

  return (
    <div className="relative mx-auto w-full max-w-3xl px-4 sm:px-6">
      <div className="glass-card overflow-hidden p-0">
        <div className="flex items-center justify-between border-b border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-4 py-2.5">
          <div className="flex items-center gap-2">
            <span className="inline-flex size-5 items-center justify-center rounded-full bg-[color-mix(in_oklch,var(--color-ministr-500)_22%,transparent)] text-[10px] font-mono font-bold text-[var(--ministr-accent-text)]">
              1
            </span>
            <span className="ministr-body-quiet text-xs font-mono">install the CLI</span>
          </div>
          <div className="inline-flex rounded-md border border-[color-mix(in_oklch,var(--color-ministr-400)_20%,transparent)] bg-[color-mix(in_oklch,var(--ministr-surface)_50%,transparent)] p-0.5 backdrop-blur">
            {TABS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActive(tab.id)}
                className={
                  'rounded px-2.5 py-1 text-xs font-medium transition ' +
                  (active === tab.id
                    ? 'bg-[var(--color-ministr-600)] text-white shadow-sm'
                    : 'ministr-body-quiet hover:text-fd-foreground')
                }
              >
                {tab.label}
              </button>
            ))}
          </div>
        </div>
        <div className="relative">
          <pre className="overflow-x-auto px-5 py-4 pr-14 font-mono text-sm text-fd-foreground/90">
            <span className="select-none text-[var(--color-ministr-400)]">$ </span>
            {current.body}
          </pre>
          <CopyButton
            value={current.body}
            label={`Copy ${current.label} install command`}
            size="sm"
            className="absolute right-3 top-3"
          />
        </div>
      </div>

      <div className="glass-card mt-4 overflow-hidden p-0">
        <div className="flex items-center gap-2 border-b border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-4 py-2.5">
          <span className="inline-flex size-5 items-center justify-center rounded-full bg-[color-mix(in_oklch,var(--color-ministr-500)_22%,transparent)] text-[10px] font-mono font-bold text-[var(--ministr-accent-text)]">
            2
          </span>
          <span className="ministr-body-quiet text-xs font-mono">wire it into your agent</span>
        </div>
        <div className="relative">
          <pre className="overflow-x-auto px-5 py-4 pr-14 font-mono text-sm text-fd-foreground/90 leading-relaxed">
            <span className="select-none text-[var(--color-ministr-400)]">$ </span>
            cd your-project{'\n'}
            <span className="select-none text-[var(--color-ministr-400)]">$ </span>
            ministr init                          <span className="ministr-body-quiet"># writes .ministr.toml + MCP configs for Claude Code, Cursor, Copilot</span>
          </pre>
          <CopyButton
            value={STEP_2_COMMANDS}
            label="Copy setup commands"
            size="sm"
            className="absolute right-3 top-3"
          />
        </div>
      </div>

      <p className="ministr-body-quiet mt-4 text-center text-xs">
        ministr auto-discovers <code className="font-mono text-[var(--color-ministr-400)]">.ministr.toml</code> from the working directory. No flags needed.
      </p>
    </div>
  );
}
