'use client';

import dynamic from 'next/dynamic';
import { Reveal } from '@/components/landing/reveal';

// Load the player client-side only. asciinema-player's ESM entry touches
// `window`/`document` at module-evaluation time, which would crash Next's
// static-export prerender even though LaunchPlayer is `'use client'`.
const LaunchPlayer = dynamic(
  () => import('@/components/landing/launch-player'),
  {
    ssr: false,
    loading: () => <LaunchPlayerSkeleton />,
  },
);

/**
 * LaunchDemo — landing-page section that embeds the real Claude Code +
 * ministr recording. Text inside the player is selectable and the
 * timeline is scrubbable — a genuine interactive upgrade over the GIF.
 */
export function LaunchDemo() {
  const base = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
  const src = `${base}/launch.cast`;

  return (
    <div className="mx-auto w-full max-w-5xl px-4 sm:px-6">
      <Reveal>
        <p className="ministr-eyebrow">Live demo</p>
      </Reveal>
      <Reveal delay={0.08}>
        <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
          See it in your terminal.
        </h2>
      </Reveal>
      <Reveal delay={0.16}>
        <p className="ministr-body mt-4 max-w-[60ch] text-[15.5px]">
          A real Claude Code session calling ministr tools on a small
          Python package. No mocks, no marketing cuts — just{' '}
          <code className="font-mono text-[0.95em]">ministr init</code>,{' '}
          <code className="font-mono text-[0.95em]">claude mcp add</code>,
          and a prompt that traces a function across the codebase. Scrub,
          pause, and copy text from any frame.
        </p>
      </Reveal>
      <Reveal delay={0.24}>
        <div className="mt-10 overflow-hidden rounded-xl border border-fd-border bg-[color-mix(in_srgb,var(--fd-card)_92%,var(--color-ministr-950)_8%)] shadow-[0_20px_60px_-30px_color-mix(in_srgb,var(--color-ministr-500)_30%,transparent)]">
          <LaunchPlayer src={src} />
        </div>
      </Reveal>
    </div>
  );
}

function LaunchPlayerSkeleton() {
  return (
    <div
      className="aspect-[16/9] w-full animate-pulse bg-[color-mix(in_srgb,var(--fd-card)_80%,var(--color-ministr-950)_20%)]"
      aria-hidden
    />
  );
}
