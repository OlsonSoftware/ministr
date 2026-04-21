'use client';

import { useEffect, useRef, useState } from 'react';

// Terminal-session replay for the hero.
//
// Gated behind a client-only mount because Safari's hydrator refuses to
// re-enter a subtree whose content is mutating from the first tick —
// the animator never gets a chance to run. SSR + first client paint
// both render the same inert shell; the animator mounts fresh on effect.

type Tag = 'prefetch' | 'cache-hit' | 'pressure' | 'evict';

type Line =
  | { kind: 'prompt'; text: string }
  | { kind: 'meta'; text: string; accent?: 'info' | 'success' | 'warning' | 'muted' }
  | { kind: 'result'; label: string; value: string; score?: string };

type Step = {
  lines: Line[];
  budget: number;
  tag?: Tag;
  /** Extra pause after this step's output finishes, before advancing. */
  pauseAfter: number;
};

const SCRIPT: Step[] = [
  {
    lines: [
      { kind: 'prompt', text: 'iris_survey("authentication middleware")' },
      { kind: 'meta', text: 'ranked 5 results · 42 ms', accent: 'muted' },
      { kind: 'result', label: 'src/auth.rs#login', value: 'Validates JWT, calls validate_token', score: '0.91' },
      { kind: 'result', label: 'src/auth.rs#logout', value: 'Revokes cookie, blacklists refresh', score: '0.87' },
      { kind: 'meta', text: 'prefetch: warming src/auth.rs#logout · validate_token', accent: 'info' },
    ],
    budget: 3,
    tag: 'prefetch',
    pauseAfter: 900,
  },
  {
    lines: [
      { kind: 'prompt', text: 'iris_read("src/auth.rs#login")' },
      { kind: 'meta', text: '420 tokens · section delivered', accent: 'muted' },
    ],
    budget: 5,
    pauseAfter: 700,
  },
  {
    lines: [
      { kind: 'prompt', text: 'iris_read("src/auth.rs#logout")' },
      { kind: 'meta', text: 'CACHE HIT — served from prefetch · 0 ms', accent: 'success' },
    ],
    budget: 7,
    tag: 'cache-hit',
    pauseAfter: 1100,
  },
  {
    lines: [
      { kind: 'prompt', text: 'iris_symbols(kind="function", query="validate")' },
      { kind: 'meta', text: '8 symbols found · top match validate_token', accent: 'muted' },
    ],
    budget: 9,
    pauseAfter: 700,
  },
  {
    lines: [{ kind: 'meta', text: '… many reads later …', accent: 'muted' }],
    budget: 62,
    pauseAfter: 550,
  },
  {
    lines: [
      { kind: 'prompt', text: 'iris_survey("rate limiting")' },
      { kind: 'meta', text: 'pressure: ELEVATED · results at CLAIM resolution', accent: 'warning' },
      { kind: 'meta', text: 'eviction_recommendations: [src/setup.rs#prerequisites, docs/intro.md]', accent: 'muted' },
    ],
    budget: 84,
    tag: 'pressure',
    pauseAfter: 1100,
  },
  {
    lines: [
      { kind: 'prompt', text: 'iris_evicted(["src/setup.rs#prerequisites"])' },
      { kind: 'meta', text: 'session shadow updated · freed 6% of budget', accent: 'success' },
    ],
    budget: 78,
    tag: 'evict',
    pauseAfter: 2400,
  },
];

type TypedPromptState = {
  step: number;
  chars: number;
  done: boolean;
};

export function SessionTrace() {
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  if (!mounted) {
    return (
      <div className="w-full">
        <Shell>
          <div className="h-[248px]" />
        </Shell>
      </div>
    );
  }

  return <TraceAnimator />;
}

function TraceAnimator() {
  const [windowStart, setWindowStart] = useState(0);
  const [committed, setCommitted] = useState(0);
  const [typing, setTyping] = useState<TypedPromptState>({
    step: 0,
    chars: 0,
    done: false,
  });
  const [displayBudget, setDisplayBudget] = useState(SCRIPT[0].budget);
  const [pulseTag, setPulseTag] = useState<Tag | null>(null);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const currentStep = windowStart + committed;

  useEffect(() => {
    const step = SCRIPT[currentStep];
    if (!step) return;

    let cancelled = false;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const after = (ms: number, fn: () => void) => {
      const t = setTimeout(() => {
        if (!cancelled) fn();
      }, ms);
      timers.push(t);
    };

    const prompt = step.lines.find((l) => l.kind === 'prompt') as
      | Extract<Line, { kind: 'prompt' }>
      | undefined;

    animateBudget(displayBudget, step.budget, (v, done) => {
      if (cancelled) return;
      setDisplayBudget(v);
      if (done && step.tag) setPulseTag(step.tag);
    }, after);

    after(40, () => setPulseTag(null));
    setTyping({ step: currentStep, chars: 0, done: !prompt });

    if (prompt) {
      let i = 0;
      const typeChar = () => {
        if (cancelled) return;
        if (i <= prompt.text.length) {
          setTyping({ step: currentStep, chars: i, done: false });
          i += 1;
          after(14 + Math.random() * 24, typeChar);
        } else {
          setTyping({ step: currentStep, chars: prompt.text.length, done: true });
          if (step.tag) after(80, () => setPulseTag(step.tag!));
          after(step.pauseAfter, advance);
        }
      };
      typeChar();
    } else {
      after(step.pauseAfter, advance);
    }

    function advance() {
      if (cancelled) return;
      const next = currentStep + 1;
      if (next < SCRIPT.length) {
        setCommitted((c) => c + 1);
        // Keep up to ~4 steps in view; trim older ones to the top of the window.
        if (next - windowStart >= 4) {
          setWindowStart((w) => w + 1);
        }
      } else {
        // Loop: brief hold, then fade back to step 0.
        after(1200, () => {
          if (cancelled) return;
          setCommitted(0);
          setWindowStart(0);
          setDisplayBudget(SCRIPT[0].budget);
          setTyping({ step: 0, chars: 0, done: false });
        });
      }
    }

    return () => {
      cancelled = true;
      for (const t of timers) clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentStep]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [committed, typing.chars, typing.done]);

  const budgetPct = Math.min(100, Math.round(displayBudget));
  const pressureHot = budgetPct > 80;

  return (
    <div className="w-full">
      <Shell budget={budgetPct} pressureHot={pressureHot} pulseTag={pulseTag}>
        <div
          ref={scrollRef}
          className="overflow-y-auto scroll-smooth px-4 py-3 h-[248px] text-left [&::-webkit-scrollbar]:hidden"
          style={{
            maskImage:
              'linear-gradient(to bottom, transparent, #000 14px, #000 calc(100% - 14px), transparent)',
            WebkitMaskImage:
              'linear-gradient(to bottom, transparent, #000 14px, #000 calc(100% - 14px), transparent)',
            scrollbarWidth: 'none',
          }}
        >
          {Array.from({ length: committed }, (_, k) => {
            const idx = windowStart + k;
            const s = SCRIPT[idx];
            if (!s) return null;
            return <CommittedStep key={idx} step={s} />;
          })}
          <LiveStep
            key={`live-${currentStep}`}
            step={SCRIPT[currentStep]}
            typing={typing}
          />
        </div>
      </Shell>
      <p className="mt-3 text-[11px] text-fd-muted-foreground/80">
        Illustrative session replay — numbers are indicative, not measured.
      </p>
    </div>
  );
}

function Shell({
  children,
  budget,
  pressureHot,
  pulseTag,
}: {
  children: React.ReactNode;
  budget?: number;
  pressureHot?: boolean;
  pulseTag?: Tag | null;
}) {
  const accentClass =
    pulseTag === 'pressure' || pressureHot
      ? 'border-[color-mix(in_srgb,var(--color-warning)_45%,var(--fd-border))] shadow-[0_0_0_1px_color-mix(in_srgb,var(--color-warning)_25%,transparent),0_20px_60px_-20px_color-mix(in_srgb,var(--color-warning)_30%,transparent)]'
      : pulseTag === 'cache-hit'
      ? 'border-[color-mix(in_srgb,var(--color-success)_45%,var(--fd-border))] shadow-[0_0_0_1px_color-mix(in_srgb,var(--color-success)_25%,transparent),0_20px_60px_-20px_color-mix(in_srgb,var(--color-success)_30%,transparent)]'
      : 'border-fd-border shadow-[0_20px_60px_-30px_color-mix(in_srgb,var(--color-iris-500)_30%,transparent)]';

  return (
    <div
      className={
        'relative rounded-xl border font-mono text-[12.5px] leading-relaxed transition-[border-color,box-shadow] duration-500 ' +
        accentClass +
        ' bg-[color-mix(in_srgb,var(--fd-card)_92%,var(--color-iris-950)_8%)]'
      }
    >
      <div className="flex items-center gap-2 border-b border-fd-border/70 px-3 py-2">
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-close)]" />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-min)]" />
        <span className="size-2.5 rounded-full bg-[var(--color-traffic-max)]" />
        <span className="ml-2 text-[11px] text-fd-muted-foreground">iris — session (example)</span>
        {typeof budget === 'number' && (
          <span className="ml-auto flex items-center gap-2 text-[10px] font-mono text-fd-muted-foreground">
            <span className="uppercase tracking-wider">budget</span>
            <span
              className="relative inline-block h-1.5 w-28 overflow-hidden rounded-full bg-fd-border/60"
              aria-label="budget utilization"
            >
              <span
                className={
                  'absolute inset-y-0 left-0 rounded-full transition-[width,background-color] duration-500 ease-out ' +
                  (pressureHot
                    ? 'bg-[var(--color-warning)]'
                    : budget > 60
                    ? 'bg-gradient-to-r from-[var(--color-iris-500)] to-[var(--color-warning)]'
                    : 'bg-[var(--color-iris-500)]')
                }
                style={{ width: `${budget}%` }}
              />
              {pulseTag === 'evict' && (
                <span className="absolute inset-0 rounded-full animate-[trace-flash_0.7s_ease-out] bg-[color-mix(in_srgb,var(--color-success)_50%,transparent)]" />
              )}
            </span>
            <span className="tabular-nums w-[3ch] text-right">{budget}%</span>
          </span>
        )}
      </div>
      {children}
    </div>
  );
}

function CommittedStep({ step }: { step: Step }) {
  return (
    <div className="mb-3 last:mb-0">
      {step.lines.map((line, i) => (
        <LineRow key={i} line={line} />
      ))}
    </div>
  );
}

function LiveStep({
  step,
  typing,
}: {
  step: Step | undefined;
  typing: TypedPromptState;
}) {
  if (!step) return null;
  return (
    <div className="mb-3 last:mb-0">
      {step.lines.map((line, i) => {
        if (line.kind === 'prompt') {
          return (
            <PromptRow
              key={i}
              text={line.text}
              typedChars={typing.chars}
              active={!typing.done}
            />
          );
        }
        if (!typing.done) return null;
        return <LineRow key={i} line={line} fadeIn index={i} />;
      })}
    </div>
  );
}

function PromptRow({
  text,
  typedChars,
  active,
}: {
  text: string;
  typedChars: number;
  active: boolean;
}) {
  const typed = text.slice(0, typedChars);
  return (
    <div className="flex items-start gap-2 text-fd-foreground">
      <span className="select-none text-[var(--color-iris-500)] font-semibold">➜</span>
      <span className="break-all whitespace-pre-wrap">{typed}</span>
      {active && typedChars < text.length && (
        <span
          aria-hidden
          className="ml-0.5 inline-block h-[1.1em] w-[0.55em] translate-y-[0.12em] bg-[var(--color-iris-500)]/80 animate-[trace-cursor_0.9s_steps(2,end)_infinite]"
        />
      )}
    </div>
  );
}

function LineRow({
  line,
  fadeIn,
  index = 0,
}: {
  line: Line;
  fadeIn?: boolean;
  index?: number;
}) {
  if (line.kind === 'prompt') {
    return (
      <div className="flex items-start gap-2 text-fd-foreground">
        <span className="select-none text-[var(--color-iris-500)] font-semibold">➜</span>
        <span className="break-all whitespace-pre-wrap">{line.text}</span>
      </div>
    );
  }

  const fadeStyle = fadeIn
    ? { animationDelay: `${index * 70}ms` }
    : undefined;
  const fadeCls = fadeIn ? ' opacity-0 animate-[trace-fade_280ms_ease-out_forwards]' : '';

  if (line.kind === 'meta') {
    const accentCls =
      line.accent === 'success'
        ? 'text-[var(--color-success)]'
        : line.accent === 'warning'
        ? 'text-[var(--color-warning)]'
        : line.accent === 'info'
        ? 'text-[var(--color-iris-400)]'
        : 'text-fd-muted-foreground';
    return (
      <div
        className={'flex items-start gap-2 pl-4 ' + accentCls + fadeCls}
        style={fadeStyle}
      >
        <span className="select-none text-fd-muted-foreground/40">└</span>
        <span className="break-all whitespace-pre-wrap">{line.text}</span>
      </div>
    );
  }

  // result
  return (
    <div
      className={'flex items-start gap-2 pl-4 ' + fadeCls}
      style={fadeStyle}
    >
      <span className="select-none text-fd-muted-foreground/40">└</span>
      <span className="text-[var(--color-iris-400)] shrink-0">{line.label}</span>
      <span className="text-fd-muted-foreground truncate flex-1">{line.value}</span>
      {line.score && (
        <span className="shrink-0 font-mono text-[10px] text-[var(--color-iris-500)] tabular-nums">
          {line.score}
        </span>
      )}
    </div>
  );
}

function animateBudget(
  from: number,
  to: number,
  onTick: (value: number, done: boolean) => void,
  after: (ms: number, fn: () => void) => void,
) {
  const steps = 14;
  const duration = 500;
  const delta = (to - from) / steps;
  for (let i = 1; i <= steps; i++) {
    after((i / steps) * duration, () => {
      const v = from + delta * i;
      onTick(v, i === steps);
    });
  }
}
