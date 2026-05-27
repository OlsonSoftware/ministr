// F2.4 -- Customer Portal entry point.
//
// Same architectural reasoning as `/billing/upgrade`: the
// authenticated cloud handler `POST /api/v1/billing/portal` mints
// the Stripe-hosted portal session bound to the calling user's
// Stripe Customer. This page explains the flow and deep-links into
// the desktop app where the bearer token lives.
//
// Stripe Checkout also redirects here on `success_url` (see
// `ministr-cloud/src/billing/checkout.rs::handle_checkout`) so a
// just-upgraded user lands somewhere coherent.

import Link from 'next/link';

export default function BillingManagePage() {
  return (
    <div className="ministr-v2">
      <section className="v2-section" style={{ paddingTop: '64px' }}>
        <p className="v2-meta" style={{ marginBottom: '16px' }}>Billing</p>
        <h1 className="v2-h2" style={{ maxWidth: 'none' }}>Manage billing</h1>
        <p className="v2-sub">
          Invoices, card on file, plan changes, and cancellation all run
          through Stripe&apos;s Customer Portal.
        </p>
      </section>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2
          className="v2-h2"
          style={{ maxWidth: 'none', fontSize: '20px', marginBottom: '28px' }}
        >
          Open the portal
        </h2>
        <div style={{ maxWidth: '58ch' }}>
          <ol
            style={{
              listStyleType: 'decimal',
              paddingLeft: '1.25rem',
              display: 'flex',
              flexDirection: 'column',
              gap: '14px',
              fontSize: '17px',
              lineHeight: '1.55',
              color: 'var(--ink-2)',
            }}
          >
            <li>
              Open the <Link href="/" style={{ color: 'var(--amber)', textDecoration: 'underline' }}>ministr desktop app</Link>.
            </li>
            <li>
              Open <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>Settings &rarr; Cloud</span>.
            </li>
            <li>
              Click <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>Manage billing</span>. We mint a
              short-lived Stripe Customer Portal session bound to your account and
              open it in your browser.
            </li>
          </ol>
        </div>
      </section>

      <footer className="v2-footer">
        <p
          style={{
            fontFamily: 'var(--font-mono), monospace',
            fontSize: '12px',
            color: 'var(--muted)',
          }}
        >
          Need to start a new subscription instead?{' '}
          <Link href="/billing/upgrade?from=pro" style={{ color: 'var(--amber)', textDecoration: 'underline' }}>
            /billing/upgrade
          </Link>
          .
        </p>
        <div className="v2-footer-links">
          <Link href="/">Home</Link>
          <Link href="/pricing">Pricing</Link>
        </div>
      </footer>
    </div>
  );
}

export const metadata = {
  title: 'Manage billing - ministr',
  description:
    'Open the Stripe Customer Portal to manage your ministr Cloud subscription.',
};
