// F2.5 — `/stewardship` page.
//
// Mirrors the canonical STEWARDSHIP.md at the repo root. Hand-
// rendered as JSX rather than loaded via MDX because:
// - the static-export pipeline doesn't need a runtime MDX loader for
//   one document
// - keeping the content here next to the pricing page lets the
//   marketing edit cycle stay in one place
// - the source-of-truth file (`/STEWARDSHIP.md`) is still the
//   primary doc; sync via PR review when wording changes there
//
// If the markdown grows substantially we'll switch to fumadocs-mdx;
// the route stays `/stewardship` so links don't churn.

import Link from 'next/link';

export default function StewardshipPage() {
  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-8 p-8 text-[15px] leading-relaxed">
      <header className="flex flex-col gap-2">
        <p className="font-mono text-xs uppercase tracking-[0.18em] text-fd-muted-foreground">
          Stewardship
        </p>
        <h1 className="text-3xl font-semibold">ministr stewardship</h1>
        <p className="text-fd-muted-foreground">
          Our open-core posture and public commitment to contributors and users.
          Borrowed in shape — and partly in phrasing — from{' '}
          <a
            className="underline"
            href="https://handbook.gitlab.com/handbook/company/stewardship/"
          >
            GitLab&rsquo;s stewardship handbook
          </a>
          .
        </p>
      </header>

      <section>
        <h2 className="mb-2 text-xl font-semibold">The promise</h2>
        <p className="font-semibold">
          When a feature is open source, we won&rsquo;t move that feature to a
          paid tier.
        </p>
        <p className="mt-2 text-fd-muted-foreground">
          A feature that ships under MIT in this repository stays under MIT. We
          may remove a feature outright if the underlying capability is being
          removed from the whole product. We will not paywall existing
          open-source functionality.
        </p>
      </section>

      <section>
        <h2 className="mb-2 text-xl font-semibold">What is MIT (and stays MIT)</h2>
        <p className="text-fd-muted-foreground">
          The local stack — everything that runs on a user&rsquo;s own machine —
          is MIT-licensed. The six core workspace crates ({' '}
          <code>ministr-core</code>, <code>ministr-api</code>,{' '}
          <code>ministr-daemon</code>, <code>ministr-mcp</code>,{' '}
          <code>ministr-cli</code>, <code>ministr-app/src-tauri</code>) carry
          MIT licences and will keep them. All 19 MCP tools, the SOLID detector,
          12 cross-language bridge detectors, ~40 language parsers, claim
          extraction, session shadow, and coherence tracking are part of the
          MIT half.
        </p>
      </section>

      <section>
        <h2 className="mb-2 text-xl font-semibold">What is closed (and why)</h2>
        <p className="text-fd-muted-foreground">
          The hosted ministr Cloud service at{' '}
          <code>mcp.ministr.ai</code> and the Enterprise on-prem image are
          paid products. The code that exists <em>only because</em> we run a
          multi-tenant service or sell an enterprise SKU lives in proprietary
          crates: <code>ministr-cloud</code>, <code>ministr-enterprise</code>,{' '}
          <code>ministr-atlas</code>, and <code>ministr-atlas-mirror</code>.
          None of this code is useful on the local stack — keeping it closed
          is how the cloud and enterprise products fund the open core.
        </p>
      </section>

      <section>
        <h2 className="mb-2 text-xl font-semibold">In practice</h2>
        <ul className="ml-5 list-disc space-y-2 text-fd-muted-foreground">
          <li>
            <strong>Forks are welcome.</strong> MIT explicitly permits commercial
            use, modification, and redistribution.
          </li>
          <li>
            <strong>The MCP tool surface is open.</strong> All 19 tools are MIT
            and will remain MIT.
          </li>
          <li>
            <strong>Self-host is fully featured.</strong> The cloud sells{' '}
            <em>hosting + scale + team + compliance</em>, not the toolset itself.
          </li>
          <li>
            <strong>No relicensing trap.</strong> Contributors keep copyright,
            inbound=outbound under MIT. We will not relicense the OSS crates.
          </li>
        </ul>
      </section>

      <section className="text-xs text-fd-muted-foreground">
        Sourcegraph killed Cody Free and Cody Pro in July 2025 and went
        Enterprise-only. That move is the cautionary tale that motivates this
        document. If we ever break this commitment, hold us to it.
      </section>

      <footer className="flex flex-wrap gap-3 border-t border-fd-border pt-4 text-sm">
        <Link className="underline" href="/pricing">
          ← Pricing
        </Link>
        <Link className="underline" href="/">
          Home
        </Link>
      </footer>
    </main>
  );
}

export const metadata = {
  title: 'Stewardship — ministr',
  description:
    'ministr stewardship: open-core posture, the never-demote promise, and what stays MIT.',
};
