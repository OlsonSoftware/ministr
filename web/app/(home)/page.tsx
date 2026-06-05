import Link from 'next/link';
import { GrepVsMinistr } from '../../components/landing/grep-vs-ministr';
import { TokenEconomics } from '../../components/landing/token-economics';
import { CrossLanguageBridge } from '../../components/landing/cross-language-bridge';
import { FourThingsGrid } from '../../components/landing/four-things-grid';

export default function HomePage() {
  return (
    <>
      {/* ── Hero ─────────────────────────────────────────── */}
      <section className="v2-hero">
        <h1 className="v2-wordmark">ministr<span className="v2-dot">.</span></h1>
        <p className="v2-lead">Give your AI agent eyes for code.</p>
        <p className="v2-sub">
          Claude Code, Cursor, and Copilot search code with grep, rg, and find.
          ministr gives them <em className="v2-offer">symbols</em>,{' '}
          <em className="v2-offer">references</em>, and{' '}
          <em className="v2-offer">cross-language calls</em>, indexed on{' '}
          <em className="v2-offer">bare metal</em> and answered in milliseconds.
        </p>
        <div className="v2-cta">
          <Link href="/install" className="v2-btn v2-btn-primary">Install ministr →</Link>
          <Link href="/docs" className="v2-btn">Read the docs</Link>
        </div>

        {/* The thesis, shown not told: grep's noise vs ministr's answer. */}
        <GrepVsMinistr />
      </section>

      <hr className="v2-rule" />

      {/* ── Features ─────────────────────────────────────── */}
      <section className="v2-section">
        <h2 className="v2-h2" style={{ maxWidth: "20ch" }}>Four things grep can&apos;t do.</h2>
        <FourThingsGrid />
      </section>

      <hr className="v2-rule" />

      {/* ── Cross-language ───────────────────────────────── */}
      <section className="v2-section">
        <h2 className="v2-h2" style={{ maxWidth: "20ch" }}>One call, three languages.</h2>
        <CrossLanguageBridge />
      </section>

      <hr className="v2-rule" />

      {/* ── Why ──────────────────────────────────────────── */}
      <section className="v2-section">
        <h2 className="v2-h2" style={{ maxWidth: "20ch" }}>Why ministr.</h2>
        <p className="v2-why-stat">
          Up to <em className="v2-num">90%</em> fewer tokens per task, with answers
          that are <em className="v2-num">structurally correct</em> instead of
          grep-approximated.
        </p>

        {/* The headline number, proven with the real benchmark. */}
        <TokenEconomics />
      </section>

      <hr className="v2-rule" />

      {/* ── Install ──────────────────────────────────────── */}
      <section className="v2-section" id="install">
        <h2 className="v2-h2" style={{ maxWidth: "20ch" }}>One installer. Every platform.</h2>
        <div className="v2-install-block">
          <div className="v2-install-step">
            <div className="v2-step-num">1.</div>
            <div className="v2-step-body">
              <p>Download and double-click. macOS, Windows, Linux. The <code>ministr</code> CLI lands on your PATH automatically.</p>
              <Link href="/install" className="v2-btn v2-btn-primary" style={{ alignSelf: 'flex-start' }}>Download installer →</Link>
            </div>
          </div>
          <div className="v2-install-step">
            <div className="v2-step-num">2.</div>
            <div className="v2-step-body">
              <p>Run <code>ministr init</code> in your project. It wires up Claude Code, Cursor, and Copilot for you.</p>
            </div>
          </div>
        </div>
      </section>

      {/* ── Footer ───────────────────────────────────────── */}
      <footer className="v2-footer">
        <div>
          <div className="v2-brand">
            <svg className="v2-logo" viewBox="0 0 926 926" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <defs>
                <linearGradient id="ministrLogoB" x1="1098.5" y1="-174" x2="-173.5" y2="1092" gradientUnits="userSpaceOnUse">
                  <stop stopColor="#F8AC18"/>
                  <stop offset="1" stopColor="#FF9900"/>
                </linearGradient>
              </defs>
              <path fillRule="evenodd" clipRule="evenodd" d="M926 926H0V0H926V926ZM241 241V685H685V241H241Z" fill="url(#ministrLogoB)"/>
            </svg>
            <span>ministr<span className="v2-dot">.</span></span>
          </div>
          <div className="v2-meta">100% local, open over MCP</div>
        </div>
        <div className="v2-footer-links">
          <Link href="/docs/getting-started">Getting started</Link>
          <Link href="/docs/tools">Tool reference</Link>
          <Link href="/docs/architecture">Architecture</Link>
          <Link href="/stewardship">Stewardship</Link>
        </div>
      </footer>
    </>
  );
}
