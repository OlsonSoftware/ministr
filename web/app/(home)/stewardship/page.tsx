import Link from 'next/link';

export default function StewardshipPage() {
  return (
    <div className="ministr-v2">
      <section className="v2-section" style={{ paddingTop: '64px' }}>
        <p className="v2-meta" style={{ marginBottom: '16px' }}>Stewardship</p>
        <h1 className="v2-h2" style={{ maxWidth: 'none' }}>ministr stewardship</h1>
        <p className="v2-sub">
          Our open-core posture and public commitment to contributors and users.
          Borrowed in shape from{' '}
          <a href="https://handbook.gitlab.com/handbook/company/stewardship/" style={{ color: 'var(--amber)' }}>
            GitLab&apos;s stewardship handbook
          </a>.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2">The promise</h2>
        <p style={{ color: 'var(--ink)', fontWeight: 500, fontSize: '18px', marginBottom: '16px' }}>
          When a feature is open source, we won&apos;t move that feature to a paid tier.
        </p>
        <p style={{ color: 'var(--ink-2)', lineHeight: '1.55' }}>
          A feature that ships under MIT in this repository stays under MIT. We may
          remove a feature outright if the underlying capability is being removed from
          the whole product. We will not paywall existing open-source functionality.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2">What is MIT</h2>
        <p style={{ color: 'var(--ink-2)', lineHeight: '1.55' }}>
          The local stack — everything that runs on a user&apos;s own machine — is
          MIT-licensed. The six core workspace crates (<code>ministr-core</code>,{' '}
          <code>ministr-api</code>, <code>ministr-daemon</code>,{' '}
          <code>ministr-mcp</code>, <code>ministr-cli</code>,{' '}
          <code>ministr-app/src-tauri</code>) carry MIT licences and will keep them.
          All 20 MCP tools, the SOLID detector, 13 cross-language bridge detectors,
          ~40 language parsers, claim extraction, session shadow, and coherence
          tracking are part of the MIT half.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2">What is closed</h2>
        <p style={{ color: 'var(--ink-2)', lineHeight: '1.55' }}>
          The hosted ministr Cloud service at <code>mcp.ministr.ai</code> and
          the Enterprise on-prem image are paid products. The code that exists{' '}
          <em>only because</em> we run a multi-tenant service or sell an enterprise
          SKU lives in proprietary crates: <code>ministr-cloud</code>,{' '}
          <code>ministr-enterprise</code>, <code>ministr-atlas</code>, and{' '}
          <code>ministr-atlas-mirror</code>. None of this code is useful on the
          local stack.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2">In practice</h2>
        <div className="v2-features" style={{ gridTemplateColumns: '1fr 1fr' }}>
          <div className="v2-feature">
            <h3>Forks welcome</h3>
            <p>MIT explicitly permits commercial use, modification, and redistribution.</p>
          </div>
          <div className="v2-feature">
            <h3>Tools are open</h3>
            <p>All 19 MCP tools are MIT and will remain MIT.</p>
          </div>
          <div className="v2-feature">
            <h3>Self-host is full</h3>
            <p>The cloud sells hosting + scale + team + compliance, not the toolset itself.</p>
          </div>
          <div className="v2-feature">
            <h3>No relicensing</h3>
            <p>Contributors keep copyright, inbound=outbound under MIT. We will not relicense.</p>
          </div>
        </div>
      </section>

      <footer className="v2-footer">
        <div style={{ color: 'var(--muted)', fontSize: '12px', fontFamily: 'var(--font-mono), monospace' }}>
          Sourcegraph killed Cody Free/Pro in July 2025 and went Enterprise-only. That move motivates this document.
        </div>
        <div className="v2-footer-links">
          <Link href="/pricing">Pricing</Link>
          <Link href="/">Home</Link>
        </div>
      </footer>
    </div>
  );
}

export const metadata = {
  title: 'Stewardship — ministr',
  description:
    'ministr stewardship: open-core posture, the never-demote promise, and what stays MIT.',
};
