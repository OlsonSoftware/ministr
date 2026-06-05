/**
 * LocalVsCloud — the pricing page's core message, at a glance.
 *
 * The page says "MIT core, paid cloud coming soon" and lists the two sides in
 * prose. This shows them side by side so the split is scannable: everything on
 * the left is free and shipping today (amber ✓); everything on the right is the
 * future hosted layer (dim ○, "soon"). No prices — the page deliberately holds
 * those until the cloud launches, and so do we.
 *
 * Static, in the page's own v2 language (warm ink, single amber, sharp corners,
 * hairline rules, mono). Claims are grounded in the page's own copy + README.
 */

const LOCAL: string[] = [
  'All 20 MCP tools',
  '40+ languages',
  '13 cross-language bridge kinds',
  'Desktop app — macOS · Windows · Linux',
  'Runs entirely on your machine',
  'Free forever — MIT-licensed',
];

const CLOUD: string[] = [
  'Hosted indexing',
  'Private repos via GitHub App',
  'Atlas — curated OSS network',
  'Team orgs with ACL',
  'Enterprise SSO + audit',
];

export function LocalVsCloud() {
  return (
    <figure
      className="v2-compare"
      aria-label="ministr at a glance: the local stack is free, MIT-licensed, and available today; the hosted cloud layer is coming soon."
    >
      {/* ── Local — free, today ─────────────────────────────── */}
      <div className="v2-cmp-col">
        <div className="v2-cmp-head">
          <span className="v2-cmp-name v2-cmp-name-amber">Local</span>
          <span className="v2-cmp-tag">free · MIT · today</span>
        </div>
        <ul className="v2-cmp-list">
          {LOCAL.map((item) => (
            <li key={item}>
              <span className="v2-cmp-yes" aria-hidden="true">
                ✓
              </span>
              <span className="v2-cmp-item">{item}</span>
            </li>
          ))}
        </ul>
      </div>

      {/* ── Cloud — coming soon ─────────────────────────────── */}
      <div className="v2-cmp-col v2-cmp-col-soon">
        <div className="v2-cmp-head">
          <span className="v2-cmp-name">Cloud</span>
          <span className="v2-cmp-tag">coming soon</span>
        </div>
        <ul className="v2-cmp-list">
          {CLOUD.map((item) => (
            <li key={item}>
              <span className="v2-cmp-soon" aria-hidden="true">
                ○
              </span>
              <span className="v2-cmp-item v2-cmp-item-soon">{item}</span>
            </li>
          ))}
        </ul>
        <p className="v2-cmp-foot">
          Stewardship promise: anything open-source today stays free, forever.
          The cloud sells hosting, not the toolset.
        </p>
      </div>
    </figure>
  );
}
