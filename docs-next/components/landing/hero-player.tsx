'use client';

import dynamic from 'next/dynamic';

// asciinema-player's ESM entry touches DOM globals at module-eval time,
// so it MUST be loaded ssr:false even though LaunchPlayer itself is
// `'use client'`. See components/landing/launch-player.tsx for the
// cleanup / useEffect wiring.
const LaunchPlayer = dynamic(
  () => import('@/components/landing/launch-player'),
  {
    ssr: false,
    loading: () => <HeroPlayerSkeleton />,
  },
);

/**
 * HeroPlayer — the hero's right-column terminal surface.
 *
 * Visually mirrors the `SessionTrace` shell: traffic-light dots,
 * ministr-tinted dark surface, ministr-accent glow shadow. The
 * asciinema-player's own chrome (border-radius on `.ap-player`, 0.75em
 * terminal inner border) is zeroed out by the overrides in
 * `app/global.css` so only our Shell renders around the canvas.
 */
export function HeroPlayer() {
  const base = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
  const src = `${base}/launch.cast`;

  return (
    <div
      className={[
        'hero-player relative rounded-xl border border-fd-border overflow-hidden',
        'font-mono text-[12.5px] leading-relaxed',
        'bg-[color-mix(in_srgb,var(--fd-card)_92%,var(--color-ministr-950)_8%)]',
        'shadow-[0_20px_60px_-30px_color-mix(in_srgb,var(--color-ministr-500)_30%,transparent)]',
      ].join(' ')}
    >
      <div className="flex items-center gap-2 border-b border-fd-border/70 px-3 py-2">
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-close)]" aria-hidden />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-min)]" aria-hidden />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-max)]" aria-hidden />
        <span className="ml-2 text-[11px] text-fd-muted-foreground">
          ministr — live recording
        </span>
        <span className="ml-auto inline-flex items-center gap-1.5 text-[10px] font-mono tracking-wider text-[var(--color-ministr-400)]">
          <span className="size-1.5 rounded-full bg-[var(--color-ministr-400)] shadow-[0_0_6px_var(--color-ministr-400)]" aria-hidden />
          REC
        </span>
      </div>
      <div className="hero-player-stage">
        <LaunchPlayer src={src} />
      </div>
    </div>
  );
}

function HeroPlayerSkeleton() {
  // Matches .hero-player-stage aspect-ratio exactly so there is no
  // reflow when the dynamically-imported player swaps in.
  return (
    <div
      className="aspect-[4/5] w-full animate-pulse bg-[color-mix(in_srgb,var(--fd-card)_80%,var(--color-ministr-950)_20%)]"
      aria-hidden
    />
  );
}
