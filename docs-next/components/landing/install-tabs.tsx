'use client';

import { useState } from 'react';

const TABS = [
  {
    id: 'macos',
    label: 'macOS',
    body: `curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash`,
  },
  {
    id: 'linux',
    label: 'Linux',
    body: `curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash`,
  },
  {
    id: 'cargo',
    label: 'Cargo',
    body: `cargo install --git https://github.com/AlrikOlson/iris-rs iris-cli`,
  },
] as const;

export function InstallTabs() {
  const [active, setActive] = useState<(typeof TABS)[number]['id']>('macos');
  const current = TABS.find((t) => t.id === active) ?? TABS[0];

  return (
    <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
      <div className="rounded-2xl border border-fd-border bg-fd-card p-5 sm:p-6">
        <div className="mb-4 inline-flex rounded-lg border border-fd-border bg-fd-background p-1">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => setActive(tab.id)}
              className={
                'rounded-md px-3 py-1.5 text-sm font-medium transition ' +
                (active === tab.id
                  ? 'bg-[var(--color-iris-500)] text-white shadow-sm'
                  : 'text-fd-muted-foreground hover:text-fd-foreground')
              }
            >
              {tab.label}
            </button>
          ))}
        </div>
        <pre className="overflow-x-auto rounded-lg border border-fd-border bg-fd-background p-3 font-mono text-sm">
          {current.body}
        </pre>
      </div>
      <div className="mx-auto mt-4 max-w-3xl rounded-2xl border border-fd-border bg-fd-card p-5 sm:p-6">
        <p className="mb-3 text-sm text-fd-muted-foreground">Then initialize and connect:</p>
        <pre className="overflow-x-auto rounded-lg border border-fd-border bg-fd-background p-3 font-mono text-sm">
{`cd your-project
iris init                          # creates .iris.toml + .mcp.json
claude mcp add iris -- iris        # Claude Code`}
        </pre>
        <p className="mt-3 text-xs text-fd-muted-foreground">
          iris auto-discovers <code className="font-mono">.iris.toml</code> from the working directory. No flags needed.
        </p>
      </div>
    </div>
  );
}
