'use client';

import { useEffect, useRef, useState } from 'react';

type Tag = 'prefetch' | 'cache-hit' | 'pressure' | 'evict' | 'ellipsis' | undefined;

const SCRIPT: Array<{ line: string; meta: string | null; budget: number; tag?: Tag; pause: number }> = [
  {
    line: 'iris_survey("authentication middleware")',
    meta: 'ranked 5 results · prefetch: warming src/auth.rs#logout',
    budget: 3,
    pause: 900,
    tag: 'prefetch',
  },
  {
    line: 'iris_read("src/auth.rs#login")',
    meta: '420 tokens · prefetch: warming validate_token',
    budget: 5,
    pause: 900,
    tag: 'prefetch',
  },
  {
    line: 'iris_read("src/auth.rs#logout")',
    meta: 'CACHE HIT — delivered from prefetch · 0 ms',
    budget: 7,
    pause: 1000,
    tag: 'cache-hit',
  },
  {
    line: 'iris_symbols(kind="function", query="validate")',
    meta: '8 symbols found',
    budget: 8,
    pause: 900,
  },
  {
    line: '… many reads later …',
    meta: null,
    budget: 60,
    pause: 700,
    tag: 'ellipsis',
  },
  {
    line: 'iris_survey("rate limiting")',
    meta: 'pressure: ELEVATED · eviction_recommendations: [src/setup.rs#prerequisites]',
    budget: 82,
    pause: 1100,
    tag: 'pressure',
  },
  {
    line: 'iris_evicted(["src/setup.rs#prerequisites"])',
    meta: 'session shadow updated',
    budget: 76,
    pause: 1400,
    tag: 'evict',
  },
];

export function SessionTrace() {
  const [step, setStep] = useState(0);
  const [typedLine, setTypedLine] = useState('');
  const [showMeta, setShowMeta] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    setReducedMotion(mq.matches);
  }, []);

  useEffect(() => {
    if (reducedMotion) {
      // Skip animation: show final state
      setStep(SCRIPT.length - 1);
      setTypedLine(SCRIPT[SCRIPT.length - 1].line);
      setShowMeta(true);
      return;
    }

    const current = SCRIPT[step];
    setTypedLine('');
    setShowMeta(false);

    let i = 0;
    const type = () => {
      if (i <= current.line.length) {
        setTypedLine(current.line.slice(0, i));
        i += 1;
        timeoutRef.current = setTimeout(type, 22 + Math.random() * 30);
      } else {
        // done typing → brief pause → show meta
        timeoutRef.current = setTimeout(() => setShowMeta(true), 280);
      }
    };
    type();

    // After the meta + pause, advance (or loop)
    const advanceMs = current.line.length * 28 + 280 + current.pause;
    const advance = setTimeout(() => {
      setStep((prev) => (prev + 1) % SCRIPT.length);
    }, advanceMs);

    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      clearTimeout(advance);
    };
  }, [step, reducedMotion]);

  const current = SCRIPT[step];
  const budgetPct = Math.min(100, current.budget);

  return (
    <div className="mx-auto mt-8 w-full max-w-2xl">
      <div className="rounded-xl border border-fd-border bg-fd-card font-mono text-[12.5px] leading-relaxed shadow-sm">
        {/* Chrome */}
        <div className="flex items-center gap-2 border-b border-fd-border px-3 py-2">
          <span className="size-2.5 rounded-full bg-[var(--color-traffic-close)]" />
          <span className="size-2.5 rounded-full bg-[var(--color-traffic-min)]" />
          <span className="size-2.5 rounded-full bg-[var(--color-traffic-max)]" />
          <span className="ml-2 text-[11px] text-fd-muted-foreground">iris session</span>
          <span className="ml-auto flex items-center gap-1.5 text-[10px] text-fd-muted-foreground">
            <span>budget</span>
            <span
              className="inline-block h-1.5 w-16 overflow-hidden rounded-full bg-fd-border"
              aria-label="budget utilization"
            >
              <span
                className={
                  'block h-full transition-all duration-500 ' +
                  (budgetPct > 80
                    ? 'bg-[var(--color-warning)]'
                    : 'bg-[var(--color-iris-500)]')
                }
                style={{ width: `${budgetPct}%` }}
              />
            </span>
            <span className="tabular-nums">{budgetPct}%</span>
          </span>
        </div>

        {/* Body */}
        <div className="px-4 py-3 text-left min-h-[88px]">
          <div className="flex items-start gap-2">
            <span className="select-none text-[var(--color-iris-500)]">➜</span>
            <span className="break-all">{typedLine}</span>
            {typedLine.length < current.line.length && !reducedMotion && (
              <span className="ml-0.5 inline-block h-4 w-2 animate-pulse bg-fd-muted-foreground/70" />
            )}
          </div>
          {current.meta && showMeta && (
            <div className="mt-1 pl-4 text-fd-muted-foreground flex items-center gap-2">
              <span className="break-all">{current.meta}</span>
              {current.tag && <TagBadge tag={current.tag} />}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function TagBadge({ tag }: { tag: Tag }) {
  const styles: Record<NonNullable<Tag>, string> = {
    prefetch: 'bg-[color-mix(in_srgb,var(--color-iris-500)_18%,transparent)] text-[var(--color-iris-500)]',
    'cache-hit': 'bg-[color-mix(in_srgb,var(--color-success)_20%,transparent)] text-[var(--color-success)]',
    pressure: 'bg-[color-mix(in_srgb,var(--color-warning)_20%,transparent)] text-[var(--color-warning)]',
    evict: 'bg-fd-muted text-fd-muted-foreground',
    ellipsis: '',
  };
  if (!tag || tag === 'ellipsis') return null;
  const labels: Record<NonNullable<Tag>, string> = {
    prefetch: 'prefetch',
    'cache-hit': 'cache hit',
    pressure: 'pressure',
    evict: 'evict',
    ellipsis: '',
  };
  return (
    <span
      className={
        'shrink-0 rounded-full px-2 py-0.5 text-[9px] font-semibold uppercase tracking-wider ' +
        styles[tag]
      }
    >
      {labels[tag]}
    </span>
  );
}
