import Link from 'next/link';

const REPO_URL = 'https://github.com/OlsonSoftware/ministr';

export default function HomePage() {
  return (
    <>
      {/* ── Hero ─────────────────────────────────────────── */}
      <section className="v2-hero">
        <h1 className="v2-wordmark">ministr<span className="v2-dot">.</span></h1>
        <p className="v2-lead">Code intelligence for your AI coding agent.</p>
        <p className="v2-sub">
          ministr helps AI coding assistants — Claude Code, Cursor, Copilot, and
          any MCP client — actually understand your codebase. Instead of guessing
          from plain-text search, your agent can jump to where something is
          defined, find everything that uses it, and follow calls across
          languages. It runs entirely on your machine.
        </p>
        <div className="v2-cta">
          <a
            href={REPO_URL}
            className="v2-btn v2-btn-primary"
            target="_blank"
            rel="noopener noreferrer"
          >
            View the source on GitHub →
          </a>
        </div>
      </section>

      <hr className="v2-rule" />

      {/* ── What it does for you ─────────────────────────── */}
      <section className="v2-section">
        <h2 className="v2-h2" style={{ maxWidth: '20ch' }}>What it does for you.</h2>
        <ul style={{ listStyle: 'none', padding: 0, margin: 0, display: 'grid', gap: '1.25rem', maxWidth: '60ch' }}>
          <li>
            <strong>Fewer wrong guesses.</strong> Your agent works from real
            definitions and references instead of grep approximations, so it
            reads and edits the right code.
          </li>
          <li>
            <strong>Works with the tools you already use.</strong> ministr speaks
            MCP, so Claude Code, Cursor, Copilot, and other MCP clients can all
            use it — no new workflow to learn.
          </li>
          <li>
            <strong>Stays on your machine.</strong> Your code is indexed locally.
            Nothing is uploaded.
          </li>
        </ul>
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
          <a href={REPO_URL} target="_blank" rel="noopener noreferrer">GitHub</a>
        </div>
      </footer>
    </>
  );
}
