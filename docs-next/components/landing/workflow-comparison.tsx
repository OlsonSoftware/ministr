'use client';

import dynamic from 'next/dynamic';
import { Reveal } from '@/components/landing/reveal';

const LaunchPlayer = dynamic(
  () => import('@/components/landing/launch-player'),
  {
    ssr: false,
    loading: () => <PanelSkeleton />,
  },
);

interface PanelProps {
  /** URL to the .cast file (asset-prefixed by the caller). */
  src: string;
  /** Top-left header text — e.g. "without ministr" or "with ministr". */
  title: string;
  /** Right-side badge tint: "warn" = red-ish, "accent" = ministr-ish. */
  tone: 'warn' | 'accent';
  /** Short 3-line stats underneath: tool count, tokens, signal %. */
  stats: { label: string; value: string; emphasis?: boolean }[];
  /** Poster npt:<mm>:<ss> or undefined. */
  poster?: string;
  /** Autoplay when it scrolls into view. */
  autoPlay?: boolean;
}

function Panel({ src, title, tone, stats, poster, autoPlay }: PanelProps) {
  const toneClasses =
    tone === 'warn'
      ? 'text-[var(--color-warning)]'
      : 'text-[var(--color-ministr-400)]';
  const toneDotGlow =
    tone === 'warn'
      ? 'bg-[var(--color-warning)] shadow-[0_0_6px_var(--color-warning)]'
      : 'bg-[var(--color-ministr-400)] shadow-[0_0_6px_var(--color-ministr-400)]';

  return (
    <div
      className={[
        'relative rounded-xl border border-fd-border overflow-hidden',
        'font-mono text-[12.5px] leading-relaxed',
        'bg-[color-mix(in_srgb,var(--fd-card)_92%,var(--color-ministr-950)_8%)]',
        'shadow-[0_20px_60px_-30px_color-mix(in_srgb,var(--color-ministr-500)_30%,transparent)]',
      ].join(' ')}
    >
      <div className="flex items-center gap-2 border-b border-fd-border/70 px-3 py-2">
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-close)]" aria-hidden />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-min)]" aria-hidden />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-max)]" aria-hidden />
        <span className="ml-2 text-[11px] text-fd-muted-foreground truncate">
          {title}
        </span>
        <span className={`ml-auto inline-flex items-center gap-1.5 text-[10px] font-mono tracking-wider ${toneClasses}`}>
          <span className={`size-1.5 rounded-full ${toneDotGlow}`} aria-hidden />
          REC
        </span>
      </div>
      <div className="hero-player-stage">
        <LaunchPlayer
          src={src}
          poster={poster}
          autoPlayOnVisible={autoPlay}
        />
      </div>
      <dl className="grid grid-cols-3 divide-x divide-fd-border/60 border-t border-fd-border/70 text-center">
        {stats.map((stat) => (
          <div key={stat.label} className="px-3 py-2">
            <dt className="text-[9.5px] uppercase tracking-[0.08em] text-fd-muted-foreground/80">
              {stat.label}
            </dt>
            <dd
              className={[
                'mt-0.5 font-mono text-[12.5px] tabular-nums',
                stat.emphasis ? toneClasses : 'text-fd-foreground',
              ].join(' ')}
            >
              {stat.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

function PanelSkeleton() {
  return (
    <div
      className="aspect-[3/2] w-full animate-pulse bg-[color-mix(in_srgb,var(--fd-card)_80%,var(--color-ministr-950)_20%)]"
      aria-hidden
    />
  );
}

/**
 * WorkflowComparison — real asciinema casts of the exact same task,
 * run two ways: Claude Code's built-in Grep/Read tools vs Claude Code
 * with ministr. Same prompt, same codebase, different tool loadout.
 *
 * If the baseline cast (launch-baseline.cast) hasn't been recorded
 * yet, this component renders only the ministr side so the page
 * doesn't 404 or show a broken placeholder.
 *
 * Both baseline recording and stat values are pending; update the
 * constants below with real numbers once assets/launch-baseline.cast
 * lands. See scripts/demo-record-baseline-cast.sh.
 */
export function WorkflowComparison({ hasBaseline = false }: { hasBaseline?: boolean }) {
  const base = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
  const ministrSrc = `${base}/launch.cast`;
  const baselineSrc = `${base}/launch-baseline.cast`;

  return (
    <div className="mx-auto w-full max-w-6xl px-4 sm:px-6">
      <Reveal>
        <p className="ministr-eyebrow">Same task · two paths</p>
      </Reveal>
      <Reveal delay={0.08}>
        <h2 className="mt-5 text-[clamp(2rem,4.2vw,3.25rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
          Real Claude Code sessions, side by side.
        </h2>
      </Reveal>
      <Reveal delay={0.16}>
        <p className="ministr-body mt-4 max-w-[66ch] text-[15.5px]">
          One prompt — &ldquo;trace how <code className="font-mono text-[0.95em]">greet</code>{' '}
          is wired up&rdquo; — recorded twice: once with only Claude Code&rsquo;s
          built-in Glob/Grep/Read tools, once with ministr as an MCP
          server. No mocks, no edits. The contrast is what ministr is
          for.
        </p>
      </Reveal>

      <Reveal delay={0.24}>
        <div
          className={[
            'mt-10 grid gap-6',
            hasBaseline ? 'lg:grid-cols-2' : 'mx-auto max-w-3xl lg:grid-cols-1',
          ].join(' ')}
        >
          {hasBaseline && (
            <Panel
              src={baselineSrc}
              title="without ministr · grep + read"
              tone="warn"
              stats={[
                { label: 'tool calls', value: '—', emphasis: true },
                { label: 'tokens', value: '—', emphasis: true },
                { label: 'signal', value: '—', emphasis: true },
              ]}
              // Autoplay the baseline; the ministr side autoplays in the hero
              // already, so we don't want two simultaneous soundless movies
              // on first scroll.
              autoPlay
            />
          )}
          <Panel
            src={ministrSrc}
            title="with ministr · semantic index"
            tone="accent"
            stats={[
              { label: 'tool calls', value: '3', emphasis: true },
              { label: 'tokens', value: '~500', emphasis: true },
              { label: 'signal', value: '100%', emphasis: true },
            ]}
            poster="npt:1:05"
            autoPlay={!hasBaseline}
          />
        </div>
      </Reveal>

      {!hasBaseline && (
        <Reveal delay={0.32}>
          <p className="mt-6 text-[12px] text-fd-muted-foreground/80">
            (Baseline recording pending —{' '}
            <code className="font-mono">scripts/demo-record-baseline-cast.sh</code>.)
          </p>
        </Reveal>
      )}
    </div>
  );
}
