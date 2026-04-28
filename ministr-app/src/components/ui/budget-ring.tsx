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
 * A radial budget gauge — the signature ministr visualization.
 *
 * - outer arc = utilization (token budget used)
 * - inner arc = warm set (prefetch cache coverage)
 * - center = caller-provided content (typically the big % number)
 */
export function BudgetRing({
  utilization,
  warm = 0,
  pressure = "none",
  size = 132,
  stroke = 8,
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
        {/* Track */}
        <circle
          cx={cx}
          cy={cy}
          r={r}
          fill="none"
          stroke="var(--color-border)"
          strokeWidth={stroke}
          strokeOpacity={0.6}
        />
        {/* Warm set (prefetch) — inner ghost arc */}
        {w > 0 && (
          <circle
            cx={cx}
            cy={cy}
            r={r - stroke - 2}
            fill="none"
            stroke={primaryColor}
            strokeOpacity={0.25}
            strokeWidth={stroke - 2}
            strokeDasharray={c}
            strokeDashoffset={c - c * w}
            strokeLinecap="round"
          />
        )}
        {/* Primary utilization arc */}
        <circle
          cx={cx}
          cy={cy}
          r={r}
          fill="none"
          stroke={primaryColor}
          strokeWidth={stroke}
          strokeDasharray={c}
          strokeDashoffset={c - c * u}
          strokeLinecap="round"
          style={{
            filter: `drop-shadow(0 0 6px color-mix(in srgb, ${primaryColor} 50%, transparent))`,
            transition: "stroke-dashoffset 600ms cubic-bezier(0.2, 0, 0, 1)",
          }}
        />
      </svg>

      <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
        {children}
      </div>
    </div>
  );
}
