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
 * Timeline markers for the hero's cast — handpicked from the real
 * recording's event stream (see `scripts/demo-record-cast.sh`'s
 * timing analysis). Surface as scrubbable dots on the player's
 * timeline so a viewer can jump to each beat of the narrative.
 */
const HERO_MARKERS: Array<[number, string]> = [
  [17, 'Claude Code launches'],
  [36, 'Prompt submitted'],
  [42, 'ministr tools fire'],
  [56, 'Response'],
];

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
    <div className="relative">
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
          {/*
           * autoPlayOnVisible = engaging first impression; respects
           * prefers-reduced-motion inside LaunchPlayer.
           *
           * poster `npt:1:05` lands on a frame near the end where Claude
           * has responded — much more compelling than the default blank
           * cursor at t=0.
           *
           * markers give the player a scrubbable timeline with the four
           * narrative beats labelled.
           */}
          <LaunchPlayer
            src={src}
            poster="npt:1:05"
            autoPlayOnVisible
            options={{ markers: HERO_MARKERS }}
          />
        </div>
      </div>

      {/*
       * Stats strip underneath. Mirrors the existing `ministr-body-quiet`
       * caption style used on other landing sections (e.g. SessionTrace's
       * "Illustrative session replay"). Specific numbers come from the
       * recording: duration from the cast header, tool-call count from
       * Claude's own "Called ministr N times" line, token count
       * approximate for a 4-tool trace.
       */}
      <div className="mt-3 flex flex-wrap items-center gap-x-5 gap-y-1 text-[11px] text-fd-muted-foreground/90">
        <span className="inline-flex items-center gap-1.5">
          <span className="size-1 rounded-full bg-[var(--color-ministr-400)]" aria-hidden />
          72 s
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="size-1 rounded-full bg-[var(--color-ministr-400)]" aria-hidden />
          3 ministr tool calls
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="size-1 rounded-full bg-[var(--color-ministr-400)]" aria-hidden />
          Python package · greet tracing
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="size-1 rounded-full bg-[var(--color-ministr-400)]" aria-hidden />
          Fully local
        </span>
      </div>
    </div>
  );
}

function HeroPlayerSkeleton() {
  // Matches .hero-player-stage aspect-ratio exactly so there is no
  // reflow when the dynamically-imported player swaps in.
  return (
    <div
      className="aspect-[3/2] w-full animate-pulse bg-[color-mix(in_srgb,var(--fd-card)_80%,var(--color-ministr-950)_20%)]"
      aria-hidden
    />
  );
}
