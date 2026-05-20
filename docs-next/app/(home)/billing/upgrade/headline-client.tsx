// F2.4 — client island for the `/billing/upgrade` page headline.
//
// We can't read `?from=<plan>` server-side: the site uses
// `output: 'export'` (static export) which forbids per-request
// rendering. The whole page must pre-render at build time. To still
// honour the query param without breaking static export, we render a
// generic shell server-side and let a tiny client island swap the
// headline at hydrate via `useSearchParams()`.
//
// `useSearchParams()` REQUIRES a `<Suspense>` boundary around it
// (Next.js fails the build otherwise) — the parent page provides one.

'use client';

import { useSearchParams } from 'next/navigation';

const PLAN_HEADLINE: Record<string, { title: string; price: string }> = {
  pro: { title: 'Upgrade to Pro', price: '$20 / month' },
  team: { title: 'Upgrade to Team', price: '$30 / seat / month (3-seat min)' },
  enterprise: { title: 'Enterprise — contact sales', price: 'Custom' },
};

export function UpgradeHeadline() {
  const params = useSearchParams();
  const from = (params.get('from') ?? '').toLowerCase();
  const headline = PLAN_HEADLINE[from] ?? PLAN_HEADLINE.pro;
  return (
    <header className="flex flex-col gap-2">
      <h1 className="text-3xl font-semibold">{headline.title}</h1>
      <p className="text-fd-muted-foreground">
        ministr Cloud — {headline.price}.
      </p>
    </header>
  );
}
