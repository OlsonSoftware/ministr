// F2.5 -- pricing table component.
//
// Server component (zero client JS -- preserves the Lighthouse >= 95
// target on /pricing). Reads tiers from `lib/pricing.ts` so the
// matrix on the page matches ROADMAP S3 byte-for-byte.
//
// Redesigned to match the v2 landing aesthetic: flat bordered grid
// cells (like v2-features), amber accent on the highlighted tier,
// no rounded corners, monospace labels.

import Link from 'next/link';
import { TIERS, type Tier } from '@/lib/pricing';

export function PricingTable() {
  return (
    <div className="v2-tier-grid">
      {TIERS.map((tier) => (
        <TierCard key={tier.slug} tier={tier} />
      ))}
    </div>
  );
}

function TierCard({ tier }: { tier: Tier }) {
  const isExternal = /^(https?:|mailto:)/.test(tier.cta.href);
  return (
    <article
      className={
        'v2-tier-card' + (tier.highlighted ? ' v2-tier-card-highlight' : '')
      }
    >
      <div className="v2-tier-head">
        <h3 className="v2-tier-name">{tier.name}</h3>
        <p className="v2-tier-price">{tier.price}</p>
      </div>
      <p className="v2-tier-tagline">{tier.tagline}</p>
      <ul className="v2-tier-bullets">
        {tier.bullets.map((b) => (
          <li key={b}>{b}</li>
        ))}
      </ul>
      <div className="v2-tier-cta">
        {isExternal ? (
          <a className="v2-btn" href={tier.cta.href}>
            {tier.cta.label}
          </a>
        ) : (
          <Link className="v2-btn" href={tier.cta.href}>
            {tier.cta.label}
          </Link>
        )}
      </div>
    </article>
  );
}
