import { cn } from "../../lib/utils";

interface BudgetRingProps {
  /** Primary fill (used / total), 0-1. */
  utilization: number;
  /** Secondary fill behind primary — e.g. prefetch warm set / total, 0-1. */
  warm?: number;
  /** Pressure level — drives the primary arc color. */
  pressure?: "none" | "low" | "medium" | "high" | "critical";
  /** Pixel size of the ring. */
  size?: number;
  /** Stroke thickness in px. */
  stroke?: number;
  /** Content to render at the ring center. */
  children?: React.ReactNode;
  className?: string;
}

/**
 * A radial gauge — kept as a ring shape since it's a graph, but stripped
 * of glow filters and smooth transitions for a brutalist read.
 */
export function BudgetRing({
  utilization,
  warm = 0,
  pressure = "none",
  size = 132,
  stroke = 10,
  children,
  className,
}: BudgetRingProps) {
  const clamp = (v: number) => Math.max(0, Math.min(1, v));
  const u = clamp(utilization);
  const w = clamp(warm);

  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const cx = size / 2;
  const cy = size / 2;

  const primaryColor = {
    none: "var(--color-accent)",
    low: "var(--color-success)",
    medium: "var(--color-accent)",
    high: "var(--color-warning)",
    critical: "var(--color-danger)",
  }[pressure];

  return (
    <div
      role="progressbar"
      aria-valuenow={Math.round(u * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-label="Budget utilization"
      className={cn(
        "relative inline-flex items-center justify-center",
        className,
      )}
      style={{ width: size, height: size }}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className="-rotate-90"
        aria-hidden="true"
      >
        {/* Track — solid border color, no opacity. */}
        <circle
          cx={cx}
          cy={cy}
          r={r}
          fill="none"
          stroke="var(--color-border)"
          strokeWidth={stroke}
        />
        {/* Warm set (prefetch) — inner ghost arc at half stroke. */}
        {w > 0 && (
          <circle
            cx={cx}
            cy={cy}
            r={r - stroke - 2}
            fill="none"
            stroke={primaryColor}
            strokeWidth={Math.max(2, stroke - 4)}
            strokeDasharray={c}
            strokeDashoffset={c - c * w}
          />
        )}
        {/* Primary utilization arc — eases to its new value on poll. */}
        <circle
          cx={cx}
          cy={cy}
          r={r}
          fill="none"
          stroke={primaryColor}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c - c * u}
          style={{
            // §8 — the `flow` easing token (reduced-motion clamps duration globally)
            transition: "stroke-dashoffset 0.45s var(--ease-flow)",
          }}
        />
      </svg>

      <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
        {children}
      </div>
    </div>
  );
}
