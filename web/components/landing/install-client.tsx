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
  tag_name?: string;
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
    return () => { cancelled = true; };
  }, []);

  const tag = latest?.tag_name ?? latest?.tag ?? null;
  const versionLabel = tag ?? (latestErr ? '—' : '…');

  const primary = useMemo(
    () => DESKTOP_INSTALLERS.find((d) => d.id === OS_TO_DESKTOP[os])!,
    [os],
  );

  return (
    <main className="install-page">
      {/* ── Hero strip ──────────────────────────────────── */}
      <div className="install-hero">
        <div className="install-wide">
          <p className="install-version">Release {versionLabel}</p>
          <h1 className="install-title">Install ministr</h1>
          <p className="install-sub">
            Desktop app + CLI in one installer. macOS, Windows, Linux.
          </p>
        </div>
      </div>

      <div className="install-wide install-body">
        {/* ── Primary download (detected OS) ─────────────── */}
        <section className="install-primary">
          <div className="install-primary-info">
            <span className="install-badge">{primary.ext.toUpperCase()}</span>
            <div>
              <h2 className="install-primary-label">{primary.label}</h2>
              <p className="install-hint">{primary.hint}</p>
              <p className="install-filename">{primary.filename}</p>
            </div>
          </div>
          <a
            href={latestDownloadUrl(primary.filename)}
            className="install-btn install-btn-primary"
          >
            Download {primary.ext.toUpperCase()}
          </a>
        </section>

        {/* ── All installers grid ────────────────────────── */}
        <h2 className="install-section-title">All platforms</h2>
        <div className="install-grid">
          {DESKTOP_INSTALLERS.map((d) => {
            const isCurrent = d.id === primary.id;
            return (
              <div
                key={d.id}
                className={
                  'install-card' + (isCurrent ? ' install-card-active' : '')
                }
              >
                <div className="install-card-head">
                  <span className="install-badge">{d.ext.toUpperCase()}</span>
                  <span className="install-card-label">{d.label}</span>
                  {isCurrent && (
                    <span className="install-detected">Detected</span>
                  )}
                </div>
                <p className="install-hint">{d.hint}</p>
                <p className="install-filename">{d.filename}</p>
                <div className="install-card-actions">
                  <a
                    href={latestDownloadUrl(d.filename)}
                    className="install-btn"
                  >
                    Download
                  </a>
                  <CopyButton
                    value={
                      tag
                        ? downloadUrl(tag, d.filename)
                        : latestDownloadUrl(d.filename)
                    }
                    label={`Copy URL for ${d.label}`}
                    size="sm"
                  />
                </div>
              </div>
            );
          })}
        </div>

        {/* ── CLI one-liners ─────────────────────────────── */}
        <h2 className="install-section-title">CLI only</h2>
        <p className="install-sub" style={{ marginTop: '-0.5rem' }}>
          Installs the <code>ministr</code> binary to{' '}
          <code>~/.ministr/bin</code>. No desktop app.
        </p>

        <div className="install-cli-tabs">
          {INSTALL_COMMANDS.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => setActiveCli(c.id)}
              aria-pressed={activeCli === c.id}
              className={
                'install-cli-tab' +
                (activeCli === c.id ? ' install-cli-tab-active' : '')
              }
            >
              {c.label}
            </button>
          ))}
        </div>

        {INSTALL_COMMANDS.filter((c) => c.id === activeCli).map((c) => (
          <div key={c.id} className="install-cli-block">
            <pre className="install-cli-code">
              <span className="install-prompt">$ </span>
              {c.command}
            </pre>
            <CopyButton
              value={c.copyText}
              label={`Copy ${c.label} command`}
              size="sm"
            />
          </div>
        ))}

        {/* ── Verify + next steps ─────────────────────────── */}
        <div className="install-next-row">
          <div className="install-next-card">
            <h3>Verify</h3>
            <p>
              Each release ships{' '}
              <a className="install-link" href={latestDownloadUrl(SHA256SUMS_FILENAME)}>
                SHA256SUMS
              </a>{' '}
              plus per-asset <code>.sha256</code> files.
            </p>
          </div>
          <div className="install-next-card">
            <h3>Wire it up</h3>
            <p>
              Run <code>ministr init</code> in your project.{' '}
              <Link className="install-link" href="/docs/getting-started">
                Getting started →
              </Link>
            </p>
          </div>
          <div className="install-next-card">
            <h3>Docs</h3>
            <p>
              <Link className="install-link" href="/docs/tools">
                Tool reference
              </Link>
              {' · '}
              <Link className="install-link" href="/docs/architecture">
                Architecture
              </Link>
              {' · '}
              <Link className="install-link" href="/docs/configuration">
                Config
              </Link>
            </p>
          </div>
        </div>
      </div>
    </main>
  );
}
