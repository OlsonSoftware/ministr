import { useMemo } from "react";
import { Terminal } from "@/components/ui/icons";
import { VizFrame } from "../ui/viz-frame";
import { toneCssVar, type Tone } from "../../lib/status";
import type { ExecRun } from "../RunConsole";
import { cn } from "../../lib/utils";

/**
 * RunTimeline — the recorded shell's temporal shape (the exec lane's
 * bespoke viz beat, ActivityPulse temporal-idiom family).
 *
 * Each recorded run is a duration bar on a shared time axis: where the
 * agent's session spent its wall-clock, which commands ran long, where
 * the failures cluster. Status reads from the bar tone (danger /
 * warning / success; accent = still running, extending to the now-edge
 * with a reduced-motion-safe opacity pulse). Command labels live in a
 * fixed LEFT GUTTER and truncate to its pixel room — structurally they
 * can never collide with bars or each other (the label-fit trap, solved
 * by construction).
 *
 * Renders inside the shared VizFrame; pure (runs + now in, SVG out).
 */

export interface RunTimelineProps {
  runs: readonly ExecRun[];
  /** Frozen clock for stories/tests; live callers pass Date.now(). */
  now: number;
  className?: string;
}

/** ViewBox width — scales to the container like the other vizzes. */
const W = 640;
/** Left gutter reserved for command labels. */
const GUTTER = 168;
/** Mono label font size inside the SVG. */
const LABEL_PX = 10;
/** ~px per mono character at LABEL_PX (the fitLabel convention). */
const PX_PER_CHAR = 6;
/** Lane pitch + bar height. */
const LANE_H = 18;
const BAR_H = 8;
/** Newest-first lane cap so the panel stays an overview, not a list. */
const MAX_LANES = 10;
const TOP_PAD = 6;
const AXIS_H = 18;

function runTone(run: ExecRun): Tone {
  switch (run.status) {
    case "running":
      return "accent";
    case "killed":
    case "timed_out":
      return "warning";
    case "exited":
      return run.exit_code === 0 ? "success" : "danger";
  }
}

/** Truncate a label to the gutter's pixel room (fitLabel convention). */
function fitLabel(label: string, maxPx: number): string {
  const maxChars = Math.floor(maxPx / PX_PER_CHAR);
  return label.length > maxChars
    ? `${label.slice(0, Math.max(1, maxChars - 1))}…`
    : label;
}

function spanLabel(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`;
  return `${(ms / 3_600_000).toFixed(1)}h`;
}

export function RunTimeline({ runs, now, className }: RunTimelineProps) {
  const model = useMemo(() => {
    const lanes = runs.slice(0, MAX_LANES);
    if (lanes.length === 0) return null;
    const t0 = Math.min(...lanes.map((r) => r.started_at_ms));
    // Epsilon keeps the window non-degenerate when every run is instant.
    const t1 = Math.max(
      t0 + 1,
      ...lanes.map((r) => r.finished_at_ms ?? now),
    );
    const plotW = W - GUTTER - 8;
    const x = (t: number) => GUTTER + ((t - t0) / (t1 - t0)) * plotW;
    const bars = lanes.map((run, i) => {
      const start = x(run.started_at_ms);
      const end = x(run.finished_at_ms ?? now);
      return {
        run,
        y: TOP_PAD + i * LANE_H,
        barX: start,
        // A visible sliver even for instant commands.
        barW: Math.max(2, end - start),
        tone: runTone(run),
        live: run.status === "running",
        label: fitLabel(run.command, GUTTER - 12),
      };
    });
    const failed = lanes.filter(
      (r) => r.status === "exited" && r.exit_code !== 0,
    ).length;
    return { bars, t0, t1, failed, shown: lanes.length, total: runs.length };
  }, [runs, now]);

  if (!model) return null;
  const height = TOP_PAD + model.bars.length * LANE_H + AXIS_H;
  const axisY = TOP_PAD + model.bars.length * LANE_H + 4;

  return (
    <VizFrame
      icon={Terminal}
      label="Execution timeline"
      readout={
        <>
          <span>
            {model.shown}
            {model.total > model.shown ? `/${model.total}` : ""} runs
          </span>
          <span>{model.failed} failed</span>
          <span>{spanLabel(model.t1 - model.t0)} window</span>
        </>
      }
      className={className}
    >
      <svg
        viewBox={`0 0 ${W} ${height}`}
        className="w-full"
        role="img"
        aria-label={`Execution timeline: ${model.shown} runs over ${spanLabel(model.t1 - model.t0)}, ${model.failed} failed`}
      >
        {model.bars.map(({ run, y, barX, barW, tone, live, label }) => (
          <g key={run.run_id}>
            {/* Command label — fixed gutter, fitLabel-truncated. */}
            <text
              x={0}
              y={y + BAR_H}
              fontSize={LABEL_PX}
              className="fill-current font-mono text-text-muted"
            >
              {label}
            </text>
            {/* Faint full-width lane guide. */}
            <line
              x1={GUTTER}
              y1={y + BAR_H / 2 + 2}
              x2={W - 8}
              y2={y + BAR_H / 2 + 2}
              stroke="var(--color-border)"
              strokeWidth={1}
            />
            {/* The run's duration bar. */}
            <rect
              x={barX}
              y={y}
              width={barW}
              height={BAR_H}
              rx={2}
              fill={toneCssVar(tone)}
              className={cn(live && "motion-safe:animate-pulse")}
            >
              <title>
                {run.command} — {run.status}
                {run.exit_code !== null ? ` (exit ${run.exit_code})` : ""}
              </title>
            </rect>
          </g>
        ))}

        {/* Time axis: window start → now-edge. */}
        <line
          x1={GUTTER}
          y1={axisY}
          x2={W - 8}
          y2={axisY}
          stroke="var(--color-border)"
          strokeWidth={1}
        />
        <text
          x={GUTTER}
          y={axisY + 11}
          fontSize={LABEL_PX}
          className="fill-current font-mono text-text-dim"
        >
          −{spanLabel(now - model.t0)}
        </text>
        <text
          x={W - 8}
          y={axisY + 11}
          fontSize={LABEL_PX}
          textAnchor="end"
          className="fill-current font-mono text-text-dim"
        >
          now
        </text>
      </svg>
    </VizFrame>
  );
}
