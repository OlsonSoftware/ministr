'use client';

import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { Pause, Play, RotateCcw } from 'lucide-react';
import { Reveal } from '@/components/landing/reveal';

/**
 * Thesis — "What agents waste."
 *
 * Text block above, full-width interactive workflow-comparison
 * diagram below. The diagram is a playable step-through: press play
 * and watch both workflows execute in real time, with cumulative
 * token bars filling as each tool call lands.
 */
export function Thesis() {
  return (
    <section className="relative py-24 sm:py-32">
      <div className="mx-auto w-full max-w-6xl px-4 sm:px-6">
        <div className="grid grid-cols-1 gap-10 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] lg:items-end lg:gap-16">
          <div>
            <Reveal>
              <p className="iris-eyebrow">The problem</p>
            </Reveal>
            <h2 className="mt-5 space-y-2 text-[clamp(2rem,4.4vw,3.5rem)] font-semibold leading-[1.05] tracking-tight text-fd-foreground">
              <Reveal as="div" delay={0.0}>Grep finds lines. Read dumps files.</Reveal>
              <Reveal as="div" delay={0.12}>
                <span className="text-fd-foreground/75">Most of what comes back is noise.</span>
              </Reveal>
              <Reveal as="div" delay={0.24}>
                <span className="text-fd-foreground/55">Then the agent re-reads it.</span>
              </Reveal>
            </h2>
          </div>
          <Reveal delay={0.4}>
            <p className="iris-body max-w-[56ch] text-[17px] leading-relaxed">
              Claude Code&rsquo;s default tools are <span className="font-mono">Glob</span>,{' '}
              <span className="font-mono">Grep</span>, and{' '}
              <span className="font-mono">Read</span>: coarse text matching plus full-file dumps.
              Agents pay for every line, get no session memory, and re-read the same files
              turn after turn.{' '}
              <span className="font-medium text-fd-foreground">
                iris replaces all three with a semantic index that ships the exact section, once.
              </span>
            </p>
          </Reveal>
        </div>

        <Reveal delay={0.3} className="relative mt-14">
          <WasteDiagram />
        </Reveal>
      </div>
    </section>
  );
}

/**
 * WasteDiagram — interactive playback of grep+read vs iris.
 *
 * A playable step-through comparing the two workflows tool-call by
 * tool-call. Press play and cumulative token bars fill while each
 * call becomes active; the grep+read side blows past the 80% budget
 * wall while iris stays in a green zone. Hover a step to see the
 * output preview; click a step to jump to it.
 *
 * Why grep burns tokens: Claude Code's Grep prepends every match with
 * the full file path (Anthropic docs), and Read returns entire files.
 * So finding a 22-line function means paying for full 340-line file
 * dumps across several candidates. Followups pay for re-reads too
 * because Claude Code has no session memory across turns.
 *
 * Why iris saves: iris_survey returns ranked section IDs, not line
 * dumps. iris_definition returns only the function body. Follow-up
 * reads turn into delta pointers via the session shadow.
 */
type Step = {
  tool: string;
  args: string;
  tokens: number;
  signal: number;
  preview: string;
  note: string;
};

// Claude Code path — reflects the actual Glob/Grep/Read toolset.
const WITHOUT_IRIS: Step[] = [
  {
    tool: 'Glob',
    args: '"**/*.rs"',
    tokens: 900,
    signal: 60,
    preview:
      'src/main.rs\nsrc/api/users.rs\nsrc/api/routes.rs\nsrc/validators.rs\nsrc/db/schema.rs\n… 115 more paths',
    note: '120 paths · one of them matters',
  },
  {
    tool: 'Grep',
    args: '"validate_email"',
    tokens: 2400,
    signal: 180,
    preview:
      'src/api/users.rs:42:fn validate_email(e: &str) -> Result<Email> {\nsrc/api/users.rs:58:    if validate_email(&req.email).is_err() {\nsrc/validators.rs:14:pub fn validate_email(…) {\n… 44 more lines, each prefixed with a full path',
    note: '47 matches · full path prepended to every line',
  },
  {
    tool: 'Read',
    args: 'src/api/users.rs',
    tokens: 3400,
    signal: 220,
    preview:
      'use crate::db::pool;\nuse crate::auth::Session;\n\npub struct User { … }\nimpl User { … }\n\nfn validate_email(e: &str) -> Result<Email> {\n  // 22 real lines here\n}\n… 280 more lines of unrelated impls',
    note: '340 lines · needed 22',
  },
  {
    tool: 'Read',
    args: 'src/validators.rs',
    tokens: 1800,
    signal: 150,
    preview:
      'pub struct Validator { … }\nimpl Validator { … }\n\npub fn validate_email(…) { … }\n\n… 160 more lines of other validators',
    note: '180 lines · needed 15',
  },
  {
    tool: 'Read',
    args: 'src/api/users.rs',
    tokens: 3400,
    signal: 0,
    preview:
      'identical 340-line dump as the earlier Read call. Claude\nCode has no session memory of what was already sent —\nthe follow-up question pays the full cost again.',
    note: 're-read on follow-up · zero new signal',
  },
];

// iris path — semantic survey + direct section read. Follow-up call
// becomes a delta pointer because the session shadow remembers.
const WITH_IRIS: Step[] = [
  {
    tool: 'iris_survey',
    args: '"validate email function"',
    tokens: 200,
    signal: 200,
    preview:
      '[\n  { id: "users.rs#validate_email", score: 0.94 },\n  { id: "validators.rs#Validator::email", score: 0.81 },\n  { id: "tests/users_test.rs#email", score: 0.73 },\n  { id: "schema.sql#users.email_col", score: 0.61 },\n  { id: "docs/auth.md#email-format", score: 0.58 }\n]',
    note: 'ranked section ids · no line dumps',
  },
  {
    tool: 'iris_definition',
    args: '"users.rs#validate_email"',
    tokens: 220,
    signal: 220,
    preview:
      'fn validate_email(e: &str) -> Result<Email> {\n  let trimmed = e.trim();\n  if trimmed.is_empty() { return Err(Empty) }\n  EMAIL_RE.is_match(trimmed)\n    .then(|| Email(trimmed.into()))\n    .ok_or(Invalid)\n}\n// (22 lines total — exact function body)',
    note: '22 lines · nothing else',
  },
  {
    tool: 'iris_references',
    args: '"validate_email"',
    tokens: 100,
    signal: 100,
    preview:
      '[\n  "src/api/users.rs:58 (caller)",\n  "src/api/signup.rs:34 (caller)",\n  "src/api/admin.rs:201 (caller)",\n  "tests/users_test.rs:12 (caller)"\n]',
    note: '4 callers · surgical list',
  },
  {
    tool: 'iris_read',
    args: 'users.rs#validate_email',
    tokens: 5,
    signal: 5,
    preview:
      '{ type: "delta-pointer", section_id: "users.rs#validate_email", already_sent_at: "t2" }\n\nsession shadow recognised the re-request — no content shipped.',
    note: 'delta pointer · already in session',
  },
];

function sum(s: Step[], k: 'tokens' | 'signal') {
  return s.reduce((a, v) => a + v[k], 0);
}

const BLIND_TOTAL = sum(WITHOUT_IRIS, 'tokens');
const IRIS_TOTAL = sum(WITH_IRIS, 'tokens');
const BUDGET = 12500; // token budget for the cumulative bar
const WALL = 0.8 * BUDGET; // 80% pressure line
const STEP_MS = 900;     // playback cadence
const TOTAL_STEPS = Math.max(WITHOUT_IRIS.length, WITH_IRIS.length);

// Rough Claude Sonnet 4 pricing as of early 2026: $3/1M input tokens.
// Shown only as a subtle "at scale, across a 10k-session week" multiplier
// so the hero metric lands emotionally.
const USD_PER_MTOK = 3;
const SESSIONS_PER_WEEK = 10_000;
const usd = (tokens: number) => (tokens / 1_000_000) * USD_PER_MTOK;
const weeklyUsd = (tokens: number) => usd(tokens) * SESSIONS_PER_WEEK;

/**
 * cumulative(steps, upto) — cumulative token/signal totals AFTER the
 * step at index `upto` has landed (inclusive). upto=-1 means no
 * steps have landed yet.
 */
function cumulative(steps: Step[], upto: number) {
  let tok = 0, sig = 0;
  for (let i = 0; i <= Math.min(upto, steps.length - 1); i++) {
    tok += steps[i].tokens;
    sig += steps[i].signal;
  }
  return { tok, sig };
}

function WasteDiagram() {
  const reduced = useReducedMotion();
  const [active, setActive] = useState(-1); // -1 = nothing played yet
  const [playing, setPlaying] = useState(false);
  const [hovered, setHovered] = useState<null | { side: 'blind' | 'iris'; index: number }>(null);
  const timer = useRef<number | null>(null);

  // Auto-run: when Play pressed, advance active step every STEP_MS
  // until TOTAL_STEPS finish, then stop.
  useEffect(() => {
    if (!playing) return;
    if (active >= TOTAL_STEPS - 1) {
      setPlaying(false);
      return;
    }
    timer.current = window.setTimeout(() => {
      setActive((a) => Math.min(a + 1, TOTAL_STEPS - 1));
    }, STEP_MS);
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, [playing, active]);

  const reset = () => {
    setActive(-1);
    setPlaying(false);
  };

  const togglePlay = () => {
    if (active >= TOTAL_STEPS - 1) {
      // play from start
      setActive(-1);
      setPlaying(true);
    } else {
      setPlaying((p) => !p);
    }
  };

  // Keyboard shortcuts bound to the diagram container (scoped via
  // tabIndex below so they only fire when the diagram has focus or
  // its descendants do). Keeps the page-wide tab/keyboard UX clean.
  const onKeyDown = (e: React.KeyboardEvent) => {
    // Ignore when the user is typing in a real input (none today, but
    // cheap guard against future form fields landing inside this tree).
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    if (e.key === ' ' || e.code === 'Space') {
      e.preventDefault();
      togglePlay();
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      setPlaying(false);
      setActive((a) => Math.min(a + 1, TOTAL_STEPS - 1));
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      setPlaying(false);
      setActive((a) => Math.max(a - 1, -1));
    } else if (e.key === 'r' || e.key === 'R') {
      e.preventDefault();
      reset();
    }
  };

  // Cumulative token & signal totals driven by `active`. These drive
  // the running budget bars at the top of the diagram.
  const blindCum = cumulative(WITHOUT_IRIS, active);
  const irisCum = cumulative(WITH_IRIS, active);
  const blindPastWall = blindCum.tok >= WALL;

  const activePreview = hovered
    ? (hovered.side === 'blind' ? WITHOUT_IRIS : WITH_IRIS)[hovered.index]
    : active >= 0
      ? // Show the most recently activated step on the grep+read side by default
        WITHOUT_IRIS[Math.min(active, WITHOUT_IRIS.length - 1)]
      : null;

  const reduction = Math.round((1 - IRIS_TOTAL / BLIND_TOTAL) * 100);
  const multiple = Math.round(BLIND_TOTAL / IRIS_TOTAL);

  return (
    <div
      className="glass-card relative overflow-hidden p-5 sm:p-7 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_oklch,var(--color-iris-400)_55%,transparent)]"
      role="region"
      aria-label="Interactive comparison: grep + read workflow vs iris workflow. Use Space to play/pause, arrow keys to step, R to reset."
      tabIndex={0}
      onKeyDown={onKeyDown}
    >
      {/* ── Header: eyebrow + task + playback controls ─────────── */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <p className="iris-eyebrow">One task · two workflows</p>
          <p className="iris-body-quiet mt-2 font-mono text-[12px] leading-relaxed">
            <span className="text-fd-foreground">task:</span> find and edit the{' '}
            <span className="text-[var(--iris-accent-text)]">validate_email</span>{' '}
            function in a mid-size Rust repo.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={togglePlay}
            aria-label={playing ? 'Pause playback (space)' : 'Play workflow comparison (space)'}
            aria-pressed={playing}
            className="inline-flex items-center gap-1.5 rounded-lg border border-[color-mix(in_oklch,var(--color-iris-400)_40%,transparent)] bg-[color-mix(in_oklch,var(--color-iris-500)_16%,transparent)] px-3 py-1.5 font-mono text-[12px] font-medium text-fd-foreground transition hover:bg-[color-mix(in_oklch,var(--color-iris-500)_26%,transparent)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_oklch,var(--color-iris-400)_55%,transparent)]"
          >
            {playing ? (
              <>
                <Pause className="size-3.5" aria-hidden /> Pause
              </>
            ) : active >= TOTAL_STEPS - 1 ? (
              <>
                <Play className="size-3.5" aria-hidden /> Replay
              </>
            ) : (
              <>
                <Play className="size-3.5" aria-hidden /> Play
              </>
            )}
          </button>
          <button
            type="button"
            onClick={reset}
            aria-label="Reset playback (R)"
            className="inline-flex items-center gap-1.5 rounded-lg border border-fd-border/50 bg-[color-mix(in_oklch,var(--iris-surface)_45%,transparent)] px-2.5 py-1.5 font-mono text-[12px] text-fd-muted-foreground transition hover:text-fd-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_oklch,var(--color-iris-400)_55%,transparent)]"
          >
            <RotateCcw className="size-3.5" aria-hidden />
          </button>
          <span className="iris-body-quiet ml-1 font-mono text-[10.5px] uppercase tracking-[0.16em]">
            step {String(Math.max(active + 1, 0)).padStart(2, '0')} / {String(TOTAL_STEPS).padStart(2, '0')}
          </span>
        </div>
      </div>

      {/* Keyboard shortcuts hint — only visible after focus, keeps
          the panel quiet for mouse users while power users get it. */}
      <p
        className="iris-body-quiet mt-2 hidden font-mono text-[10px] uppercase tracking-[0.16em] md:block"
        aria-hidden
      >
        <kbd className="rounded border border-fd-border/50 bg-[color-mix(in_oklch,var(--iris-surface)_55%,transparent)] px-1">space</kbd>{' '}
        play/pause ·{' '}
        <kbd className="rounded border border-fd-border/50 bg-[color-mix(in_oklch,var(--iris-surface)_55%,transparent)] px-1">← →</kbd>{' '}
        step ·{' '}
        <kbd className="rounded border border-fd-border/50 bg-[color-mix(in_oklch,var(--iris-surface)_55%,transparent)] px-1">R</kbd>{' '}
        reset · click any step to jump
      </p>

      {/* ── Running cumulative budget bars. These are the hero — they
             fill as steps become active, crossing the 80% pressure
             wall on the grep+read side. ──────────────────────────── */}
      <div className="mt-6 grid grid-cols-1 gap-4 md:grid-cols-2">
        <CumulativeBar
          label="grep + read"
          sublabel="Claude Code default"
          total={blindCum.tok}
          budget={BUDGET}
          past={blindPastWall}
          accent="warning"
          reduced={!!reduced}
        />
        <CumulativeBar
          label="iris"
          sublabel={active >= TOTAL_STEPS - 1 ? `−${reduction}% (${multiple}× less)` : 'semantic · stateful'}
          total={irisCum.tok}
          budget={BUDGET}
          past={false}
          accent="success"
          reduced={!!reduced}
        />
      </div>

      {/* ── Two-column playback: the tool-call timeline. Each row
             highlights as it becomes active during playback. ─────── */}
      <div className="mt-6 grid grid-cols-1 gap-5 md:grid-cols-2">
        <Column
          title="grep + read workflow"
          subtitle={`${WITHOUT_IRIS.length} calls · ${BLIND_TOTAL.toLocaleString()} tokens · 95% noise`}
          steps={WITHOUT_IRIS}
          mode="blind"
          active={active}
          hovered={hovered}
          onHover={setHovered}
          onJump={(i) => { setPlaying(false); setActive(i); }}
          reduced={!!reduced}
        />
        <Column
          title="iris workflow"
          subtitle={`${WITH_IRIS.length} calls · ${IRIS_TOTAL.toLocaleString()} tokens · 100% signal`}
          steps={WITH_IRIS}
          mode="iris"
          active={active}
          hovered={hovered}
          onHover={setHovered}
          onJump={(i) => { setPlaying(false); setActive(i); }}
          reduced={!!reduced}
        />
      </div>

      {/* ── Verdict card appears when playback completes ──────── */}
      <AnimatePresence>
        {active >= TOTAL_STEPS - 1 && (
          <Verdict blind={BLIND_TOTAL} iris={IRIS_TOTAL} reduced={!!reduced} />
        )}
      </AnimatePresence>

      {/* ── Output preview pane: shows what the currently-active
             (or hovered) tool call actually returned. Makes the
             noise-vs-signal argument concrete. ────────────────────── */}
      <div className="mt-6 rounded-lg border border-[color-mix(in_oklch,var(--color-iris-400)_18%,transparent)] bg-[color-mix(in_oklch,var(--iris-surface-strong)_40%,transparent)] p-4">
        <div className="flex items-center justify-between">
          <span className="iris-body-quiet font-mono text-[10px] uppercase tracking-[0.18em]">
            {activePreview ? 'tool output' : 'press play, or hover a step'}
          </span>
          {activePreview && (
            <span className="font-mono text-[10.5px] tabular-nums text-fd-foreground/75">
              {activePreview.tokens.toLocaleString()} tok ·{' '}
              <span
                className={
                  activePreview.signal / activePreview.tokens > 0.5
                    ? 'text-[var(--color-success)]'
                    : 'text-[var(--color-warning)]'
                }
              >
                {Math.round((activePreview.signal / Math.max(activePreview.tokens, 1)) * 100)}% signal
              </span>
            </span>
          )}
        </div>
        {/* Scroll lives on this wrapper so the animated pre keeps
            overflow: visible and doesn't clip the top of its content
            during the fade/slide transition. */}
        <div className="mt-2 overflow-x-auto">
          <AnimatePresence mode="wait">
            <motion.div
              key={(activePreview?.tool ?? 'idle') + (activePreview?.args ?? '')}
              initial={reduced ? false : { opacity: 0, y: 4 }}
              animate={reduced ? undefined : { opacity: 1, y: 0 }}
              exit={reduced ? undefined : { opacity: 0, y: -4 }}
              transition={{ duration: 0.25, ease: [0.2, 0.8, 0.2, 1] }}
            >
              <TypewriterBlock
                text={
                  activePreview
                    ? activePreview.preview
                    : 'Press Play to watch both workflows execute in real time.\nHover any step to preview what that tool call returned.'
                }
                reduced={!!reduced}
              />
            </motion.div>
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}

/* ---------------- sub-components ---------------- */

function CumulativeBar({
  label,
  sublabel,
  total,
  budget,
  past,
  accent,
  reduced,
}: {
  label: string;
  sublabel: string;
  total: number;
  budget: number;
  past: boolean;
  accent: 'warning' | 'success';
  reduced: boolean;
}) {
  const pct = Math.min((total / budget) * 100, 100);
  const accentVar = accent === 'warning' ? 'var(--color-warning)' : 'var(--color-success)';

  // Detect wall crossing to trigger a one-shot shockwave pulse.
  const [justCrossed, setJustCrossed] = useState(false);
  const wasPast = useRef(past);
  useEffect(() => {
    if (!wasPast.current && past) {
      setJustCrossed(true);
      const t = window.setTimeout(() => setJustCrossed(false), 900);
      return () => window.clearTimeout(t);
    }
    wasPast.current = past;
  }, [past]);

  return (
    <motion.div
      animate={
        justCrossed && !reduced
          ? { x: [0, -3, 3, -2, 2, 0], boxShadow: [
              '0 0 0 0 color-mix(in oklch, var(--color-warning) 0%, transparent)',
              '0 0 0 10px color-mix(in oklch, var(--color-warning) 40%, transparent)',
              '0 0 0 20px color-mix(in oklch, var(--color-warning) 0%, transparent)',
            ] }
          : { x: 0 }
      }
      transition={{ duration: 0.6, ease: [0.2, 0.8, 0.2, 1] }}
      className={
        'relative overflow-hidden rounded-lg border px-4 py-3 transition-colors ' +
        (accent === 'warning'
          ? 'border-[color-mix(in_oklch,var(--color-warning)_25%,transparent)] bg-[color-mix(in_oklch,var(--color-warning)_7%,transparent)]'
          : 'border-[color-mix(in_oklch,var(--color-success)_28%,transparent)] bg-[color-mix(in_oklch,var(--color-success)_7%,transparent)]')
      }
    >
      {/* Alert scanline that sweeps across the bar on wall-crossing */}
      {justCrossed && !reduced && (
        <motion.div
          aria-hidden
          className="pointer-events-none absolute inset-0"
          initial={{ x: '-100%' }}
          animate={{ x: '100%' }}
          transition={{ duration: 0.7, ease: 'easeOut' }}
          style={{
            background:
              'linear-gradient(90deg, transparent 0%, color-mix(in oklch, var(--color-warning) 35%, transparent) 50%, transparent 100%)',
          }}
        />
      )}

      <div className="relative flex items-baseline justify-between">
        <span className="font-mono text-[11px] uppercase tracking-[0.18em]">
          <span style={{ color: accentVar }}>{label}</span>
          <span className="iris-body-quiet ml-2">({sublabel})</span>
        </span>
        <span
          className="font-mono text-[clamp(1.1rem,1.8vw,1.35rem)] font-semibold tabular-nums"
          style={{ color: accentVar }}
        >
          <AnimatedNumber value={total} reduced={reduced} />
          <span className="iris-body-quiet ml-1 text-[10.5px] font-normal">tok</span>
        </span>
      </div>
      <div className="relative mt-2 h-2.5 overflow-hidden rounded-full bg-[color-mix(in_oklch,var(--iris-surface-strong)_65%,transparent)]">
        <motion.span
          className="absolute inset-y-0 left-0 rounded-full"
          style={{
            background: accentVar,
            boxShadow: past
              ? '0 0 12px color-mix(in oklch, var(--color-warning) 55%, transparent)'
              : 'none',
          }}
          animate={{ width: `${pct}%` }}
          transition={{ duration: reduced ? 0 : 0.5, ease: [0.2, 0.8, 0.2, 1] }}
        />
        {/* Past-the-wall pulsing overlay — when grep+read crosses 80%
            a pressure-warning stripes animation plays over the bar. */}
        {past && !reduced && (
          <motion.span
            aria-hidden
            className="absolute inset-y-0 left-0 rounded-full"
            style={{
              width: `${pct}%`,
              background:
                'repeating-linear-gradient(45deg, transparent 0 6px, color-mix(in oklch, var(--color-warning) 40%, transparent) 6px 10px)',
            }}
            animate={{ backgroundPositionX: ['0px', '40px'] }}
            transition={{ duration: 0.9, repeat: Infinity, ease: 'linear' }}
          />
        )}
        {/* 80% pressure wall */}
        <div
          aria-hidden
          className="pointer-events-none absolute inset-y-[-3px] left-[80%] w-px bg-[var(--color-warning)]"
        />
        <div
          aria-hidden
          className="pointer-events-none absolute left-[80%] -top-3.5 -translate-x-1/2 font-mono text-[9px] uppercase tracking-[0.16em] text-[var(--color-warning)]"
        >
          80%
        </div>
      </div>
      <div className="mt-1 flex items-baseline justify-between font-mono text-[10px] uppercase tracking-[0.18em]">
        <span className="iris-body-quiet">
          {'$'}
          {usd(total).toFixed(4)} this session · {'$'}
          {Math.round(weeklyUsd(total)).toLocaleString()}/wk @ 10k sessions
        </span>
        <motion.span
          style={{ color: accentVar }}
          animate={past && !reduced ? { opacity: [1, 0.4, 1] } : { opacity: 1 }}
          transition={{ duration: 1.1, repeat: past ? Infinity : 0, ease: 'easeInOut' }}
        >
          {past ? 'past 80% · pressure' : accent === 'warning' ? 'climbing' : 'healthy'}
        </motion.span>
      </div>
    </motion.div>
  );
}

function Column({
  title,
  subtitle,
  steps,
  mode,
  active,
  hovered,
  onHover,
  onJump,
  reduced,
}: {
  title: string;
  subtitle: string;
  steps: Step[];
  mode: 'blind' | 'iris';
  active: number;
  hovered: null | { side: 'blind' | 'iris'; index: number };
  onHover: (h: null | { side: 'blind' | 'iris'; index: number }) => void;
  onJump: (i: number) => void;
  reduced: boolean;
}) {
  const SCALE = 3600; // shared so rows line up across columns
  return (
    <div>
      <div className="mb-2 flex items-baseline justify-between font-mono text-[10px] uppercase tracking-[0.18em]">
        <span className={mode === 'blind' ? 'text-[var(--color-warning)]' : 'text-[var(--color-success)]'}>
          {title}
        </span>
        <span className="iris-body-quiet">{subtitle}</span>
      </div>
      <ol className="space-y-2">
        {steps.map((s, i) => {
          const isActive = i <= active;
          const isCurrent = i === active;
          const isHovered = hovered?.side === mode && hovered.index === i;
          const signalPct = s.signal / Math.max(s.tokens, 1);
          return (
            <li
              key={i}
              role="button"
              tabIndex={0}
              aria-label={`Jump to step ${i + 1}: ${s.tool} ${s.args}`}
              aria-current={isCurrent ? 'step' : undefined}
              onMouseEnter={() => onHover({ side: mode, index: i })}
              onMouseLeave={() => onHover(null)}
              onClick={() => onJump(i)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  e.stopPropagation();
                  onJump(i);
                }
              }}
              className={
                'group cursor-pointer rounded-md border px-3 py-2 transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[color-mix(in_oklch,var(--color-iris-400)_55%,transparent)] ' +
                (isActive
                  ? 'border-[color-mix(in_oklch,var(--color-iris-400)_32%,transparent)] bg-[color-mix(in_oklch,var(--iris-surface-strong)_55%,transparent)]'
                  : 'border-[color-mix(in_oklch,var(--color-iris-400)_10%,transparent)] bg-[color-mix(in_oklch,var(--iris-surface-strong)_20%,transparent)] opacity-55') +
                (isCurrent
                  ? ' ring-1 ring-[color-mix(in_oklch,var(--color-iris-400)_50%,transparent)] shadow-[0_0_0_4px_color-mix(in_oklch,var(--color-iris-400)_12%,transparent)]'
                  : '') +
                (isHovered ? ' scale-[1.01]' : '')
              }
            >
              <div className="flex items-baseline justify-between gap-2">
                <span className="flex min-w-0 items-baseline gap-2 truncate font-mono text-[12px]">
                  <span className="iris-body-quiet shrink-0">{String(i + 1).padStart(2, '0')}</span>
                  <span
                    className={
                      'shrink-0 ' +
                      (mode === 'blind'
                        ? 'text-[var(--color-warning)]'
                        : 'text-[var(--iris-accent-text)]')
                    }
                  >
                    {s.tool}
                  </span>
                  <span className="truncate text-fd-foreground/65">({s.args})</span>
                </span>
                <span className="shrink-0 font-mono text-[11px] tabular-nums text-fd-foreground/85">
                  {s.tokens.toLocaleString()} tok
                </span>
              </div>

              {/* Signal / noise bar — animates in when this step
                  becomes active during playback. */}
              <div className="mt-2 flex h-[8px] w-full overflow-hidden rounded-[3px] bg-[color-mix(in_oklch,var(--iris-surface-strong)_70%,transparent)]">
                <motion.span
                  className="h-full"
                  style={{
                    background: mode === 'iris' ? 'var(--color-success)' : 'var(--color-iris-500)',
                  }}
                  animate={{ width: `${isActive ? (s.signal / SCALE) * 100 : 0}%` }}
                  transition={{ duration: reduced ? 0 : 0.35, ease: [0.2, 0.8, 0.2, 1] }}
                />
                <motion.span
                  className="h-full"
                  style={{
                    background:
                      'repeating-linear-gradient(45deg, color-mix(in oklch, var(--color-warning) 70%, transparent) 0 4px, color-mix(in oklch, var(--color-warning) 20%, transparent) 4px 8px)',
                  }}
                  animate={{ width: `${isActive ? ((s.tokens - s.signal) / SCALE) * 100 : 0}%` }}
                  transition={{ duration: reduced ? 0 : 0.35, ease: [0.2, 0.8, 0.2, 1] }}
                />
              </div>

              <div className="mt-1.5 flex items-baseline justify-between font-mono text-[10px]">
                <span className="iris-body-quiet truncate pr-2">{s.note}</span>
                <span
                  className={
                    'shrink-0 uppercase tracking-[0.14em] ' +
                    (signalPct > 0.5 ? 'text-[var(--color-success)]' : 'text-[var(--color-warning)]')
                  }
                >
                  {Math.round(signalPct * 100)}% signal
                </span>
              </div>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/**
 * AnimatedNumber — smoothly interpolates to the target value so the
 * cumulative counters read like a live dashboard during playback.
 */
function AnimatedNumber({ value, reduced }: { value: number; reduced: boolean }) {
  const [display, setDisplay] = useState(value);
  const prev = useRef(value);
  useEffect(() => {
    if (reduced) {
      setDisplay(value);
      prev.current = value;
      return;
    }
    const from = prev.current;
    const to = value;
    const dur = 480;
    const start = performance.now();
    let raf = 0;
    const tick = (t: number) => {
      const k = Math.min(1, (t - start) / dur);
      const e = 1 - Math.pow(1 - k, 3);
      setDisplay(Math.round(from + (to - from) * e));
      if (k < 1) raf = requestAnimationFrame(tick);
      else prev.current = to;
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value, reduced]);
  return <span>{display.toLocaleString()}</span>;
}

/**
 * TypewriterBlock — reveals multiline code output one character at a
 * time with a blinking caret. Feels like a live terminal session
 * rather than a static screenshot. Respects prefers-reduced-motion.
 */
function TypewriterBlock({ text, reduced }: { text: string; reduced: boolean }) {
  const [i, setI] = useState(reduced ? text.length : 0);
  const cancelRef = useRef<number | null>(null);

  useEffect(() => {
    if (reduced) {
      setI(text.length);
      return;
    }
    setI(0);
    const totalMs = Math.min(900, Math.max(280, text.length * 6));
    const start = performance.now();
    const tick = (t: number) => {
      const k = Math.min(1, (t - start) / totalMs);
      // Ease-out so the tail finishes gently, not abruptly.
      const e = 1 - Math.pow(1 - k, 2);
      setI(Math.round(e * text.length));
      if (k < 1) cancelRef.current = requestAnimationFrame(tick);
    };
    cancelRef.current = requestAnimationFrame(tick);
    return () => {
      if (cancelRef.current !== null) cancelAnimationFrame(cancelRef.current);
    };
  }, [text, reduced]);

  const shown = text.slice(0, i);
  const done = i >= text.length;
  return (
    <pre className="whitespace-pre font-mono text-[11.5px] leading-[1.55] text-fd-foreground/85">
      {shown}
      <span
        aria-hidden
        className={
          'inline-block w-[0.5ch] translate-y-[2px] bg-[var(--iris-accent-text)] ' +
          (done ? 'animate-pulse' : '')
        }
        style={{ height: '0.95em' }}
      />
    </pre>
  );
}

/**
 * Verdict — hero metric card that slides up when playback finishes.
 * Shows the multiplier in a huge number, with sublines for tokens
 * saved and dollar-per-week impact at 10k sessions.
 */
function Verdict({
  blind,
  iris,
  reduced,
}: {
  blind: number;
  iris: number;
  reduced: boolean;
}) {
  const multiple = Math.round(blind / Math.max(iris, 1));
  const reduction = Math.round((1 - iris / blind) * 100);
  const weeklySaved = Math.round(weeklyUsd(blind - iris));
  return (
    <motion.div
      initial={reduced ? false : { opacity: 0, y: 12, scale: 0.98 }}
      animate={reduced ? undefined : { opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: 0.55, ease: [0.2, 0.8, 0.2, 1] }}
      className="relative mt-5 overflow-hidden rounded-lg border border-[color-mix(in_oklch,var(--color-iris-400)_32%,transparent)] bg-gradient-to-br from-[color-mix(in_oklch,var(--color-iris-500)_14%,transparent)] via-[color-mix(in_oklch,var(--color-violet-500)_10%,transparent)] to-[color-mix(in_oklch,var(--color-fuchsia-400)_10%,transparent)] px-4 py-4"
    >
      {/* Spectrum hairline */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 top-0 h-px"
        style={{
          background:
            'linear-gradient(90deg in oklch, transparent 0%, var(--color-iris-500) 25%, var(--color-violet-500) 55%, var(--color-fuchsia-400) 80%, transparent 100%)',
          opacity: 0.8,
        }}
      />
      <div className="flex flex-wrap items-baseline justify-between gap-3">
        <div>
          <span className="iris-eyebrow">Verdict</span>
          <div className="mt-1 flex items-baseline gap-3">
            <span className="font-mono text-[clamp(2rem,4vw,3rem)] font-semibold leading-none tabular-nums text-fd-foreground">
              {multiple}×
            </span>
            <span className="iris-body font-mono text-[12px] uppercase tracking-[0.18em]">
              fewer tokens · −{reduction}%
            </span>
          </div>
        </div>
        <div className="flex flex-col items-end text-right">
          <span className="font-mono text-[clamp(1.1rem,2vw,1.45rem)] font-semibold tabular-nums text-[var(--color-success)]">
            ${weeklySaved.toLocaleString()}
          </span>
          <span className="iris-body-quiet font-mono text-[10.5px] uppercase tracking-[0.18em]">
            saved per week @ 10k sessions
          </span>
        </div>
      </div>
      <p className="iris-body-quiet mt-3 font-mono text-[11px] leading-relaxed">
        same task, same agent, same model — just the retrieval layer changed. the saved budget
        becomes reasoning room for the <span className="text-fd-foreground">next</span> question.
      </p>
    </motion.div>
  );
}
