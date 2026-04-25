'use client';

import Link from 'next/link';
import type { ReactNode } from 'react';
import { useEffect, useRef, useState } from 'react';
import { ArrowRight, ChevronLeft, ChevronRight, Pause, Play } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { GlassCard } from '@/components/landing/glass-card';

/* ---------------------------------------------------------------
   Scripted sequence — a real ministr tool call, end to end.
   Each step activates the layers + mechanisms involved and tells
   the channels which direction data is flowing.
   --------------------------------------------------------------- */

type LayerKey = 'agent' | 'daemon' | 'index' | 'corpus';
type MechKey = 'shadow' | 'prefetch' | 'budget' | 'coherence' | 'delta' | 'search';
type Direction = 'down' | 'up' | 'idle';

type Step = {
  id: string;
  label: string;
  caption: string;
  detail: string; // longer prose shown in the left column
  activeLayers: LayerKey[];
  activeMechs: MechKey[];
  mcp: Direction;      // channel between Agent ↔ ministr daemon
  query: Direction;    // channel between ministr daemon ↔ Index
  corpus: Direction;   // channel between Index ↔ Corpus
};

const STEPS: Step[] = [
  {
    id: 'send',
    label: 'agent sends tool call',
    caption: 'Claude Code → ministr_read("src/auth.rs#login")',
    detail:
      'The agent — Claude Code, Cursor, Copilot, any MCP client — wants a section of your code. It issues a JSON-RPC call over stdio to ministr, spawned as a subprocess. No network hop. The whole conversation will stay on this one machine.',
    activeLayers: ['agent'],
    activeMechs: [],
    mcp: 'down', query: 'idle', corpus: 'idle',
  },
  {
    id: 'shadow',
    label: 'session-shadow lookup',
    caption: 'shadow: has this agent already seen this section?',
    detail:
      'Before doing any work, ministr asks Session Shadow: “has this agent already received this section in this turn?” The shadow is a per-session ledger keyed by content ID — section, claim, or symbol — with a content hash stored alongside so ministr can tell if the underlying content has changed since last delivery. If the agent already has it and it is unchanged, ministr returns a trivial pointer instead of re-serving text the agent already paid budget for.',
    activeLayers: ['daemon'],
    activeMechs: ['shadow', 'budget'],
    mcp: 'idle', query: 'idle', corpus: 'idle',
  },
  {
    id: 'query',
    label: 'hybrid search over the index',
    caption: 'miss → dense + sparse search on SQLite + HNSW',
    detail:
      'Shadow miss. ministr issues a hybrid query: dense embeddings (HNSW, ANN at millisecond latency) plus SPLADE-style sparse term matching from the SQLite index. The two lanes are fused at rank-time so keyword and meaning both matter.',
    activeLayers: ['daemon', 'index'],
    activeMechs: ['search'],
    mcp: 'idle', query: 'down', corpus: 'idle',
  },
  {
    id: 'read',
    label: 'corpus read',
    caption: 'stored section text fetched from SQLite',
    detail:
      'The index resolves the hit to a content ID. ministr fetches the pre-parsed section text from SQLite — no re-parsing at read time. Tree-sitter only ran at ingestion time to split files into sections and extract symbols. Reads are fully read-only; ministr never mutates your repo.',
    activeLayers: ['index', 'corpus'],
    activeMechs: [],
    mcp: 'idle', query: 'idle', corpus: 'down',
  },
  {
    id: 'assemble',
    label: 'delta assembly',
    caption: 'delta delivery: only the lines the agent does not have',
    detail:
      'ministr compares what it is about to send against the session shadow. Lines the agent already has get elided. The response is a delta — a diff of just the new or changed lines plus a pointer to the unchanged region. Agents stop paying for re-reads.',
    activeLayers: ['daemon', 'index'],
    activeMechs: ['delta'],
    mcp: 'idle', query: 'up', corpus: 'up',
  },
  {
    id: 'respond',
    label: 'response delivered',
    caption: '← 420 tokens · 3 changed lines · shadow updated',
    detail:
      'The delta returns to the agent over MCP. In the same atomic step, ministr writes what it just delivered into Session Shadow, so the next turn’s lookup is a hit. The agent sees the content; ministr remembers what it sent.',
    activeLayers: ['daemon', 'agent'],
    activeMechs: ['delta', 'shadow'],
    mcp: 'up', query: 'idle', corpus: 'idle',
  },
  {
    id: 'prefetch',
    label: 'predictive prefetch',
    caption: 'warming likely next reads: #logout, #refresh, #revoke',
    detail:
      'While the agent thinks, ministr uses sequential, structural, and topical heuristics to guess the next read. The next section in the document, neighbouring siblings at the same depth, topically similar sections from anywhere in the corpus — it pre-warms them into the index cache. When the agent asks next turn, that read is already hot.',
    activeLayers: ['daemon', 'index'],
    activeMechs: ['prefetch'],
    mcp: 'idle', query: 'down', corpus: 'idle',
  },
  {
    id: 'coherence',
    label: 'coherence watch',
    caption: 'files change on disk → delivered content flagged stale',
    detail:
      'A file watcher is always running. If any file referenced in the session shadow changes on disk, ministr flags the earlier delivery as stale. Next time the agent references that content, ministr ships a delta against the new version instead of silently serving stale data.',
    activeLayers: ['daemon', 'corpus'],
    activeMechs: ['coherence'],
    mcp: 'idle', query: 'idle', corpus: 'up',
  },
];

/* ---------------------------------------------------------------
   ArchitectureFlow — left column (copy) + right column (diagram)
   --------------------------------------------------------------- */

export function ArchitectureFlow() {
  const [stepIndex, setStepIndex] = useState(0);
  const [playing, setPlaying] = useState(false); // default paused, user opts in
  const [progress, setProgress] = useState(0);
  const reduced = useReducedMotion();
  const startRef = useRef<number>(0);
  const rafRef = useRef<number | null>(null);

  const step = STEPS[stepIndex];
  // Slow auto-advance: each step lingers 5s when playing
  const STEP_DURATION = 5000;

  useEffect(() => {
    if (!playing || reduced) return;
    startRef.current = performance.now();
    const tick = (now: number) => {
      const t = Math.min(1, (now - startRef.current) / STEP_DURATION);
      setProgress(t);
      if (t >= 1) {
        setStepIndex((i) => (i + 1) % STEPS.length);
        setProgress(0);
      } else {
        rafRef.current = requestAnimationFrame(tick);
      }
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, [playing, reduced, stepIndex]);

  const goPrev = () => {
    setStepIndex((i) => (i - 1 + STEPS.length) % STEPS.length);
    setProgress(0);
  };
  const goNext = () => {
    setStepIndex((i) => (i + 1) % STEPS.length);
    setProgress(0);
  };
  const jumpTo = (i: number) => {
    setStepIndex(i);
    setProgress(0);
  };

  return (
    <section className="relative py-24 sm:py-32">
      <div className="mx-auto w-full max-w-6xl px-4 sm:px-6">
        <div className="grid gap-10 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.25fr)] lg:gap-16 lg:items-start">
          <div className="lg:sticky lg:top-24">
            <p className="ministr-eyebrow">How it wires up</p>
            <h2 className="mt-4 text-[clamp(2rem,4vw,3rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
              A single local process between your agent and your files.
            </h2>

            {/* Step narration — updates per active step */}
            <div className="mt-8 min-h-[220px]">
              <div className="flex items-center gap-3">
                <span className="ministr-eyebrow-sm tabular-nums">
                  step {String(stepIndex + 1).padStart(2, '0')} / {String(STEPS.length).padStart(2, '0')}
                </span>
                <span className="h-px flex-1 bg-[color-mix(in_oklch,var(--color-ministr-400)_25%,transparent)]" />
              </div>

              <AnimatePresence mode="wait">
                <motion.div
                  key={step.id}
                  initial={reduced ? false : { opacity: 0, y: 8 }}
                  animate={reduced ? undefined : { opacity: 1, y: 0 }}
                  exit={reduced ? undefined : { opacity: 0, y: -6 }}
                  transition={{ duration: 0.35, ease: [0.2, 0.8, 0.2, 1] }}
                  className="mt-3"
                >
                  <h3 className="text-[clamp(1.35rem,2.2vw,1.75rem)] font-semibold leading-tight tracking-tight text-fd-foreground">
                    {step.label}
                  </h3>
                  <p className="mt-2 font-mono text-[12.5px] text-[var(--ministr-accent-text)]">
                    {step.caption}
                  </p>
                  <p className="ministr-body mt-4 max-w-[58ch] text-[15px] leading-relaxed">
                    {step.detail}
                  </p>
                </motion.div>
              </AnimatePresence>
            </div>

            {/* Prev / Next controls */}
            <div className="mt-6 flex items-center gap-2">
              <button
                type="button"
                onClick={goPrev}
                aria-label="Previous step"
                className="inline-flex items-center gap-1 rounded-lg border border-fd-border/60 bg-[color-mix(in_oklch,var(--ministr-surface)_55%,transparent)] px-3 py-1.5 text-[12.5px] text-fd-foreground transition hover:border-[color-mix(in_oklch,var(--color-ministr-400)_55%,transparent)] hover:bg-[color-mix(in_oklch,var(--color-ministr-500)_14%,transparent)]"
              >
                <ChevronLeft className="size-3.5" aria-hidden />
                Prev
              </button>
              <button
                type="button"
                onClick={goNext}
                aria-label="Next step"
                className="inline-flex items-center gap-1 rounded-lg border border-[color-mix(in_oklch,var(--color-ministr-400)_50%,transparent)] bg-[color-mix(in_oklch,var(--color-ministr-500)_16%,transparent)] px-3 py-1.5 text-[12.5px] font-medium text-fd-foreground transition hover:bg-[color-mix(in_oklch,var(--color-ministr-500)_26%,transparent)]"
              >
                Next
                <ChevronRight className="size-3.5" aria-hidden />
              </button>
              <button
                type="button"
                onClick={() => setPlaying((p) => !p)}
                aria-label={playing ? 'Pause autoplay' : 'Autoplay'}
                className="ml-2 inline-flex items-center gap-1 rounded-lg border border-fd-border/40 bg-[color-mix(in_oklch,var(--ministr-surface)_45%,transparent)] px-3 py-1.5 text-[11.5px] text-fd-muted-foreground transition hover:text-fd-foreground"
              >
                {playing ? (
                  <>
                    <Pause className="size-3" aria-hidden />
                    Pause auto
                  </>
                ) : (
                  <>
                    <Play className="size-3" aria-hidden />
                    Autoplay
                  </>
                )}
              </button>
              <span className="ml-auto font-mono text-[10.5px] uppercase tracking-wider text-fd-muted-foreground">
                {playing ? 'auto · 5s / step' : 'paused'}
              </span>
            </div>

            <Link
              href="/docs/architecture"
              className="mt-8 inline-flex items-center gap-1.5 text-[14px] font-medium text-[var(--ministr-accent-text)] transition hover:text-[var(--color-ministr-500)]"
            >
              Read the full architecture
              <ArrowRight className="size-4" aria-hidden />
            </Link>
          </div>

          <GlassCard padded={false} className="overflow-hidden p-5 sm:p-6">
            <FlowDiagram
              step={step}
              stepIndex={stepIndex}
              progress={progress}
              onJump={jumpTo}
            />
          </GlassCard>
        </div>
      </div>
    </section>
  );
}

/* ---------------------------------------------------------------
   FlowDiagram — receives the current step from the parent narration.
   --------------------------------------------------------------- */

function FlowDiagram({
  step,
  stepIndex,
  progress,
  onJump,
}: {
  step: Step;
  stepIndex: number;
  progress: number;
  onJump: (i: number) => void;
}) {
  const isActiveLayer = (k: LayerKey) => step.activeLayers.includes(k);
  const isActiveMech = (k: MechKey) => step.activeMechs.includes(k);

  return (
    <div className="flex flex-col font-mono text-[12px]">
      <FlowLayer
        kicker="Agent"
        meta="any MCP client"
        tone="muted"
        active={isActiveLayer('agent')}
      >
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip active={isActiveLayer('agent')}>Claude Code</Chip>
          <Chip>Cursor</Chip>
          <Chip>Copilot</Chip>
          <Chip>Continue</Chip>
          <Chip>…</Chip>
        </div>
      </FlowLayer>

      <Channel label="MCP · stdio" sub="tool call ↓   context delta ↑" direction={step.mcp} />

      <FlowLayer
        kicker="ministr"
        meta="Rust · single local process"
        tone="featured"
        active={isActiveLayer('daemon')}
      >
        <div className="grid grid-cols-2 gap-x-2 gap-y-1.5">
          <MechanismRow label="Session Shadow"      detail="tracks what's been sent"        active={isActiveMech('shadow')} />
          <MechanismRow label="Predictive Prefetch" detail="warms next likely reads"        active={isActiveMech('prefetch')} />
          <MechanismRow label="Hybrid Search"       detail="dense + sparse at rank-time"    active={isActiveMech('search')} />
          <MechanismRow label="Delta Delivery"      detail="ships only changed lines"       active={isActiveMech('delta')} />
          <MechanismRow label="Budget & Pressure"   detail="auto-compresses at 80%"         active={isActiveMech('budget')} />
          <MechanismRow label="Coherence Watcher"   detail="flags stale deliveries"         active={isActiveMech('coherence')} />
        </div>
      </FlowLayer>

      <Channel label="query · update" sub="read-through cache" direction={step.query} />

      <FlowLayer
        kicker="Index"
        meta="on-disk · persistent"
        tone="muted"
        active={isActiveLayer('index')}
      >
        <div className="grid grid-cols-2 gap-2">
          <StorageBox name="SQLite" detail="shadow · symbols" />
          <StorageBox name="HNSW"   detail="dense vectors · ANN" />
        </div>
      </FlowLayer>

      <Channel label="read-only" sub="never mutated by ministr" direction={step.corpus} />

      <FlowLayer
        kicker="Corpus"
        meta="your repo · your files"
        tone="muted"
        active={isActiveLayer('corpus')}
        terminal
      >
        <div className="flex items-center justify-between text-fd-muted-foreground">
          <span className="truncate">src/ · docs/ · CHANGELOG.md · …</span>
          <span className="shrink-0 rounded border border-fd-border/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wider">
            no network egress
          </span>
        </div>
      </FlowLayer>

      <StepDots
        steps={STEPS}
        stepIndex={stepIndex}
        progress={progress}
        onJump={onJump}
      />
    </div>
  );
}

/* ---------------------------------------------------------------
   Primitives
   --------------------------------------------------------------- */

function FlowLayer({
  kicker,
  meta,
  children,
  tone = 'muted',
  active = false,
  terminal = false,
}: {
  kicker: string;
  meta: string;
  children: ReactNode;
  tone?: 'muted' | 'featured';
  active?: boolean;
  terminal?: boolean;
}) {
  const isFeatured = tone === 'featured';
  return (
    <motion.div
      data-active={active}
      animate={{
        boxShadow: active
          ? '0 14px 50px -18px color-mix(in oklch, var(--color-ministr-500) 65%, transparent)'
          : isFeatured
            ? '0 10px 40px -20px color-mix(in oklch, var(--color-ministr-500) 50%, transparent)'
            : '0 0 0 transparent',
      }}
      transition={{ duration: 0.4 }}
      className={
        'relative rounded-xl border px-4 py-3.5 transition-colors duration-300 ' +
        (active
          ? 'border-[color-mix(in_oklch,var(--color-ministr-400)_55%,transparent)] bg-[color-mix(in_oklch,var(--color-ministr-500)_12%,transparent)]'
          : isFeatured
            ? 'border-[color-mix(in_oklch,var(--color-ministr-400)_40%,transparent)] bg-[color-mix(in_oklch,var(--color-ministr-500)_8%,transparent)]'
            : 'border-fd-border/50 bg-[color-mix(in_oklch,var(--ministr-surface)_45%,transparent)]')
      }
    >
      <div className="mb-3 flex items-baseline justify-between">
        <div className="flex items-baseline gap-2">
          <span
            className={
              'text-[10px] font-mono uppercase tracking-[0.22em] transition-colors ' +
              (active || isFeatured
                ? 'text-[var(--ministr-accent-text)]'
                : 'text-fd-muted-foreground')
            }
          >
            {kicker}
          </span>
          {active && (
            <span className="rounded bg-[color-mix(in_oklch,var(--color-ministr-500)_18%,transparent)] px-1.5 py-px text-[9.5px] uppercase tracking-wider text-[var(--ministr-accent-text)]">
              active
            </span>
          )}
        </div>
        <span className="text-[10px] text-fd-muted-foreground/80">{meta}</span>
      </div>
      {children}
    </motion.div>
  );
}

function Channel({
  label,
  sub,
  direction,
}: {
  label: string;
  sub?: string;
  direction: Direction;
}) {
  const active = direction !== 'idle';
  return (
    <div className="relative h-16">
      {/* Pipe — thin vertical line, centered horizontally, runs
          top→bottom with breathing room top/bottom. */}
      <div
        aria-hidden
        className={
          'absolute left-1/2 top-2 bottom-2 w-px -translate-x-1/2 transition-opacity duration-500 ' +
          (active
            ? 'bg-[color-mix(in_oklch,var(--color-ministr-400)_75%,transparent)] opacity-100'
            : 'bg-[color-mix(in_oklch,var(--color-ministr-400)_22%,transparent)] opacity-60')
        }
      />

      {/* Pulse — remounted on every direction change (via `key`) so
          the animation restarts cleanly. Transform-based so it stays
          snapped to the pipe. */}
      {active && (
        <span
          key={direction + '-pulse'}
          aria-hidden
          className={
            'absolute left-1/2 top-0 size-2 rounded-full bg-[var(--color-ministr-400)] shadow-[0_0_14px_var(--color-ministr-400)] ' +
            (direction === 'down' ? 'channel-pulse-down' : 'channel-pulse-up')
          }
        />
      )}

      {/* Label plate on the right, absolutely positioned so it never
          affects the pipe/pulse centering. */}
      <div className="absolute right-2 top-1/2 -translate-y-1/2 flex flex-col items-end">
        <span
          className={
            'font-mono text-[10.5px] transition-colors ' +
            (active
              ? 'text-[var(--ministr-accent-text)]'
              : 'text-fd-muted-foreground/70')
          }
        >
          {label}
        </span>
        {sub && (
          <span className="font-mono text-[9.5px] text-fd-muted-foreground/70">{sub}</span>
        )}
      </div>

      {/* Left-side direction hint (optional small arrow glyph) */}
      <div className="absolute left-2 top-1/2 -translate-y-1/2 font-mono text-[10px] text-fd-muted-foreground/60">
        {direction === 'down' ? '↓' : direction === 'up' ? '↑' : ' '}
      </div>
    </div>
  );
}

function Chip({
  children,
  active = false,
}: {
  children: ReactNode;
  active?: boolean;
}) {
  return (
    <span
      className={
        'rounded-md border px-2 py-0.5 text-[11px] transition-colors duration-300 ' +
        (active
          ? 'border-[color-mix(in_oklch,var(--color-ministr-400)_60%,transparent)] bg-[color-mix(in_oklch,var(--color-ministr-500)_22%,transparent)] text-fd-foreground'
          : 'border-fd-border/50 bg-[color-mix(in_oklch,var(--ministr-surface-strong)_40%,transparent)] text-fd-muted-foreground')
      }
    >
      {children}
    </span>
  );
}

function MechanismRow({
  label,
  detail,
  active = false,
}: {
  label: string;
  detail: string;
  active?: boolean;
}) {
  return (
    <motion.div
      animate={{
        backgroundColor: active
          ? 'color-mix(in oklch, var(--color-ministr-500) 20%, transparent)'
          : 'color-mix(in oklch, var(--ministr-surface) 35%, transparent)',
      }}
      transition={{ duration: 0.3 }}
      className={
        'flex items-center gap-2 rounded-md px-2.5 py-1.5 border transition-colors duration-300 ' +
        (active
          ? 'border-[color-mix(in_oklch,var(--color-ministr-400)_55%,transparent)]'
          : 'border-transparent')
      }
    >
      <span
        aria-hidden
        className={
          'shrink-0 transition-colors ' +
          (active ? 'text-[var(--color-ministr-400)]' : 'text-[var(--ministr-accent-text)]')
        }
      >
        ◇
      </span>
      <span className="shrink-0 text-fd-foreground">{label}</span>
      <span className="truncate text-[10.5px] text-fd-muted-foreground">{detail}</span>
      {active && (
        <span
          aria-hidden
          className="ml-auto size-1.5 shrink-0 rounded-full bg-[var(--color-ministr-400)]"
        />
      )}
    </motion.div>
  );
}

function StorageBox({ name, detail }: { name: string; detail: string }) {
  return (
    <div className="rounded-md border border-fd-border/40 bg-[color-mix(in_oklch,var(--ministr-surface-strong)_45%,transparent)] px-3 py-2">
      <div className="text-[11.5px] font-semibold text-fd-foreground">{name}</div>
      <div className="mt-0.5 truncate text-[10px] text-fd-muted-foreground">{detail}</div>
    </div>
  );
}

/* ---------------------------------------------------------------
   StepDots — minimal bar of clickable progress pips under the diagram.
   --------------------------------------------------------------- */

function StepDots({
  steps,
  stepIndex,
  progress,
  onJump,
}: {
  steps: Step[];
  stepIndex: number;
  progress: number;
  onJump: (i: number) => void;
}) {
  return (
    <div className="mt-5 flex items-center gap-1.5">
      {steps.map((s, i) => {
        const past = i < stepIndex;
        const current = i === stepIndex;
        const fill = current ? progress : past ? 1 : 0;
        return (
          <button
            key={s.id}
            type="button"
            onClick={() => onJump(i)}
            aria-label={s.label}
            aria-current={current || undefined}
            title={s.label}
            className="group relative h-1 flex-1 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--ministr-surface-strong)_75%,transparent)] transition-all hover:h-1.5"
          >
            <span
              className="absolute inset-y-0 left-0 rounded-full bg-[var(--color-ministr-500)] transition-all"
              style={{
                width: current ? `${Math.max(fill * 100, 8)}%` : `${fill * 100}%`,
                transitionDuration: current ? '60ms' : '300ms',
              }}
            />
          </button>
        );
      })}
    </div>
  );
}
