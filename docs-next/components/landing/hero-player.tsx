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
 * HeroPlayer — asciinema recording in a minimal labelled frame.
 *
 * Earlier version wore a faux-terminal chrome: traffic-light dots +
 * a blinking "REC" badge + four decorative accent-dotted caption rows
 * under the player. All of that read as mock-terminal cosplay — the
 * player already is a terminal. Stripped to a single caption line so
 * the recording speaks for itself.
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
        ].join(' ')}
      >
        <div className="border-b border-fd-border/70 px-3 py-2">
          <span className="text-[11px] text-fd-muted-foreground">
            Claude Code + ministr — 72 s, 3 tool calls, 100% local
          </span>
        </div>
        <div className="hero-player-stage">
          {/* autoPlayOnVisible respects prefers-reduced-motion inside
              LaunchPlayer. poster `npt:1:05` lands near Claude's
              response frame; markers give a scrubbable timeline. */}
          <LaunchPlayer
            src={src}
            poster="npt:1:05"
            autoPlayOnVisible
            options={{ markers: HERO_MARKERS }}
          />
        </div>
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
