'use client';

/**
 * AgentRace — a replay of a REAL side-by-side run, animated as a race.
 *
 * Two agents, the same task, the same model — one with ministr, one with
 * grep/glob. The data in `race-data.json` is a genuine recording (each tool
 * call stamped by arrival time via `claude -p --output-format stream-json`;
 * see benchmarks/agent-task/record_race.py). The marker advances on the real
 * recorded wall-clock; tool calls tick as it passes them; the final
 * tokens/cost reveal at the finish line. Nothing here is synthesized.
 *
 * v2 language (warm ink, single amber, hairline, mono). Reduced-motion renders
 * the settled end state with no animation.
 */
import { useEffect, useRef, useState } from 'react';
import Link from 'next/link';
import race from './race-data.json';

const ANIM_SECONDS = 7; // compress the real wall-clock into ~7s of replay

type Ev = { t: number; kind: string; name: string; out: number; detail?: string };
type Arm = {
  label: string;
  uses_ministr?: boolean;
  events: Ev[];
  tool_calls?: number;
  solved?: boolean;
  num_turns?: number;
  total_cost_usd?: number;
  output_tokens?: number;
  wall?: number;
};

const ministr = (race.arms as Record<string, Arm>).ministr;
const grep = (race.arms as Record<string, Arm>).grep;
const LANES: { key: 'ministr' | 'grep'; arm: Arm }[] = [
  { key: 'ministr', arm: ministr },
  { key: 'grep', arm: grep },
];
const MAX_WALL = Math.max(ministr?.wall ?? 1, grep?.wall ?? 1);
const SAVED_PCT =
  ministr?.total_cost_usd && grep?.total_cost_usd
    ? Math.round((1 - ministr.total_cost_usd / grep.total_cost_usd) * 100)
    : null;

function reducedMotion() {
  return typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

export function AgentRace() {
  // vt = virtual recorded-seconds elapsed (0 → MAX_WALL).
  const [vt, setVt] = useState(0);
  const [running, setRunning] = useState(false);
  const raf = useRef<number | null>(null);
  const root = useRef<HTMLElement | null>(null);

  function play() {
    if (reducedMotion()) {
      setVt(MAX_WALL);
      return;
    }
    cancel();
    const speed = MAX_WALL / ANIM_SECONDS;
    const t0 = performance.now();
    setRunning(true);
    const tick = (now: number) => {
      const v = ((now - t0) / 1000) * speed;
      if (v >= MAX_WALL) {
        setVt(MAX_WALL);
        setRunning(false);
        raf.current = null;
        return;
      }
      setVt(v);
      raf.current = requestAnimationFrame(tick);
    };
    raf.current = requestAnimationFrame(tick);
  }
  function cancel() {
    if (raf.current != null) cancelAnimationFrame(raf.current);
    raf.current = null;
  }

  // Auto-play once when scrolled into view (or settle immediately if reduced).
  useEffect(() => {
    if (reducedMotion()) {
      setVt(MAX_WALL);
      return;
    }
    const el = root.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          play();
          io.disconnect();
        }
      },
      { threshold: 0.4 },
    );
    io.observe(el);
    return () => {
      io.disconnect();
      cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ariaLabel =
    `Replay of a real side-by-side run on ${race.repo} (${race.model}). ` +
    (SAVED_PCT != null
      ? `Both agents solved the task; the ministr agent finished using ${Math.abs(SAVED_PCT)}% ${SAVED_PCT >= 0 ? 'less' : 'more'} than the grep agent.`
      : 'Both agents solved the task.');

  return (
    <figure className="v2-race" ref={root as React.RefObject<HTMLElement>} aria-label={ariaLabel}>
      <div className="v2-race-head">
        <span className="v2-race-eyebrow">Same task · same agent · replayed from a real run</span>
        <button
          type="button"
          className="v2-race-replay"
          onClick={play}
          aria-label="Replay the run"
        >
          {running ? 'racing…' : '↻ replay'}
        </button>
      </div>

      <div className="v2-race-lanes">
        {LANES.map(({ key, arm }) => {
          if (!arm) return null;
          const wall = arm.wall ?? MAX_WALL;
          const prog = Math.min(vt / wall, 1);
          const done = vt >= wall;
          const toolsSeen = arm.events.filter((e) => e.kind === 'tool' && e.t <= vt).length;
          return (
            <div className={`v2-race-lane v2-race-${key}`} key={key}>
              <div className="v2-race-meta">
                <span className="v2-race-name">{arm.label}</span>
                <span className="v2-race-sub">{key === 'ministr' ? 'ministr MCP · no grep' : 'grep/glob · no ministr'}</span>
              </div>
              <div className="v2-race-track">
                <span className="v2-race-fill" style={{ width: `${prog * 100}%` }} />
                {arm.events
                  .filter((e) => e.kind === 'tool')
                  .map((e, i) => (
                    <span
                      key={i}
                      className={`v2-race-tick ${e.t <= vt ? 'lit' : ''}`}
                      style={{ left: `${Math.min(e.t / wall, 1) * 100}%` }}
                      title={e.detail ? `${e.name}: ${e.detail}` : e.name}
                    />
                  ))}
                <span className="v2-race-marker" style={{ left: `${prog * 100}%` }} aria-hidden />
                <span className="v2-race-flag" aria-hidden>{done && arm.solved ? '✓' : ''}</span>
              </div>
              <div className="v2-race-readout">
                {done ? (
                  <>
                    <b className="v2-race-cost">${arm.total_cost_usd?.toFixed(3)}</b>
                    <span>{arm.solved ? 'solved' : 'failed'}</span>
                    <span>{arm.tool_calls ?? toolsSeen} tools</span>
                    <span>{arm.num_turns} turns</span>
                  </>
                ) : (
                  <>
                    <span className="v2-race-live">{Math.min(vt, wall).toFixed(0)}s</span>
                    <span>{toolsSeen} tools</span>
                  </>
                )}
              </div>
            </div>
          );
        })}
      </div>

      <figcaption className="v2-race-cap">
        {SAVED_PCT != null && SAVED_PCT >= 5 ? (
          <>
            Same fix, same model — the <b>ministr</b> agent solved it for{' '}
            <b>{SAVED_PCT}% less</b>.{' '}
          </>
        ) : SAVED_PCT != null && SAVED_PCT <= -5 ? (
          <>
            Same fix, same model — the grep agent was {Math.abs(SAVED_PCT)}%
            cheaper on this particular run.{' '}
          </>
        ) : (
          <>Both agents solved it; cost was about even on this run.{' '}</>
        )}
        A replay of one real recorded run on {race.repo} ({race.model}); both
        solved. Early signal, small sample —{' '}
        <Link href="/docs" className="v2-race-link">
          see the benchmark →
        </Link>
      </figcaption>
    </figure>
  );
}
