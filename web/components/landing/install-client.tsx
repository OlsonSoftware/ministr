'use client';

import { useEffect, useMemo, useState } from 'react';
import Link from 'next/link';
import { CopyButton } from '@/components/landing/copy-button';
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

function signingNote(ext: string): string {
  if (ext === 'pkg')
    return 'Signed and notarized by Apple — installs with no Gatekeeper warning.';
  if (ext === 'exe')
    return 'Currently unsigned — SmartScreen will warn on first launch.';
  return 'Currently unsigned — your OS may warn on first launch.';
}

/**
 * Install — the manuscript continued. Same single column, hairline
 * rules, numbered sections, and bordered figures as the landing.
 * OS detection and the latest-release lookup are kept; the chrome
 * (glass cards, pills, badges, motion) is gone.
 */
export function InstallClient() {
  // 'unknown' on first paint avoids a hydration mismatch; the effect
  // below resolves the real OS once on the client.
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
      .then((r) =>
        r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`)),
      )
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

  const primary = useMemo(
    () => DESKTOP_INSTALLERS.find((d) => d.id === OS_TO_DESKTOP[os])!,
    [os],
  );

  const versionLabel = latest ? latest.tag : latestErr ? '—' : 'loading…';

  return (
    <main
      data-ministr-landing
      className="ministr-landing ministr-ms relative isolate overflow-x-hidden"
    >
      <article className="py-20 sm:py-28">
        {/* ── Masthead ─────────────────────────────────────── */}
        <header className="ms-col">
          <p className="ms-folio">Release {versionLabel}</p>
          <h1 className="ms-wordmark mt-5">
            Install ministr<span>.</span>
          </h1>
          <p className="ms-p mt-6">
            Desktop app first — one drop-in installer for macOS, Windows,
            and Linux that also puts the <code>ministr</code> CLI on your
            PATH. If the CLI is all you want, the one-liners are in §3.
          </p>
        </header>

        <Rule className="my-16 sm:my-20" />

        {/* ── §1 Recommended ───────────────────────────────── */}
        <Section folio="§ 1" title="Recommended for your system">
          <figure className="ms-figure mt-2">
            <div className="flex flex-col gap-5 sm:flex-row sm:items-baseline sm:justify-between">
              <div className="min-w-0">
                <p className="font-mono text-[13px] text-[var(--ministr-accent-text)]">
                  {primary.ext.toUpperCase()} · {primary.label}
                </p>
                <p className="ms-p mt-2 !text-[0.95rem]">{primary.hint}</p>
                <p className="mt-1 font-mono text-[12px] text-fd-muted-foreground">
                  {primary.filename}
                </p>
              </div>
              <a
                href={latestDownloadUrl(primary.filename)}
                className="ms-link shrink-0 self-start font-medium text-[15px] sm:self-center"
              >
                Download {primary.ext.toUpperCase()} →
              </a>
            </div>
            <figcaption className="ms-figcap">
              <b>Note</b>
              {signingNote(primary.ext)}
            </figcaption>
          </figure>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §2 All installers ────────────────────────────── */}
        <Section folio="§ 2" title="All desktop installers">
          <p className="ms-p">
            Every platform bundle attached to the latest release
            {latest ? <> ({latest.tag})</> : null}. Copy a direct URL for
            scripted installs.
          </p>
          <div className="ms-rows mt-8">
            {DESKTOP_INSTALLERS.map((d) => (
              <div key={d.id} className="ms-row">
                <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
                  <div className="min-w-0">
                    <span className="font-mono text-[13px] text-[var(--ministr-accent-text)]">
                      {d.ext.toUpperCase()}
                    </span>
                    <span className="ml-3 font-semibold text-fd-foreground">
                      {d.label}
                    </span>
                    <p className="mt-1 truncate font-mono text-[12px] text-fd-muted-foreground" title={d.filename}>
                      {d.filename}
                    </p>
                  </div>
                  <span className="inline-flex items-center gap-4">
                    <a
                      href={latestDownloadUrl(d.filename)}
                      className="ms-link font-medium text-[14px]"
                    >
                      Download →
                    </a>
                    <CopyButton
                      value={
                        latest
                          ? downloadUrl(latest.tag, d.filename)
                          : latestDownloadUrl(d.filename)
                      }
                      label={`Copy direct URL for ${d.label}`}
                      size="sm"
                    />
                  </span>
                </div>
                <p className="ms-p mt-2 !text-[0.875rem]">{d.hint}</p>
              </div>
            ))}
          </div>
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §3 CLI only ──────────────────────────────────── */}
        <Section folio="§ 3" title="Just need the CLI?">
          <p className="ms-p">
            A one-liner for any platform. It installs the{' '}
            <code>ministr</code> binary to <code>~/.ministr/bin</code> (or{' '}
            <code>%USERPROFILE%\.ministr\bin</code> on Windows); add that
            directory to your <code>PATH</code> if the script doesn&rsquo;t.
          </p>

          <div className="mt-7 flex gap-5 text-[13px]">
            {INSTALL_COMMANDS.map((c) => {
              const active = activeCli === c.id;
              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => setActiveCli(c.id)}
                  aria-pressed={active}
                  className={
                    'font-mono uppercase tracking-[0.14em] transition ' +
                    (active
                      ? 'text-[var(--ministr-accent-text)] underline underline-offset-[6px] decoration-1'
                      : 'text-fd-muted-foreground hover:text-fd-foreground')
                  }
                >
                  {c.label}
                </button>
              );
            })}
          </div>

          {INSTALL_COMMANDS.filter((c) => c.id === activeCli).map((c) => (
            <figure key={c.id} className="ms-figure mt-4">
              <div className="flex items-start justify-between gap-4">
                <pre className="ms-mono flex-1">
                  <span className="ms-prompt select-none">$ </span>
                  {c.command}
                </pre>
                <CopyButton
                  value={c.copyText}
                  label={`Copy ${c.label} install command`}
                  size="sm"
                />
              </div>
              {c.note && (
                <figcaption className="ms-figcap">{c.note}</figcaption>
              )}
            </figure>
          ))}
        </Section>

        <Rule className="my-16 sm:my-20" />

        {/* ── §4 Verify / next ─────────────────────────────── */}
        <Section folio="§ 4" title="Verify, then wire it up">
          <div className="ms-rows">
            <div className="ms-row">
              <h3>Verify the download</h3>
              <p>
                Each release ships a unified{' '}
                <a className="ms-link" href={latestDownloadUrl(SHA256SUMS_FILENAME)}>
                  SHA256SUMS
                </a>{' '}
                file plus per-asset <code>.sha256</code> companions.
              </p>
            </div>
            <div className="ms-row">
              <h3>After install</h3>
              <p>
                Run <code>ministr init</code> in your project to wire up
                Claude Code, Cursor, and Copilot at once. See{' '}
                <Link className="ms-link" href="/docs/getting-started">
                  Getting started
                </Link>
                .
              </p>
            </div>
          </div>
        </Section>

        <Rule className="my-16 sm:my-20" />

        <footer className="ms-col">
          <nav className="flex flex-wrap gap-x-6 gap-y-2 text-[14px]">
            <Link href="/" className="ms-link">
              Home
            </Link>
            <Link href="/docs/getting-started" className="ms-link">
              Getting started
            </Link>
            <Link href="/docs/architecture" className="ms-link">
              Architecture
            </Link>
            <Link href="/docs/tools" className="ms-link">
              Tool reference
            </Link>
          </nav>
          <p className="ms-folio mt-8">Local · Rust · no API calls</p>
        </footer>
      </article>
    </main>
  );
}

function Rule({ className = '' }: { className?: string }) {
  return (
    <div className={'ms-col ' + className}>
      <hr className="ms-rule-line" />
    </div>
  );
}

function Section({
  folio,
  title,
  children,
}: {
  folio: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="ms-col">
      <p className="ms-folio">{folio}</p>
      <h2 className="ms-h">{title}</h2>
      <div className="mt-5">{children}</div>
    </section>
  );
}
