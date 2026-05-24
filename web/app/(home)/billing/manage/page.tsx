// F2.4 — Customer Portal entry point.
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
    <main className="mx-auto flex max-w-2xl flex-col gap-6 p-8">
      <header className="flex flex-col gap-2">
        <h1 className="text-3xl font-semibold">Manage billing</h1>
        <p className="text-fd-muted-foreground">
          Invoices, card on file, plan changes, and cancellation all run
          through Stripe&rsquo;s Customer Portal.
        </p>
      </header>

      <section className="rounded-lg border border-fd-border p-6">
        <h2 className="mb-3 text-lg font-medium">Open the portal</h2>
        <ol className="list-decimal space-y-2 pl-5 text-sm">
          <li>
            Open the <Link className="underline" href="/">ministr desktop app</Link>.
          </li>
          <li>
            Open <span className="font-mono">Settings → Cloud</span>.
          </li>
          <li>
            Click <span className="font-mono">Manage billing</span>. We mint a
            short-lived Stripe Customer Portal session bound to your account and
            open it in your browser.
          </li>
        </ol>
      </section>

      <section className="text-xs text-fd-muted-foreground">
        Need to start a new subscription instead?{' '}
        <Link className="underline" href="/billing/upgrade?from=pro">
          /billing/upgrade
        </Link>
        .
      </section>
    </main>
  );
}

export const metadata = {
  title: 'Manage billing — ministr',
  description:
    'Open the Stripe Customer Portal to manage your ministr Cloud subscription.',
};
