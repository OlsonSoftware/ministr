'use client';

import Link from 'next/link';
import { useState } from 'react';
import { ArrowRight, Download } from 'lucide-react';
import { CopyButton } from '@/components/landing/copy-button';
import { INSTALL_COMMANDS, type CliCommandId } from '@/lib/install';

// Step 2's command bundle. `ministr init` already writes .mcp.json
// (Claude Code), .cursor/mcp.json, and .vscode/mcp.json — so the
// canonical wire-up is just the two commands, not three.
const STEP_2_COMMANDS = [
  'cd your-project',
  'ministr init',
].join('\n');

export function InstallTabs() {
  const [active, setActive] = useState<CliCommandId>('macos');
  const current =
    INSTALL_COMMANDS.find((t) => t.id === active) ?? INSTALL_COMMANDS[0];

  return (
    <div className="relative mx-auto w-full max-w-3xl px-4 sm:px-6">
      {/* ── Primary: desktop installer ───────────────────────────────── */}
      <div className="glass-card overflow-hidden p-0">
        <div className="flex items-center gap-2 border-b border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-4 py-2.5">
          <span className="inline-flex size-5 items-center justify-center rounded-full bg-[color-mix(in_oklch,var(--color-ministr-500)_22%,transparent)] text-[10px] font-mono font-bold text-[var(--ministr-accent-text)]">
            1
          </span>
          <span className="ministr-body-quiet text-xs font-mono">install ministr</span>
        </div>
        <div className="flex flex-col gap-4 p-6 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <p className="text-base font-semibold text-fd-foreground">
              Download the installer for your OS
            </p>
            <p className="ministr-body-quiet mt-1 text-sm">
              macOS, Windows &amp; Linux. Double-click to install — the{' '}
              <code className="font-mono text-[var(--color-ministr-400)]">ministr</code>{' '}
              CLI is added to your PATH automatically.
            </p>
          </div>
          <Link
            href="/install"
            className="ministr-cta-primary group inline-flex shrink-0 items-center justify-center gap-2 rounded-lg px-5 py-3 text-sm font-semibold"
          >
            <Download className="size-4" aria-hidden />
            Get the installer
            <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
          </Link>
        </div>
      </div>

      {/* ── Step 2: wire into your agent ─────────────────────────────── */}
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

      {/* ── Secondary: CLI-only one-liner ────────────────────────────── */}
      <div className="glass-card mt-6 overflow-hidden p-0">
        <div className="flex items-center justify-between border-b border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-4 py-2.5">
          <span className="ministr-body-quiet text-xs font-mono">just need the CLI?</span>
          <div className="inline-flex rounded-md border border-[color-mix(in_oklch,var(--color-ministr-400)_20%,transparent)] bg-[color-mix(in_oklch,var(--ministr-surface)_50%,transparent)] p-0.5 backdrop-blur">
            {INSTALL_COMMANDS.map((tab) => (
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
            {current.command}
          </pre>
          {current.note && (
            <p className="ministr-body-quiet border-t border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-5 py-3 text-xs">
              {current.note}
            </p>
          )}
          <CopyButton
            value={current.copyText}
            label={`Copy ${current.label} install command`}
            size="sm"
            className="absolute right-3 top-3"
          />
        </div>
      </div>

      <p className="mt-4 text-center text-xs">
        <Link
          href="/install"
          className="inline-flex items-center gap-1.5 text-fd-muted-foreground transition hover:text-[var(--ministr-accent-text)]"
        >
          All installers, checksums &amp; other download methods
          <ArrowRight className="size-3.5" aria-hidden />
        </Link>
      </p>
    </div>
  );
}
