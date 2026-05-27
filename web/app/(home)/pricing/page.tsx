import Link from 'next/link';

export default function PricingPage() {
  return (
    <>
      <section className="v2-section">
        <p className="v2-label">Pricing</p>
        <h1 className="v2-h2">MIT core. Paid cloud coming soon.</h1>
        <p className="v2-sub">
          The local stack is free and MIT-licensed — install it now and use every
          tool with no restrictions. Hosted cloud plans (private-repo indexing, Atlas,
          team features) are coming soon.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2" style={{ fontSize: 28 }}>What you get today — free</h2>
        <div className="v2-features">
          <div className="v2-feature">
            <h3>All 20 MCP tools</h3>
            <p>Semantic search, symbol navigation, reference graphs, cross-language bridges, SOLID detector — the full tool surface.</p>
          </div>
          <div className="v2-feature">
            <h3>40+ languages</h3>
            <p>Rust, Python, TypeScript, Go, Java, C/C++, and dozens more via tree-sitter.</p>
          </div>
          <div className="v2-feature">
            <h3>13 bridge kinds</h3>
            <p>Tauri, PyO3, napi-rs, wasm-bindgen, gRPC, HTTP routes, FFI, and more — cross-language calls your agent can follow.</p>
          </div>
          <div className="v2-feature">
            <h3>Desktop app</h3>
            <p>Tauri v2 dashboard with activity stream, command palette, and system tray. macOS, Windows, Linux.</p>
          </div>
        </div>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2" style={{ fontSize: 28 }}>Coming soon — cloud</h2>
        <p className="v2-sub">
          Hosted indexing, private repos via GitHub App, Atlas (curated OSS network),
          team orgs with ACL, and enterprise SSO/audit. We will announce pricing
          when the cloud launches.
        </p>
        <p className="v2-prose" style={{ marginTop: 24 }}>
          Our stewardship promise: a feature that ships open source will never move to a paid tier.
          The cloud sells hosting, not the toolset.{' '}
          <Link href="/stewardship" style={{ color: 'var(--amber)', textDecoration: 'underline', textUnderlineOffset: '3px' }}>
            Read the full promise.
          </Link>
        </p>
      </section>
    </>
  );
}

export const metadata = {
  title: 'Pricing',
  description:
    'ministr pricing — the local stack is free and MIT-licensed. Hosted cloud plans coming soon.',
};
