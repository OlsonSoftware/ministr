/**
 * ActivityPulse — the agent's tool-call rhythm as a HEARTBEAT.
 *
 * The Activity board shows WHO is connected and their economics; this is the
 * at-a-glance picture of what they're DOING moment-to-moment. A bespoke
 * deterministic SVG histogram: tool-call events bucketed into ~48 windows over
 * the last few minutes, each bucket a vertical bar (height ∝ call count)
 * stack-segmented into cache-HITS (success, bottom) and MISSES (accent, top),
 * so the agent's tempo AND ministr's cache efficiency read together. The latest
 * bucket is a brighter live now-edge.
 *
 * No deps, no physics. The pure `ActivityPulse` renders from props (Storybook);
 * `ActivityPulseConnector` polls the global recent_activity ring buffer.
 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Activity } from "@/components/ui/icons";
import type { ActivityEvent } from "../../lib/types";
import { cn } from "../../lib/utils";

// ── Layout (SVG user units; scales to the container width). ──────────────────
const W = 600;
const H = 116;
const PAD_X = 3;
const TOP_Y = 10;
const BASE_Y = H - 18; // baseline; axis labels sit below it
const BAR_AREA = BASE_Y - TOP_Y;
const BUCKETS = 48;
const DEFAULT_WINDOW_MS = 5 * 60 * 1000;

interface Bucket {
  total: number;
  hits: number;
}

function bucketize(events: ActivityEvent[], now: number, windowMs: number): Bucket[] {
  const out: Bucket[] = Array.from({ length: BUCKETS }, () => ({ total: 0, hits: 0 }));
  // Defensive: the IPC ring buffer (or a story mock) can hand back a non-array.
  if (!Array.isArray(events)) return out;
  const size = windowMs / BUCKETS;
  for (const e of events) {
    const age = now - e.timestamp_ms;
    if (age < 0 || age >= windowMs) continue;
    // age 0 → newest bucket (rightmost, index BUCKETS-1).
    const idx = Math.min(BUCKETS - 1, Math.max(0, BUCKETS - 1 - Math.floor(age / size)));
    out[idx].total += 1;
    if (e.cache_hit) out[idx].hits += 1;
  }
  return out;
}

export interface ActivityPulseProps {
  events: ActivityEvent[];
  /** Window covered by the histogram (default 5 minutes). */
  windowMs?: number;
  /** Override "now" for deterministic rendering (defaults to Date.now()). */
  now?: number;
  className?: string;
}

export function ActivityPulse({
  events,
  windowMs = DEFAULT_WINDOW_MS,
  now,
  className,
}: ActivityPulseProps) {
  const ts = now ?? Date.now();
  const { buckets, total, hits, peak, lastActive } = useMemo(() => {
    const buckets = bucketize(events, ts, windowMs);
    let total = 0;
    let hits = 0;
    let peak = 0;
    let lastActive = -1;
    buckets.forEach((b, i) => {
      total += b.total;
      hits += b.hits;
      if (b.total > peak) peak = b.total;
      if (b.total > 0) lastActive = i;
    });
    return { buckets, total, hits, peak, lastActive };
  }, [events, ts, windowMs]);

  const minutes = Math.round(windowMs / 60000);
  const rate = total > 0 ? total / (windowMs / 60000) : 0;
  const hitPct = total > 0 ? Math.round((hits / total) * 100) : 0;
  const bucketSec = Math.round(windowMs / BUCKETS / 1000);

  const bucketW = (W - 2 * PAD_X) / BUCKETS;
  const barW = Math.max(2, bucketW - 1.5);

  const label =
    total === 0
      ? `Activity pulse: no tool calls in the last ${minutes} minutes.`
      : `Activity pulse over the last ${minutes} minutes: ${total} tool call${total === 1 ? "" : "s"}, ${rate.toFixed(1)} per minute, ${hitPct}% served from cache, peak ${peak} in a ${bucketSec}-second window.`;

  return (
    <div
      className={cn(
        "flex flex-col gap-2 rounded-xl border border-border bg-surface-raised px-4 py-3 shadow-sm",
        className,
      )}
    >
      {/* Eyebrow + readout. */}
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1.5 text-accent">
          <Activity className="h-3.5 w-3.5" strokeWidth={2} />
          <span className="font-mono text-mono-micro font-bold uppercase tracking-[0.12em]">
            Live activity
          </span>
        </div>
        <div className="flex items-center gap-4">
          <Stat value={total.toLocaleString()} label="calls" />
          <Stat value={total > 0 ? rate.toFixed(1) : "—"} label="/min" />
          <Stat
            value={total > 0 ? `${hitPct}%` : "—"}
            label="cache"
            tone={total > 0 ? "success" : undefined}
          />
        </div>
      </div>

      {/* The heartbeat. */}
      <div className="relative">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          className="w-full"
          style={{ maxHeight: H }}
          role="img"
          aria-label={label}
        >
          {/* Baseline. */}
          <line
            x1={PAD_X}
            y1={BASE_Y + 0.5}
            x2={W - PAD_X}
            y2={BASE_Y + 0.5}
            className="text-border"
            stroke="currentColor"
            strokeWidth={1}
          />

          {peak > 0 &&
            buckets.map((b, i) => {
              if (b.total === 0) return null;
              const x = PAD_X + i * bucketW + (bucketW - barW) / 2;
              const barH = Math.max(2, (b.total / peak) * BAR_AREA);
              const hitH = b.total > 0 ? (b.hits / b.total) * barH : 0;
              const missH = barH - hitH;
              const isLive = i === lastActive;
              return (
                <g key={i} opacity={isLive ? 1 : 0.82}>
                  {/* miss segment (accent) — top */}
                  {missH > 0.5 && (
                    <rect
                      x={x}
                      y={BASE_Y - barH}
                      width={barW}
                      height={missH}
                      rx={1}
                      className="fill-accent"
                    />
                  )}
                  {/* hit segment (success) — bottom */}
                  {hitH > 0.5 && (
                    <rect
                      x={x}
                      y={BASE_Y - hitH}
                      width={barW}
                      height={hitH}
                      rx={1}
                      className="fill-success"
                    />
                  )}
                </g>
              );
            })}

          {/* Axis ticks. */}
          <text
            x={PAD_X}
            y={H - 5}
            className="fill-text-dim font-mono"
            style={{ fontSize: 9 }}
          >
            {minutes}m ago
          </text>
          <text
            x={W - PAD_X}
            y={H - 5}
            textAnchor="end"
            className="fill-text-dim font-mono"
            style={{ fontSize: 9 }}
          >
            now
          </text>

          {total === 0 && (
            <text
              x={W / 2}
              y={BASE_Y - BAR_AREA / 2}
              textAnchor="middle"
              className="fill-text-dim font-mono"
              style={{ fontSize: 11, letterSpacing: 0.4 }}
            >
              Quiet — no tool activity
            </text>
          )}
        </svg>

        {/* The live now-edge marker — a reduced-motion-safe pulse at the right,
            only when the most-recent bucket is active. */}
        {lastActive === BUCKETS - 1 && (
          <span
            aria-hidden
            className="ministr-pulse absolute right-[3px] top-0 inline-block h-2 w-2 -translate-y-1/2 rounded-full bg-accent"
            style={{ top: `${(TOP_Y / H) * 100}%` }}
          />
        )}
      </div>
    </div>
  );
}

function Stat({
  value,
  label,
  tone,
}: {
  value: string;
  label: string;
  tone?: "success";
}) {
  return (
    <div className="flex items-baseline gap-1">
      <span
        className={cn(
          "font-mono text-sm font-semibold tabular-nums",
          tone === "success" ? "text-success" : "text-text",
        )}
      >
        {value}
      </span>
      <span className="font-mono text-mono-micro uppercase tracking-[0.08em] text-text-dim">
        {label}
      </span>
    </div>
  );
}

// ───────────────────────────────────────────────────────────────────────────
// Connector — polls the global recent_activity ring buffer.

const POLL_MS = 3000;
const LIMIT = 500;

function tauriReady(): boolean {
  return Boolean(
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__,
  );
}

/** Polls global tool-call activity (all sessions) and renders the pulse,
 *  filtered to the active corpus. Mount only when there are live sessions —
 *  the board's resting state should make no requests. */
export function ActivityPulseConnector({
  corpusId,
  className,
}: {
  corpusId: string | null;
  className?: string;
}) {
  const [events, setEvents] = useState<ActivityEvent[]>([]);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      if (cancelled) return;
      if (typeof document !== "undefined" && document.hidden) {
        timer = setTimeout(() => void tick(), POLL_MS);
        return;
      }
      if (tauriReady()) {
        try {
          const all = await invoke<ActivityEvent[]>("recent_activity", {
            limit: LIMIT,
          });
          if (!cancelled) {
            const list = Array.isArray(all) ? all : [];
            setEvents(
              corpusId ? list.filter((e) => e.corpus_id === corpusId) : list,
            );
          }
        } catch {
          // transient; keep the last snapshot and retry on the next tick.
        }
      }
      if (!cancelled) timer = setTimeout(() => void tick(), POLL_MS);
    };

    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [corpusId]);

  return <ActivityPulse events={events} className={className} />;
}
