// F2.5 — pricing table component.
//
// Server component (zero client JS — preserves the Lighthouse ≥ 95
// target on /pricing). Reads tiers from `lib/pricing.ts` so the
// matrix on the page matches ROADMAP §3 byte-for-byte.
//
// SOLID: single responsibility — render the table. Tier shape comes
// from the data module; the page composes layout (header, hero,
// footer) around this component.

import Link from 'next/link';
import { TIERS, type Tier } from '@/lib/pricing';

export function PricingTable() {
  return (
    <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
      {TIERS.map((tier) => (
        <TierCard key={tier.slug} tier={tier} />
      ))}
    </div>
  );
}

function TierCard({ tier }: { tier: Tier }) {
  const isExternal = /^(https?:|mailto:)/.test(tier.cta.href);
  const borderClass = tier.highlighted
    ? 'border-fd-primary'
    : 'border-fd-border';
  return (
    <article
      className={`flex flex-col gap-4 rounded-lg border p-5 ${borderClass}`}
    >
      <header className="flex flex-col gap-1">
        <h3 className="text-lg font-semibold">{tier.name}</h3>
        <p className="text-2xl font-mono">{tier.price}</p>
        <p className="text-xs text-fd-muted-foreground">{tier.tagline}</p>
      </header>
      <ul className="flex flex-col gap-2 text-sm">
        {tier.bullets.map((b) => (
          <li key={b} className="flex gap-2">
            <span aria-hidden className="text-fd-muted-foreground">·</span>
            <span>{b}</span>
          </li>
        ))}
      </ul>
      <div className="mt-auto pt-2">
        {isExternal ? (
          <a
            className="inline-flex items-center rounded-md border border-fd-border px-3 py-1.5 text-sm hover:border-fd-primary"
            href={tier.cta.href}
          >
            {tier.cta.label}
          </a>
        ) : (
          <Link
            className="inline-flex items-center rounded-md border border-fd-border px-3 py-1.5 text-sm hover:border-fd-primary"
            href={tier.cta.href}
          >
            {tier.cta.label}
          </Link>
        )}
      </div>
    </article>
  );
}
