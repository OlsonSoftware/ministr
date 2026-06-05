/**
 * LogRateRibbon — the daemon's log volume as a severity-banded HISTOGRAM.
 *
 * The console below shows the raw tail; this is the at-a-glance shape of the
 * stream: the recent buffered lines bucketed left→right (oldest→newest) into a
 * deterministic SVG histogram, each bucket a vertical bar (height ∝ line count)
 * stack-segmented by severity — INFO (neutral, bottom), WARN (warning, middle),
 * ERROR (danger, top) — so an error burst literally rises red out of the noise.
 * This is the 2026 observability idiom (a log-events histogram grouped by
 * severity), brought into ministr's command-deck instrument family via VizFrame.
 *
 * Pure + deterministic: renders from the per-line levels prop (no clock, no
 * IPC), so Storybook and the live console drive the same component. The live
 * now-edge marker is a reduced-motion-safe DOM pulse OUTSIDE the SVG (box-shadow
 * glow does not render on SVG nodes).
 */
import { useMemo, type ReactNode } from "react";
import { ScrollText } from "@/components/ui/icons";
import { cn } from "../../lib/utils";
import { VizFrame } from "../ui/viz-frame";

export type LogLevel = "error" | "warn" | "info";

// ── Layout (SVG user units; scales to the container width). ──────────────────
const W = 600;
const H = 100;
const PAD_X = 3;
const TOP_Y = 8;
const BASE_Y = H - 15; // baseline; axis labels sit below it
const BAR_AREA = BASE_Y - TOP_Y;
const BUCKETS = 56;

interface RateBucket {
  info: number;
  warn: number;
  error: number;
  total: number;
}

function bucketize(levels: LogLevel[]): RateBucket[] {
  const out: RateBucket[] = Array.from({ length: BUCKETS }, () => ({
    info: 0,
    warn: 0,
    error: 0,
    total: 0,
  }));
  // Defensive: a story mock (or a future streaming source) can hand back a
  // non-array.
  if (!Array.isArray(levels) || levels.length === 0) return out;
  const n = levels.length;
  for (let i = 0; i < n; i++) {
    // oldest line → leftmost bucket (index 0), newest → rightmost.
    const b = Math.min(BUCKETS - 1, Math.floor((i / n) * BUCKETS));
    const lvl = levels[i];
    out[b][lvl] += 1;
    out[b].total += 1;
  }
  return out;
}

export interface LogRateRibbonProps {
  /** Per-line severity for the buffered lines, oldest → newest. */
  levels: LogLevel[];
  className?: string;
}

export function LogRateRibbon({ levels, className }: LogRateRibbonProps) {
  const { buckets, peak, errors, warns, lastActive } = useMemo(() => {
    const buckets = bucketize(levels);
    let peak = 0;
    let errors = 0;
    let warns = 0;
    let lastActive = -1;
    buckets.forEach((b, i) => {
      if (b.total > peak) peak = b.total;
      errors += b.error;
      warns += b.warn;
      if (b.total > 0) lastActive = i;
    });
    return { buckets, peak, errors, warns, lastActive };
  }, [levels]);

  const total = Array.isArray(levels) ? levels.length : 0;
  const bucketW = (W - 2 * PAD_X) / BUCKETS;
  const barW = Math.max(2, bucketW - 1.2);

  const label =
    total === 0
      ? "Log rate: no log lines buffered."
      : `Log rate across the last ${total} buffered line${total === 1 ? "" : "s"}: ${errors} error${errors === 1 ? "" : "s"}, ${warns} warning${warns === 1 ? "" : "s"}, peak ${peak} line${peak === 1 ? "" : "s"} in a window.`;

  return (
    <VizFrame
      icon={ScrollText}
      label="Log rate"
      className={className}
      readout={
        <>
          <Stat value={total.toLocaleString()} label="lines" />
          <Stat
            value={warns > 0 ? warns.toLocaleString() : "—"}
            label="warn"
            tone={warns > 0 ? "warn" : undefined}
          />
          <Stat
            value={errors > 0 ? errors.toLocaleString() : "—"}
            label="error"
            tone={errors > 0 ? "error" : undefined}
          />
        </>
      }
    >
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
              const barH = Math.max(1.5, (b.total / peak) * BAR_AREA);
              const infoH = (b.info / b.total) * barH;
              const warnH = (b.warn / b.total) * barH;
              const errH = (b.error / b.total) * barH;
              const isLive = i === lastActive;
              // Stack from the baseline up: info (neutral) bottom, warn middle,
              // error top — so a red spike reads above the noise floor.
              let y = BASE_Y;
              const segs: ReactNode[] = [];
              if (infoH > 0.4) {
                y -= infoH;
                segs.push(
                  <rect
                    key="i"
                    x={x}
                    y={y}
                    width={barW}
                    height={infoH}
                    rx={1}
                    className="fill-text-dim"
                  />,
                );
              }
              if (warnH > 0.4) {
                y -= warnH;
                segs.push(
                  <rect
                    key="w"
                    x={x}
                    y={y}
                    width={barW}
                    height={warnH}
                    rx={1}
                    className="fill-warning"
                  />,
                );
              }
              if (errH > 0.4) {
                y -= errH;
                segs.push(
                  <rect
                    key="e"
                    x={x}
                    y={y}
                    width={barW}
                    height={errH}
                    rx={1}
                    className="fill-danger"
                  />,
                );
              }
              return (
                <g key={i} opacity={isLive ? 1 : 0.85}>
                  {segs}
                </g>
              );
            })}

          {/* Axis ticks. */}
          <text
            x={PAD_X}
            y={H - 4}
            className="fill-text-dim font-mono"
            style={{ fontSize: 9 }}
          >
            oldest
          </text>
          <text
            x={W - PAD_X}
            y={H - 4}
            textAnchor="end"
            className="fill-text-dim font-mono"
            style={{ fontSize: 9 }}
          >
            newest
          </text>

          {total === 0 && (
            <text
              x={W / 2}
              y={BASE_Y - BAR_AREA / 2}
              textAnchor="middle"
              className="fill-text-dim font-mono"
              style={{ fontSize: 11, letterSpacing: 0.4 }}
            >
              Quiet — no log lines buffered
            </text>
          )}
        </svg>

        {/* The live now-edge — a reduced-motion-safe pulse at the right, only
            when the newest bucket holds lines. */}
        {lastActive === BUCKETS - 1 && (
          <span
            aria-hidden
            className="ministr-pulse absolute right-[3px] inline-block h-2 w-2 -translate-y-1/2 rounded-full bg-accent"
            style={{ top: `${(TOP_Y / H) * 100}%` }}
          />
        )}
      </div>
    </VizFrame>
  );
}

function Stat({
  value,
  label,
  tone,
}: {
  value: string;
  label: string;
  tone?: "warn" | "error";
}) {
  return (
    <div className="flex items-baseline gap-1">
      <span
        className={cn(
          "font-mono text-sm font-semibold tabular-nums",
          tone === "error"
            ? "text-danger"
            : tone === "warn"
              ? "text-warning"
              : "text-text",
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
