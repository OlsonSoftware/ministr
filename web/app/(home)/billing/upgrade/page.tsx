// F2.4 -- landing page for the `from=<plan>` upgrade flow.
//
// The actual Stripe Checkout session is minted by the authenticated
// cloud endpoint `POST /api/v1/billing/checkout`; this page exists to
// (a) explain the upcoming flow, (b) deep-link visitors into the
// desktop app where they're signed in and can trigger the call, and
// (c) honour the `?from=<current_plan>` query param so the headline
// matches the section 3 pricing card the user came from.
//
// Why no client-side checkout button: the Stripe Checkout session is
// bound to a server-known `customer_id` (set on first GitHub sign-in
// in F1.5). Surfacing a button here would let an unauthenticated
// browser tab mint a session against another user's customer -- by
// design, the cloud handler reads `Tenant` from the bearer token and
// rejects bearer-less requests with 401.
//
// # Static export
//
// `web/next.config.mjs` uses `output: 'export'`. That forbids
// per-request rendering, so we can't read `searchParams` server-side
// (Next 16 fails the build with `dynamic = "error"`). Instead the
// headline lives in a small client island (`headline-client.tsx`)
// that runs `useSearchParams()` at hydrate -- see the import below.
// The static shell pre-renders at build time; the `?from=` swap
// happens after the bundle loads.

import { Suspense } from 'react';
import Link from 'next/link';

import { UpgradeHeadline } from './headline-client';

export default function BillingUpgradePage() {
  return (
    <div className="ministr-v2">
      {/* useSearchParams() requires a Suspense boundary -- see Next.js
          docs on "Missing Suspense boundary with useSearchParams". */}
      <Suspense fallback={<HeadlineFallback />}>
        <UpgradeHeadline />
      </Suspense>

      <hr className="v2-rule" />

      <section className="v2-section">
        <h2
          className="v2-h2"
         
        >
          How to complete the upgrade
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
              Open the <Link href="/" style={{ color: 'var(--amber)', textDecoration: 'underline' }}>ministr desktop app</Link> (or
              install it from the home page).
            </li>
            <li>
              Open <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>Settings &rarr; Cloud</span> and sign in with
              GitHub.
            </li>
            <li>
              Click <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>Upgrade plan</span> on the cloud panel.
              We mint a Stripe Checkout session bound to your account and open it in
              your browser.
            </li>
            <li>
              Complete payment with a real card (or the test card{' '}
              <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>4242 4242 4242 4242</span> on{' '}
              <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>sk_test_...</span> deployments).
            </li>
            <li>
              Stripe&apos;s webhook fires{' '}
              <span style={{ fontFamily: 'var(--font-mono), monospace', fontSize: '0.9em' }}>customer.subscription.updated</span> against
              our cloud; your plan flips on the next API call.
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
          Already paying? Manage invoices, swap cards, or cancel at{' '}
          <Link href="/billing/manage" style={{ color: 'var(--amber)', textDecoration: 'underline' }}>
            /billing/manage
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

/**
 * Server-rendered fallback shown while the client island hydrates.
 * Defaults to the Pro copy so first paint isn't blank.
 */
function HeadlineFallback() {
  return (
    <section className="v2-section">
      <p className="v2-meta" style={{ marginBottom: '16px' }}>Billing</p>
      <h1 className="v2-h2">Upgrade to Pro</h1>
      <p className="v2-sub">ministr Cloud -- $20 / month.</p>
    </section>
  );
}

export const metadata = {
  title: 'Upgrade',
  description:
    'Upgrade your ministr Cloud subscription. Stripe-hosted Checkout with full PCI offloading.',
};
