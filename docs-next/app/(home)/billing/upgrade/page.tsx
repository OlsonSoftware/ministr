// F2.4 — landing page for the `from=<plan>` upgrade flow.
//
// The actual Stripe Checkout session is minted by the authenticated
// cloud endpoint `POST /api/v1/billing/checkout`; this page exists to
// (a) explain the upcoming flow, (b) deep-link visitors into the
// desktop app where they're signed in and can trigger the call, and
// (c) honour the `?from=<current_plan>` query param so the headline
// matches the section §3 pricing card the user came from.
//
// Why no client-side checkout button: the Stripe Checkout session is
// bound to a server-known `customer_id` (set on first GitHub sign-in
// in F1.5). Surfacing a button here would let an unauthenticated
// browser tab mint a session against another user's customer — by
// design, the cloud handler reads `Tenant` from the bearer token and
// rejects bearer-less requests with 401.
//
// # Static export
//
// `docs-next/next.config.mjs` uses `output: 'export'`. That forbids
// per-request rendering, so we can't read `searchParams` server-side
// (Next 16 fails the build with `dynamic = "error"`). Instead the
// headline lives in a small client island (`headline-client.tsx`)
// that runs `useSearchParams()` at hydrate — see the import below.
// The static shell pre-renders at build time; the `?from=` swap
// happens after the bundle loads.

import { Suspense } from 'react';
import Link from 'next/link';

import { UpgradeHeadline } from './headline-client';

export default function BillingUpgradePage() {
  return (
    <main className="mx-auto flex max-w-2xl flex-col gap-6 p-8">
      {/* useSearchParams() requires a Suspense boundary — see Next.js
          docs on "Missing Suspense boundary with useSearchParams". */}
      <Suspense fallback={<HeadlineFallback />}>
        <UpgradeHeadline />
      </Suspense>

      <section className="rounded-lg border border-fd-border p-6">
        <h2 className="mb-3 text-lg font-medium">How to complete the upgrade</h2>
        <ol className="list-decimal space-y-2 pl-5 text-sm">
          <li>
            Open the <Link className="underline" href="/">ministr desktop app</Link> (or
            install it from the home page).
          </li>
          <li>
            Open <span className="font-mono">Settings → Cloud</span> and sign in with
            GitHub.
          </li>
          <li>
            Click <span className="font-mono">Upgrade plan</span> on the cloud panel.
            We mint a Stripe Checkout session bound to your account and open it in
            your browser.
          </li>
          <li>
            Complete payment with a real card (or the test card{' '}
            <span className="font-mono">4242 4242 4242 4242</span> on{' '}
            <span className="font-mono">sk_test_…</span> deployments).
          </li>
          <li>
            Stripe&rsquo;s webhook fires{' '}
            <span className="font-mono">customer.subscription.updated</span> against
            our cloud; your plan flips on the next API call.
          </li>
        </ol>
      </section>

      <section className="text-xs text-fd-muted-foreground">
        Already paying? Manage invoices, swap cards, or cancel at{' '}
        <Link className="underline" href="/billing/manage">
          /billing/manage
        </Link>
        .
      </section>
    </main>
  );
}

/**
 * Server-rendered fallback shown while the client island hydrates.
 * Defaults to the Pro copy so first paint isn't blank.
 */
function HeadlineFallback() {
  return (
    <header className="flex flex-col gap-2">
      <h1 className="text-3xl font-semibold">Upgrade to Pro</h1>
      <p className="text-fd-muted-foreground">ministr Cloud — $20 / month.</p>
    </header>
  );
}

export const metadata = {
  title: 'Upgrade — ministr',
  description:
    'Upgrade your ministr Cloud subscription. Stripe-hosted Checkout with full PCI offloading.',
};
