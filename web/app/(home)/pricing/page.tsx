// F2.5 — `/pricing` page.
//
// Static server component. The tier matrix lives in `lib/pricing.ts`
// and is rendered via the SOLID-split `PricingTable` component so
// the same source feeds future landing snippets. Zero client JS on
// this page keeps the Lighthouse target (≥95) safely in reach.

import Link from 'next/link';

import { BridgeGraphHero } from '@/components/landing/bridge-graph';
import { PricingTable } from '@/components/landing/pricing-table';
import { POSITIONING_LINE } from '@/lib/pricing';

export default function PricingPage() {
  return (
    <main className="mx-auto flex max-w-6xl flex-col gap-10 p-8">
      <header className="flex flex-col gap-3">
        <p className="font-mono text-xs uppercase tracking-[0.18em] text-fd-muted-foreground">
          Pricing
        </p>
        <h1 className="text-3xl font-semibold sm:text-4xl">
          MIT core. Paid cloud.
        </h1>
        <p className="max-w-3xl text-fd-muted-foreground">{POSITIONING_LINE}</p>
      </header>

      <PricingTable />

      <section className="rounded-lg border border-fd-border p-6">
        <header className="mb-4 flex flex-col gap-1">
          <h2 className="text-lg font-medium">The differentiator</h2>
          <p className="text-sm text-fd-muted-foreground">
            Twelve bridge detectors × ~40 language parsers. Every paid tier
            queries the same cross-language graph; Team adds the interactive
            visualizer (F3.6).
          </p>
        </header>
        <BridgeGraphHero
          className="mx-auto max-w-2xl"
          caption={
            <>
              <strong>Cross-language bridges, one query surface.</strong>{' '}
              Tauri commands wire JS to Rust; PyO3 wires Rust to Python; NAPI
              wires Rust to TypeScript. <code>ministr_bridge</code> resolves
              every edge across language boundaries — local stack today, cloud
              + team visualizer for paid tiers.
            </>
          }
        />
      </section>

      <section className="rounded-lg border border-fd-border p-6 text-sm">
        <h2 className="mb-2 text-lg font-medium">Our promise</h2>
        <p className="text-fd-muted-foreground">
          When a feature is open source, we won&rsquo;t move that feature to a paid
          tier. The local stack stays MIT forever. The cloud sells{' '}
          <em>hosting + scale + team + compliance</em>, not the toolset itself.{' '}
          <Link className="underline" href="/stewardship">
            Read the stewardship promise.
          </Link>
        </p>
      </section>

      <section className="flex flex-col gap-2 text-xs text-fd-muted-foreground">
        <p>
          Numbers above mirror{' '}
          <code>ROADMAP §3</code> byte-for-byte. Local install:{' '}
          <Link className="underline" href="/install">
            /install
          </Link>
          .
        </p>
      </section>
    </main>
  );
}

export const metadata = {
  title: 'Pricing — ministr',
  description:
    'ministr pricing. Free MIT local stack. Hosted cloud from $20/month. Enterprise on contact.',
};
