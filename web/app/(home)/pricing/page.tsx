import Link from 'next/link';
import { PricingTable } from '@/components/landing/pricing-table';
import { POSITIONING_LINE } from '@/lib/pricing';

export default function PricingPage() {
  return (
    <div className="ministr-v2">
      <section className="v2-section" style={{ paddingTop: '64px' }}>
        <p className="v2-meta" style={{ marginBottom: '16px' }}>Pricing</p>
        <h1 className="v2-h2" style={{ maxWidth: 'none' }}>MIT core. Paid cloud.</h1>
        <p className="v2-sub" style={{ marginBottom: '48px' }}>{POSITIONING_LINE}</p>
        <PricingTable />
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2 className="v2-h2">Our promise</h2>
        <p style={{ color: 'var(--ink-2)', fontSize: '17px', lineHeight: '1.55', maxWidth: '58ch' }}>
          When a feature is open source, we won&apos;t move that feature to a paid
          tier. The local stack stays MIT forever. The cloud sells{' '}
          <em className="v2-offer">hosting + scale + team + compliance</em>, not the toolset itself.{' '}
          <Link href="/stewardship" style={{ color: 'var(--amber)', textDecoration: 'underline' }}>
            Read the stewardship promise.
          </Link>
        </p>
      </section>

      <footer className="v2-footer">
        <div className="v2-footer-links">
          <Link href="/">Home</Link>
          <Link href="/stewardship">Stewardship</Link>
          <Link href="/docs">Docs</Link>
        </div>
      </footer>
    </div>
  );
}

export const metadata = {
  title: 'Pricing — ministr',
  description:
    'ministr pricing. Free MIT local stack. Hosted cloud from $20/month. Enterprise on contact.',
};
