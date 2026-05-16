import { toneBgClass, toneTextClass } from "../../lib/status";
import {
  type BudgetThresholds,
  clampPct,
  utilizationTone,
} from "../../lib/sessions";
import { cn } from "../../lib/utils";

interface BudgetBarProps {
  /** 0..1 utilization. */
  utilization: number;
  /** `hero` = 12px bordered (the unmissable hero bar); `card` = 8px
   *  hairline (the list-card bar). Default `card`. */
  size?: "hero" | "card";
  /** Right-aligned integer % label. Defaults on for `hero`, off for
   *  `card`. */
  showValue?: boolean;
  /** Daemon-reported thresholds when available; falls back to defaults. */
  thresholds?: BudgetThresholds;
  className?: string;
}

/**
 * The budget-utilization bar. Colour is derived from utilization (never a
 * pressure string), so it cannot regress to the old grey/colourless state.
 * Brutalist: sharp corners, solid-offset frame, no blur. The fill width
 * carries `.motion-data` (the one sanctioned data-motion) so it eases on
 * poll and snaps under `prefers-reduced-motion`.
 */
export function BudgetBar({
  utilization,
  size = "card",
  showValue,
  thresholds,
  className,
}: BudgetBarProps) {
  const tone = utilizationTone(utilization, thresholds);
  const pct = clampPct(utilization * 100);
  const showPct = showValue ?? size === "hero";

  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div
        className={cn(
          "relative flex-1 overflow-hidden bg-surface-overlay",
          size === "hero"
            ? "h-3 border-2 border-border"
            : "h-2 border border-border-soft",
        )}
      >
        <div
          className={cn("h-full motion-data", toneBgClass(tone))}
          style={{ width: `${pct}%` }}
        />
      </div>
      {showPct && (
        <span
          className={cn(
            "shrink-0 font-mono text-xs font-bold tabular-nums",
            toneTextClass(tone),
          )}
        >
          {pct}%
        </span>
      )}
    </div>
  );
}
