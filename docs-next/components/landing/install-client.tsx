'use client';

import { useEffect, useMemo, useState } from 'react';
import Link from 'next/link';
import { ArrowRight, Download, Terminal } from 'lucide-react';
import { CopyButton } from '@/components/landing/copy-button';
import { GlassCard } from '@/components/landing/glass-card';
import {
  DESKTOP_INSTALLERS,
  INSTALL_COMMANDS,
  SHA256SUMS_FILENAME,
  detectOsFamily,
  downloadUrl,
  latestDownloadUrl,
  latestReleaseUrl,
  type CliCommandId,
  type DesktopPlatformId,
  type OsFamily,
} from '@/lib/install';

interface LatestMeta {
  tag: string;
  name?: string;
  published_at?: string;
}

const OS_TO_DESKTOP: Record<OsFamily, DesktopPlatformId> = {
  macos: 'macos-aarch64',
  windows: 'windows-x64',
  linux: 'linux-deb',
  unknown: 'macos-aarch64',
};

const OS_TO_CLI: Record<OsFamily, CliCommandId> = {
  macos: 'macos',
  windows: 'windows',
  linux: 'linux',
  unknown: 'macos',
};

export function InstallClient() {
  // Default to 'unknown' on first paint to avoid hydration mismatches; the
  // useEffect below populates the real OS once we're on the client.
  const [os, setOs] = useState<OsFamily>('unknown');
  const [latest, setLatest] = useState<LatestMeta | null>(null);
  const [latestErr, setLatestErr] = useState<string | null>(null);
  const [activeCli, setActiveCli] = useState<CliCommandId>('macos');

  useEffect(() => {
    const detected = detectOsFamily(
      typeof navigator !== 'undefined' ? navigator.userAgent : null,
    );
    setOs(detected);
    setActiveCli(OS_TO_CLI[detected]);
  }, []);

  useEffect(() => {
    let cancelled = false;
    fetch(latestReleaseUrl(), { cache: 'no-store' })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`))))
      .then((meta: LatestMeta) => {
        if (!cancelled) setLatest(meta);
      })
      .catch((e) => {
        if (!cancelled) setLatestErr(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const primaryInstaller = useMemo(
    () => DESKTOP_INSTALLERS.find((d) => d.id === OS_TO_DESKTOP[os])!,
    [os],
  );

  const tag = latest?.tag ?? 'latest';
  const versionLabel = latest ? latest.tag : latestErr ? '—' : 'loading…';

  return (
    <div className="ministr-landing relative isolate flex flex-col items-stretch overflow-x-hidden pb-0">
      {/* ── Hero ───────────────────────────────────────────────────────── */}
      <section className="relative w-full pt-24 pb-12 sm:pt-28 sm:pb-16">
        <div className="mx-auto w-full max-w-4xl px-4 sm:px-6 text-center">
          <span className="inline-flex items-center gap-2 rounded-full border border-[color-mix(in_oklch,var(--color-ministr-400)_28%,transparent)] bg-[color-mix(in_oklch,var(--ministr-surface)_60%,transparent)] px-3 py-1 text-[11px] font-mono text-fd-muted-foreground backdrop-blur">
            <Download className="size-3.5 text-[var(--color-ministr-400)]" aria-hidden />
            ministr {versionLabel}
          </span>

          <h1 className="ministr-hero-mark mt-6 text-[clamp(2.5rem,6vw,4.5rem)] font-semibold leading-[0.95] tracking-tight text-fd-foreground">
            Install ministr<span className="text-[var(--color-ministr-500)]">.</span>
          </h1>

          <p className="ministr-body mx-auto mt-5 max-w-[52ch] text-[15.5px] leading-relaxed">
            Desktop app first — drop-in installer for macOS, Windows, and Linux.
            CLI-only install scripts are below if that&rsquo;s all you need.
          </p>
        </div>
      </section>

      {/* ── Primary desktop download (OS-detected) ─────────────────────── */}
      <section className="relative pb-12">
        <div className="mx-auto w-full max-w-4xl px-4 sm:px-6">
          <p className="ministr-eyebrow text-center">Recommended for your system</p>
          <div className="mt-6">
            <GlassCard padded={false} className="overflow-hidden">
              <div className="flex flex-col gap-6 p-6 sm:flex-row sm:items-center sm:justify-between sm:p-8">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-3">
                    <PlatformBadge ext={primaryInstaller.ext} />
                    <h2 className="text-xl font-semibold text-fd-foreground sm:text-2xl">
                      {primaryInstaller.label}
                    </h2>
                  </div>
                  <p className="ministr-body-quiet mt-2 text-sm">{primaryInstaller.hint}</p>
                  <p className="ministr-body-quiet mt-1 font-mono text-xs">
                    {primaryInstaller.filename}
                  </p>
                </div>
                <div className="flex flex-col items-stretch gap-2 sm:items-end">
                  <a
                    href={latestDownloadUrl(primaryInstaller.filename)}
                    className="ministr-cta-primary group inline-flex items-center justify-center gap-2 rounded-lg px-5 py-3 text-base font-semibold"
                  >
                    Download {primaryInstaller.ext.toUpperCase()}
                    <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" aria-hidden />
                  </a>
                  <p className="ministr-body-quiet text-center text-[11px] sm:text-right">
                    Unsigned in v0.2.x — Gatekeeper / SmartScreen will warn on first launch.
                  </p>
                </div>
              </div>
            </GlassCard>
          </div>
        </div>
      </section>

      {/* ── All desktop installers ─────────────────────────────────────── */}
      <section className="relative pb-16">
        <div className="mx-auto w-full max-w-5xl px-4 sm:px-6">
          <div className="flex items-baseline justify-between gap-4">
            <p className="ministr-eyebrow">All desktop installers</p>
            {tag !== 'latest' && (
              <span className="ministr-body-quiet font-mono text-xs">{tag}</span>
            )}
          </div>
          <div className="mt-5 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            {DESKTOP_INSTALLERS.map((d) => (
              <GlassCard key={d.id} padded={false} className="flex h-full flex-col p-5">
                <div className="flex items-center gap-2">
                  <PlatformBadge ext={d.ext} />
                  <span className="text-sm font-semibold text-fd-foreground">{d.label}</span>
                </div>
                <p className="ministr-body-quiet mt-2 flex-1 text-xs">{d.hint}</p>
                <p className="mt-3 truncate font-mono text-[11px] text-fd-muted-foreground" title={d.filename}>
                  {d.filename}
                </p>
                <div className="mt-3 flex items-center gap-2">
                  <a
                    href={latestDownloadUrl(d.filename)}
                    className="ministr-cta-primary inline-flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-2 text-sm font-medium"
                  >
                    Download
                  </a>
                  <CopyButton
                    value={
                      latest
                        ? downloadUrl(latest.tag, d.filename)
                        : latestDownloadUrl(d.filename)
                    }
                    label={`Copy direct URL for ${d.label}`}
                    size="md"
                  />
                </div>
              </GlassCard>
            ))}
          </div>
        </div>
      </section>

      {/* ── CLI tabs (secondary) ───────────────────────────────────────── */}
      <section className="relative pb-16">
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <p className="ministr-eyebrow flex items-center gap-2">
            <Terminal className="size-3.5" aria-hidden />
            Just need the CLI?
          </p>
          <h2 className="mt-3 text-2xl font-semibold leading-tight tracking-tight text-fd-foreground sm:text-3xl">
            One-liner for any platform.
          </h2>
          <p className="ministr-body mt-3 text-[14.5px]">
            Installs the <code className="font-mono">ministr</code> binary to{' '}
            <code className="font-mono">~/.ministr/bin</code> (or{' '}
            <code className="font-mono">%USERPROFILE%\.ministr\bin</code> on Windows). Add that
            directory to your <code className="font-mono">PATH</code> if the script doesn&rsquo;t do
            it for you.
          </p>

          <div className="mt-6">
            <GlassCard padded={false} className="overflow-hidden p-0">
              <div className="flex items-center justify-between border-b border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-4 py-2.5">
                <span className="ministr-body-quiet font-mono text-xs">install the CLI</span>
                <div className="inline-flex rounded-md border border-[color-mix(in_oklch,var(--color-ministr-400)_20%,transparent)] bg-[color-mix(in_oklch,var(--ministr-surface)_50%,transparent)] p-0.5 backdrop-blur">
                  {INSTALL_COMMANDS.map((c) => (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => setActiveCli(c.id)}
                      className={
                        'rounded px-2.5 py-1 text-xs font-medium transition ' +
                        (activeCli === c.id
                          ? 'bg-[var(--color-ministr-600)] text-white shadow-sm'
                          : 'ministr-body-quiet hover:text-fd-foreground')
                      }
                    >
                      {c.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="relative">
                {INSTALL_COMMANDS.filter((c) => c.id === activeCli).map((c) => (
                  <div key={c.id}>
                    <pre className="overflow-x-auto px-5 py-4 pr-14 font-mono text-sm text-fd-foreground/90">
                      <span className="select-none text-[var(--color-ministr-400)]">$ </span>
                      {c.command}
                    </pre>
                    {c.note && (
                      <p className="ministr-body-quiet border-t border-[color-mix(in_oklch,var(--color-ministr-400)_18%,transparent)] px-5 py-3 text-xs">
                        {c.note}
                      </p>
                    )}
                    <CopyButton
                      value={c.copyText}
                      label={`Copy ${c.label} install command`}
                      size="sm"
                      className="absolute right-3 top-3"
                    />
                  </div>
                ))}
              </div>
            </GlassCard>
          </div>
        </div>
      </section>

      {/* ── Verify + footer ─────────────────────────────────────────────── */}
      <section className="relative pb-24">
        <div className="mx-auto w-full max-w-3xl px-4 sm:px-6">
          <div className="ministr-spectrum-rule" />
          <div className="mt-8 grid gap-6 text-sm sm:grid-cols-2">
            <div>
              <p className="ministr-eyebrow">Verify</p>
              <p className="ministr-body-quiet mt-2">
                Each release ships a unified{' '}
                <a
                  className="text-[var(--ministr-accent-text)] underline-offset-2 hover:underline"
                  href={latestDownloadUrl(SHA256SUMS_FILENAME)}
                >
                  SHA256SUMS
                </a>
                {' '}file plus per-asset{' '}
                <code className="font-mono">.sha256</code> companions.
              </p>
            </div>
            <div>
              <p className="ministr-eyebrow">After install</p>
              <p className="ministr-body-quiet mt-2">
                Run{' '}
                <code className="font-mono text-[var(--color-ministr-400)]">ministr init</code>{' '}
                in your project to wire up Claude Code, Cursor, and Copilot. See{' '}
                <Link
                  className="text-[var(--ministr-accent-text)] underline-offset-2 hover:underline"
                  href="/docs/getting-started"
                >
                  Getting Started
                </Link>
                .
              </p>
            </div>
          </div>

          <div className="mt-10 flex flex-wrap justify-center gap-x-6 gap-y-2 text-[13px]">
            <Link
              href="/docs/architecture"
              className="text-fd-muted-foreground transition hover:text-[var(--ministr-accent-text)]"
            >
              Architecture
            </Link>
            <Link
              href="/docs/tools"
              className="text-fd-muted-foreground transition hover:text-[var(--ministr-accent-text)]"
            >
              Tool reference
            </Link>
          </div>
        </div>
      </section>
    </div>
  );
}

function PlatformBadge({ ext }: { ext: 'dmg' | 'exe' | 'deb' | 'AppImage' }) {
  const label = ext.toUpperCase();
  return (
    <span className="inline-flex h-7 min-w-12 items-center justify-center rounded-md border border-[color-mix(in_oklch,var(--color-ministr-400)_28%,transparent)] bg-[color-mix(in_oklch,var(--color-ministr-500)_18%,transparent)] px-2 font-mono text-[11px] font-semibold text-[var(--ministr-accent-text)]">
      {label}
    </span>
  );
}
