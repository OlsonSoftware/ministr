import { memo, type ReactNode } from "react";
import { type Tone, toneCssVar } from "../../lib/status";
import { cn } from "../../lib/utils";

interface SparklineProps {
  /** Chronological values, oldest → newest. */
  data: number[];
  /** `line` = stepped stroke (tokens over time); `band` = per-sample
   *  colour run (pressure over time). Default `line`. */
  mode?: "line" | "band";
  /** For `band`: one tone per sample (same length as `data`). */
  bandTones?: Tone[];
  width?: number;
  height?: number;
  /** `line` stroke tone. Default `accent`. */
  tone?: Tone;
  /** Required text alternative — the graphic is decorative, this carries
   *  the meaning for assistive tech. */
  ariaLabel: string;
  className?: string;
}

/**
 * A datasheet sparkline. Deliberately *stepped*, never smoothed —
 * brutalism rejects bezier curves. Drawn fresh on every poll (no path
 * transition), so it is reduced-motion-safe by construction. SVG-only, no
 * chart lib, `crispEdges` so 2px strokes stay sharp.
 */
function SparklineImpl({
  data,
  mode = "line",
  bandTones,
  width = 120,
  height = 36,
  tone = "accent",
  ariaLabel,
  className,
}: SparklineProps) {
  const wrap = (body: ReactNode) => (
    <svg
      role="img"
      aria-label={ariaLabel}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      shapeRendering="crispEdges"
      className={cn("block w-full", className)}
      style={{ height }}
    >
      {body}
    </svg>
  );

  // No data → a single flat baseline rule (never an error / empty box).
  if (data.length === 0) {
    return wrap(
      <line
        x1={0}
        y1={height - 1}
        x2={width}
        y2={height - 1}
        stroke="var(--color-border-soft)"
        strokeWidth={1}
      />,
    );
  }

  if (mode === "band") {
    const n = data.length;
    const seg = width / n;
    return wrap(
      <>
        {data.map((_, i) => (
          <rect
            // index key: the series is positional and fully redrawn each poll
            key={i}
            x={i * seg}
            y={0}
            width={seg + 0.5 /* hairline overlap kills sub-pixel gaps */}
            height={height}
            fill={toneCssVar(bandTones?.[i] ?? "muted")}
          />
        ))}
      </>,
    );
  }

  // line mode — step-after polyline.
  const min = Math.min(...data);
  const max = Math.max(...data);
  const span = max - min || 1;
  const n = data.length;
  const x = (i: number) => (n === 1 ? width / 2 : (i / (n - 1)) * width);
  // 1px inset top/bottom so a full-height value isn't clipped by the stroke.
  const y = (v: number) => height - 1 - ((v - min) / span) * (height - 2);

  const stroke = toneCssVar(tone);

  if (n === 1) {
    return wrap(
      <rect
        x={width / 2 - 2}
        y={y(data[0]) - 2}
        width={4}
        height={4}
        fill={stroke}
      />,
    );
  }

  // Build the step-after point list: hold each y until the next x.
  const pts: string[] = [];
  for (let i = 0; i < n; i++) {
    if (i > 0) pts.push(`${x(i)},${y(data[i - 1])}`);
    pts.push(`${x(i)},${y(data[i])}`);
  }
  const lastX = x(n - 1);
  const lastY = y(data[n - 1]);

  return wrap(
    <>
      <line
        x1={0}
        y1={height - 1}
        x2={width}
        y2={height - 1}
        stroke="var(--color-border-soft)"
        strokeWidth={1}
      />
      <polyline
        points={pts.join(" ")}
        fill="none"
        stroke={stroke}
        strokeWidth={2}
        strokeLinejoin="miter"
        strokeLinecap="square"
        vectorEffect="non-scaling-stroke"
      />
      <rect x={lastX - 2} y={lastY - 2} width={4} height={4} fill={stroke} />
    </>,
  );
}

/** Memoised — props are a value array + scalars; the store hands a stable
 *  array ref when the underlying samples are unchanged. */
export const Sparkline = memo(SparklineImpl);
